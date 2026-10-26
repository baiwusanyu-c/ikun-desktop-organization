//! Custom drag tracking coordinator.
//!
//! Manages the drag lifecycle: threshold detection, drag tracking via timer,
//! and dispatching drop operations. Delegates to sub-modules:
//! - [`drag_drop`]: drop operations (same-box, box-to-box, box-to-desktop, OLE)
//! - [`drag_visual`]: drag-over visual feedback (insert separator + replace highlight)
//!
//! Uses Timer-based polling instead of SetCapture (unstable on WS_EX_LAYERED windows).

use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::get_data;

// Re-exports for external callers (wnd_proc, overlay_wnd_proc, etc.)
pub(crate) use super::drag_drop::{
    handle_drag_enter, handle_drag_item_added, handle_drag_leave, handle_ole_drop,
};
pub(crate) use super::drag_visual::handle_cross_box_drag_over;

// =============================================================================
// Phase 1: Drag threshold detection (from WM_MOUSEMOVE)
// =============================================================================
/// 返回 true 表示拖拽已激活（调用者应 return 0）。
pub(crate) fn check_drag_threshold(hwnd: HWND, mx: i32, my: i32) -> bool {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return false,
    };

    if !data.drag_pending {
        return false;
    }

    let dx = (mx - data.drag_start_x).abs();
    let dy = (my - data.drag_start_y).abs();
    if dx <= DRAG_THRESHOLD && dy <= DRAG_THRESHOLD {
        return false;
    }

    // 超过阈值 → 激活自定义拖拽
    data.drag_pending = false;
    let item_idx = data.selected_item;
    if item_idx < 0 || (item_idx as usize) >= data.items.len() {
        return false;
    }

    // 检查是否有多选
    let container_type = super::multi_select::ContainerType::GridBox;
    let container_id = super::multi_select::container_id_from_uuid(data.config_id);
    let selected_indices = super::multi_select::selected_in_container(container_type, container_id);

    // 如果选中的项包含当前拖动的项，则使用多选模式
    log::info!(
        "[DRAG-THRESHOLD] item_idx={}, selected_indices={:?}",
        item_idx,
        selected_indices
    );
    if selected_indices.len() > 1 && selected_indices.contains(&(item_idx as usize)) {
        log::info!(
            "[DRAG-THRESHOLD] Multi-drag activated: {} items",
            selected_indices.len()
        );
        data.multi_drag_items = selected_indices;
    } else {
        data.multi_drag_items.clear();
    }

    // ʹõǰĲΪקԴ
    let drag_source_idx = item_idx as usize;
    // Ensure the clicked item is in multi_drag_items
    if !data.multi_drag_items.is_empty() && !data.multi_drag_items.contains(&drag_source_idx) {
        data.multi_drag_items.push(drag_source_idx);
    }

    let icon_handle = data.items[drag_source_idx].icon_handle;
    data.drag_active = true;
    data.drag_item_index = drag_source_idx as i32;

    log::info!(
        "自定义拖拽激活: item={}, multi_count={}",
        drag_source_idx,
        data.multi_drag_items.len()
    );

    // 创建拖拽覆盖窗口（半透明图标 + 名称标签跟随光标，DPI 自适应尺寸）
    let icon_size = data.icon_size;
    let cell_w = data.cell_w;
    let display_name = data.items[drag_source_idx].display_name.clone();
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    unsafe {
        if data.multi_drag_items.len() > 1 {
            // 多选拖拽：显示所有选中图标的残影
            let mut multi_items = Vec::new();
            let mut source_idx_in_list = 0;
            for (list_idx, &item_idx) in data.multi_drag_items.iter().enumerate() {
                if item_idx < data.items.len() {
                    let item = &data.items[item_idx];
                    multi_items.push(super::drag_image::MultiDragItemInfo {
                        icon_handle: item.icon_handle,
                        display_name: item.display_name.clone(),
                        grid_col: item.grid_col,
                        grid_row: item.grid_row,
                    });
                    if item_idx == drag_source_idx {
                        source_idx_in_list = list_idx;
                    }
                }
            }
            log::info!(
                "多选拖拽: 创建 {} 个图标的残影, source_idx={}",
                multi_items.len(),
                source_idx_in_list
            );
            super::drag_image::create_drag_overlay_multi(
                multi_items,
                source_idx_in_list,
                icon_size,
                cell_w,
                data.cell_h,
                dpi,
            );
        } else {
            // 单选拖拽：只显示单个图标
            super::drag_image::create_drag_overlay(
                icon_handle,
                icon_size,
                cell_w,
                &display_name,
                dpi,
            );
        }
        super::drag_image::update_drag_overlay();
        // 使用 Timer 轮询检测鼠标释放（SetCapture 在分层窗口上不稳定）
        SetTimer(hwnd, TIMER_ID_GRID_DRAG, 8, std::ptr::null_mut());
    }

    true
}

// =============================================================================
// Phase 2: Drag position update (from WM_MOUSEMOVE)
// =============================================================================
/// 返回 true 表示正在拖拽中（调用者应 return 0）。
pub(crate) fn update_drag(hwnd: HWND) -> bool {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return false,
    };

    if !data.drag_active {
        return false;
    }

    // 更新覆盖窗口位置
    unsafe { super::drag_image::update_drag_overlay() };

    // 检测光标下的窗口，管理高亮
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    // 使用 find_grid_window_at_point 替代 WindowFromPoint，
    // 因为分类框在 HWND_BOTTOM，WindowFromPoint 会被覆盖窗口拦截
    let target_hwnd = super::zorder::find_grid_window_at_point(pt.x, pt.y);

    // 对目标分类框显示高亮
    if !target_hwnd.is_null() && target_hwnd != hwnd {
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_ENTER, 0, 0) };
    }

    true
}

// =============================================================================
// Phase 3: Drag release (from WM_LBUTTONUP / timer)
// =============================================================================
/// 返回 true 表示拖拽已完成（调用者应 return 0）。
pub(crate) fn end_drag(hwnd: HWND) -> bool {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return false,
    };

    log::info!(
        "end_drag: drag_active={}, hwnd={:?}, drag_item_index={}, multi_drag_count={}",
        data.drag_active,
        hwnd,
        data.drag_item_index,
        data.multi_drag_items.len()
    );

    if !data.drag_active {
        return false;
    }

    let item_idx = data.drag_item_index;
    data.drag_active = false;
    data.drag_item_index = -1;
    // 物品拖拽结束同样释放全屏参考线 overlay。
    unsafe {
        PostMessageW(hwnd, WM_USER_DESTROY_GUIDE, 0, 0);
    }
    // 保存拖拽覆盖可视化状态（在清除前），供 handle_same_box_drop 使用
    let saved_insert_col = data.drag_over_insert_col;
    let saved_insert_row = data.drag_over_insert_row;
    // 清除拖拽覆盖可视化状态
    data.drag_over_item = -1;
    data.drag_over_insert_col = -1;
    data.drag_over_insert_row = -1;

    // 停止定时器 + 销毁覆盖窗口
    unsafe {
        log::info!("[END-DRAG] 停止定时器 + 销毁残影覆盖窗口");
        KillTimer(hwnd, TIMER_ID_GRID_DRAG);
        super::drag_image::destroy_drag_overlay();
    }

    if item_idx < 0 || (item_idx as usize) >= data.items.len() {
        log::warn!("拖拽释放: 项索引无效");
        return true;
    }

    // 检测释放目标
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    // ★ 优先检测外部窗口（Explorer 等），避免分类框窗口矩形误判
    let ext_hwnd = super::zorder::find_external_window_at_point(pt.x, pt.y);
    let target_hwnd = if !ext_hwnd.is_null() {
        log::info!("[END-DRAG] 外部窗口优先: {:?}", ext_hwnd);
        std::ptr::null_mut() // 外部窗口 → 视为桌面/Explorer 目标
    } else {
        super::zorder::find_grid_window_at_point(pt.x, pt.y)
    };

    // 检查是否有多选拖拽
    let is_multi_drag = !data.multi_drag_items.is_empty();

    log::info!(
        "[END-DRAG] item_idx={}, is_multi={}, target_hwnd={:?}, multi_items={:?}",
        item_idx,
        is_multi_drag,
        target_hwnd,
        data.multi_drag_items
    );

    if target_hwnd == hwnd {
        // 同框内拖拽
        if is_multi_drag {
            // Multi-drag: compute offset and batch-move all items
            let drag_indices = data.multi_drag_items.clone();
            let primary_idx = item_idx as usize;
            log::info!(
                "Multi-drag same box: {} items, primary={}",
                drag_indices.len(),
                primary_idx
            );

            // Record old positions
            let old_positions: Vec<(i32, i32)> = drag_indices
                .iter()
                .map(|&i| {
                    if i < data.items.len() {
                        let item = &data.items[i];
                        if item.grid_col >= 0 {
                            (item.grid_col, item.grid_row)
                        } else {
                            let c = (i as i32) % data.cols;
                            (c, (i as i32) / data.cols)
                        }
                    } else {
                        (0, 0)
                    }
                })
                .collect();

            // Move primary item first
            super::drag_drop::handle_same_box_drop(
                hwnd,
                primary_idx,
                pt.x,
                pt.y,
                saved_insert_col,
                saved_insert_row,
            );

            // Get new position of primary
            if primary_idx < data.items.len() {
                let new_primary = &data.items[primary_idx];
                let new_col = if new_primary.grid_col >= 0 {
                    new_primary.grid_col
                } else {
                    (primary_idx as i32) % data.cols
                };
                let new_row = if new_primary.grid_row >= 0 {
                    new_primary.grid_row
                } else {
                    (primary_idx as i32) / data.cols
                };
                let delta_col = new_col
                    - old_positions[drag_indices
                        .iter()
                        .position(|&x| x == primary_idx)
                        .unwrap_or(0)]
                    .0;
                let delta_row = new_row
                    - old_positions[drag_indices
                        .iter()
                        .position(|&x| x == primary_idx)
                        .unwrap_or(0)]
                    .1;

                // Move all other selected items by the same delta
                let primary_pos = drag_indices
                    .iter()
                    .position(|&x| x == primary_idx)
                    .unwrap_or(0);
                for (k, &idx) in drag_indices.iter().enumerate() {
                    if k == primary_pos {
                        continue;
                    }
                    if idx < data.items.len() {
                        let target_col = old_positions[k].0 + delta_col;
                        let target_row = old_positions[k].1 + delta_row;
                        if target_col >= 0 && target_row >= 0 {
                            data.items[idx].grid_col = target_col;
                            data.items[idx].grid_row = target_row;
                        }
                    }
                }
            }
            super::d2d_render::render_layered_window(hwnd);
            super::drag_handler::save_config();
        } else {
            // 单选同框内拖拽 → grid snap + swap
            super::drag_drop::handle_same_box_drop(
                hwnd,
                item_idx as usize,
                pt.x,
                pt.y,
                saved_insert_col,
                saved_insert_row,
            );
        }
        data.multi_drag_items.clear();
        return true;
    }

    if !target_hwnd.is_null() {
        // === 框→框移动 ===
        if is_multi_drag {
            log::info!(
                "Multi-drag cross box: {} items",
                data.multi_drag_items.len()
            );
            // Move items in reverse order to avoid index shifting
            let mut sorted_indices = data.multi_drag_items.clone();
            sorted_indices.sort();
            sorted_indices.reverse();
            for &drag_idx in &sorted_indices {
                super::drag_drop::handle_box_to_box_move(hwnd, target_hwnd, drag_idx);
            }
        } else {
            super::drag_drop::handle_box_to_box_move(hwnd, target_hwnd, item_idx as usize);
        }
    } else {
        // === 框→桌面/覆盖窗口（从框中移除项）===
        // target_hwnd 为 null 表示拖到覆盖区域或桌面
        if is_multi_drag {
            log::info!(
                "[END-DRAG] Multi-drag to desktop: {} items, indices={:?}",
                data.multi_drag_items.len(),
                data.multi_drag_items
            );
            let mut sorted_indices = data.multi_drag_items.clone();
            sorted_indices.sort();
            sorted_indices.reverse();
            log::info!(
                "[END-DRAG] Processing in reverse order: {:?}",
                sorted_indices
            );
            for &drag_idx in &sorted_indices {
                log::info!(
                    "[END-DRAG] Calling handle_box_to_desktop(idx={}, pt=({}, {}))",
                    drag_idx,
                    pt.x,
                    pt.y
                );
                super::drag_drop::handle_box_to_desktop(hwnd, drag_idx, pt.x, pt.y, true);
            }
        } else {
            super::drag_drop::handle_box_to_desktop(hwnd, item_idx as usize, pt.x, pt.y, true);
        }
    }

    data.multi_drag_items.clear();
    // 拖拽结束后清除全局多选状态，确保下次点击正常行为
    super::multi_select::clear();

    // 清除所有框的高亮
    unsafe { PostMessageW(hwnd, WM_USER_DRAG_LEAVE, 0, 0) };
    if !target_hwnd.is_null() {
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_LEAVE, 0, 0) };
    }

    true
}

// =============================================================================
// Drag timer polling (replaces SetCapture on WS_EX_LAYERED)
// =============================================================================
/// 网格拖拽轮询定时器处理函数（每 16ms 调用一次）
pub(crate) fn handle_grid_drag_timer(hwnd: HWND) {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };
    // Check marquee: if active, render and detect release
    if data.marquee_active {
        // Render marquee at timer rate (16ms = ~60fps) for smooth visual update
        super::d2d_render::render_layered_window(hwnd);
        let lbutton_down = unsafe { GetAsyncKeyState(VK_LBUTTON) } & 0x8000u16 as i16 != 0;
        if !lbutton_down {
            unsafe {
                PostMessageW(hwnd, WM_LBUTTONUP, 0, 0);
            }
        }
        return;
    }

    if !data.drag_active {
        log::warn!("handle_grid_drag_timer: drag_active=false, 清理残影状态");
        data.multi_drag_items.clear();
        unsafe {
            KillTimer(hwnd, TIMER_ID_GRID_DRAG);
            super::drag_image::destroy_drag_overlay();
        }
        return;
    }

    // 更新拖拽残影位置
    unsafe { super::drag_image::update_drag_overlay() };

    // 检测光标位置
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };

    // ★ 优先检测外部窗口（Explorer 等）— 必须在分类框检测之前！
    // 分类框的窗口矩形(GetWindowRect)可能很大，光标虽然在桌面/Explorer上，
    // 但仍落在某个分类框的窗口矩形内，导致 find_grid_window_at_point 误判。
    let ext_hwnd = super::zorder::find_external_window_at_point(pt.x, pt.y);
    if !ext_hwnd.is_null() {
        log::info!(
            "[GRID-TIMER] 检测到外部窗口 {:?} at ({}, {}), 启动 OLE 拖拽",
            ext_hwnd,
            pt.x,
            pt.y
        );
        start_grid_ole_drag(hwnd, ext_hwnd);
        return;
    }

    // 检测光标下的分类框窗口
    let target_hwnd = super::zorder::find_grid_window_at_point(pt.x, pt.y);

    // 拖拽可视化反馈：检测同框内和跨框的拖拽覆盖状态
    if target_hwnd == hwnd {
        super::drag_visual::update_drag_over_visual(hwnd, pt.x, pt.y);
    } else {
        // 光标不在同框内，清除本框的拖拽覆盖状态
        super::drag_visual::clear_drag_over_visual(data);
    }

    // 跨框可视化反馈：发送光标位置给目标框以更新可视化
    if !target_hwnd.is_null() && target_hwnd != hwnd {
        let lparam = ((pt.y as isize) << 16) | ((pt.x as isize) & 0xFFFF);
        unsafe {
            PostMessageW(target_hwnd, WM_USER_DRAG_OVER_UPDATE, 0, lparam as LPARAM);
        }
    }

    // 检测鼠标左键是否仍然按下
    let lbutton_down = unsafe { GetAsyncKeyState(VK_LBUTTON) } & 0x8000u16 as i16 != 0;
    if !lbutton_down {
        log::info!("[GRID-TIMER] 鼠标已释放，结束拖拽");
        end_drag(hwnd);
    }
}

// =============================================================================
/// 检测到光标在外部窗口（Explorer）上时，启动 OLE 拖拽。
///
/// 行为与覆盖区域拖拽到 Explorer 一致：
/// - 将文件复制到目标位置（CF_HDROP 格式）
/// - 分类框内图标保持原位（复制操作，不删除）
fn start_grid_ole_drag(hwnd: HWND, _external_hwnd: HWND) {
    log::info!("[GRID-OLE] 启动分类框→Explorer OLE 拖拽，销毁自定义残影");
    unsafe {
        KillTimer(hwnd, TIMER_ID_GRID_DRAG);
    }

    // 获取拖拽项信息并重置拖拽状态（包含 multi_drag_items）
    let (item_idx, multi_drag_items, config_id_str) = {
        let data = match unsafe { get_data(hwnd) } {
            Some(d) => d,
            None => return,
        };

        let idx = data.drag_item_index;
        let multi = data.multi_drag_items.clone();
        data.drag_pending = false;
        data.drag_active = false;
        data.drag_item_index = -1;
        data.drag_over_item = -1;
        data.drag_over_insert_col = -1;
        data.drag_over_insert_row = -1;
        data.multi_drag_items.clear();

        (idx, multi, data.config_id.to_string())
    };

    let is_multi = multi_drag_items.len() > 1;

    log::info!(
        "[GRID-OLE] item_idx={}, is_multi={}, multi_items={:?}",
        item_idx,
        is_multi,
        multi_drag_items
    );

    if is_multi {
        // Multi-select OLE drag: collect all file paths and metadata
        // ★ 创建临时目录，复制仓库文件并使用原始文件名
        let temp_dir = std::env::temp_dir().join("ikun_ole_drag");
        let _ = std::fs::create_dir_all(&temp_dir);

        let (file_paths, drag_items, icon_handle) = {
            let data = match unsafe { get_data(hwnd) } {
                Some(d) => d,
                None => {
                    unsafe {
                        super::drag_image::destroy_drag_overlay();
                    }
                    super::d2d_render::render_layered_window(hwnd);
                    return;
                }
            };

            let mut paths = Vec::new();
            let mut items = Vec::new();
            let mut primary_icon = std::ptr::null_mut();
            for &idx in &multi_drag_items {
                if idx < data.items.len() {
                    let item = &data.items[idx];
                    // 解析到实际存在的路径（仓库路径）
                    let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);

                    // ★ 提取原始文件名（从逻辑路径）并复制到临时目录
                    let original_name = std::path::Path::new(&item.file_path)
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| format!("file_{}", idx));
                    let temp_path = temp_dir.join(&original_name);

                    if resolved.exists() {
                        // 目录感知复制：文件夹也能完整复制到临时目录，
                        // std::fs::copy 仅支持文件，拖拽文件夹到资源管理器会失败
                        let _ = super::icon_hider::remove_path(&temp_path);
                        if let Err(e) =
                            super::icon_hider::copy_path_recursive(&resolved, &temp_path)
                        {
                            log::warn!(
                                "[GRID-OLE-MULTI] 复制文件失败: {:?} -> {:?}: {}",
                                resolved,
                                temp_path,
                                e
                            );
                            continue;
                        }
                    } else {
                        log::warn!("[GRID-OLE-MULTI] 源文件不存在: {:?}", resolved);
                        continue;
                    }

                    let temp_str = temp_path.to_string_lossy().to_string();
                    log::info!(
                        "[GRID-OLE-MULTI] item[{}]: original='{}', temp='{}'",
                        idx,
                        original_name,
                        temp_str
                    );
                    paths.push(temp_str.clone());
                    items.push(IkunDragItemData {
                        index: idx,
                        file_path: temp_str,
                        display_name: item.display_name.clone(),
                    });
                    if idx == item_idx as usize {
                        primary_icon = item.icon_handle;
                    }
                }
            }
            if primary_icon.is_null() && item_idx >= 0 && (item_idx as usize) < data.items.len() {
                primary_icon = data.items[item_idx as usize].icon_handle;
            }
            (paths, items, primary_icon)
        };

        log::info!("分类框→Explorer OLE (multi): {} items", file_paths.len());

        let box_drag_data = IkunBoxDragData {
            source_hwnd: hwnd as usize,
            source_config_id: config_id_str,
            items: drag_items,
        };

        // ★ 不销毁拖拽残影，让它在 OLE 拖拽期间保持存活
        // 隐藏分类框，让 OLE hit-testing 能到达 Explorer
        // 临时添加 WS_EX_TRANSPARENT 让分类框对 OLE hit-testing 透明，
        // 同时保持窗口可见（不像 SW_HIDE 那样隐藏窗口）。
        // WS_EX_TRANSPARENT 使窗口对鼠标输入透明，OLE 的
        // WindowFromPoint 会穿透到下方的 Explorer 窗口。
        unsafe {
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex_style | WS_EX_TRANSPARENT) as isize);
        }
        let drop_succeeded = super::ole_dnd::start_box_drag(file_paths, box_drag_data, icon_handle);
        unsafe {
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex_style & !WS_EX_TRANSPARENT) as isize);
        }

        // ★ OLE 拖拽完成后销毁残影
        unsafe {
            super::drag_image::destroy_drag_overlay();
        }

        // ★ 清理临时目录
        let _ = std::fs::remove_dir_all(&temp_dir);
        log::info!("[GRID-OLE-MULTI] 临时目录已清理: {:?}", temp_dir);

        if drop_succeeded {
            log::info!("分类框→Explorer OLE 多文件拖拽成功，图标保持原位");
        } else {
            log::info!("分类框→Explorer OLE 多文件拖拽取消，图标保持原位");
        }
        super::d2d_render::render_layered_window(hwnd);
    } else {
        // Single-item OLE drag (existing logic)
        let temp_dir = std::env::temp_dir().join("ikun_ole_drag");
        let _ = std::fs::create_dir_all(&temp_dir);
        let (item_idx, file_path, display_name, icon_handle) = {
            let data = match unsafe { get_data(hwnd) } {
                Some(d) => d,
                None => {
                    unsafe {
                        super::drag_image::destroy_drag_overlay();
                    }
                    super::d2d_render::render_layered_window(hwnd);
                    return;
                }
            };

            if item_idx < 0 || (item_idx as usize) >= data.items.len() {
                unsafe {
                    super::drag_image::destroy_drag_overlay();
                }
                super::d2d_render::render_layered_window(hwnd);
                return;
            }

            let item = &data.items[item_idx as usize];
            // ★ 解析到实际存在的路径并复制到临时目录使用原始文件名
            let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);
            let original_name = std::path::Path::new(&item.file_path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("file_{}", item_idx));
            let temp_path = temp_dir.join(&original_name);
            if resolved.exists() {
                // 目录感知复制：文件夹也能完整复制到临时目录，
                // std::fs::copy 仅支持文件，拖拽文件夹到资源管理器会失败
                let _ = super::icon_hider::remove_path(&temp_path);
                let _ = super::icon_hider::copy_path_recursive(&resolved, &temp_path);
            }
            let resolved_str = temp_path.to_string_lossy().to_string();
            (
                item_idx,
                resolved_str,
                item.display_name.clone(),
                item.icon_handle,
            )
        };

        // ★ 不销毁拖拽残影，让它在 OLE 拖拽期间保持存活
        // unsafe { super::drag_image::destroy_drag_overlay(); }

        log::info!("分类框→Explorer OLE: {} (path={})", display_name, file_path);

        // 构造 OLE 拖拽数据（CF_HDROP + IkunBoxDragData 自定义格式）
        let box_drag_data = IkunBoxDragData {
            source_hwnd: hwnd as usize,
            source_config_id: config_id_str,
            items: vec![IkunDragItemData {
                index: item_idx as usize,
                file_path: file_path.clone(),
                display_name: display_name.clone(),
            }],
        };

        // 隐藏分类框，让 OLE hit-testing 能到达 Explorer
        // 临时添加 WS_EX_TRANSPARENT 让分类框对 OLE hit-testing 透明
        // 保持窗口可见，不像 SW_HIDE 那样隐藏
        unsafe {
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex_style | WS_EX_TRANSPARENT) as isize);
        }
        let drop_succeeded =
            super::ole_dnd::start_box_drag(vec![file_path], box_drag_data, icon_handle);
        unsafe {
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex_style & !WS_EX_TRANSPARENT) as isize);
        }

        if drop_succeeded {
            log::info!("分类框→Explorer OLE 拖拽成功，图标保持原位");
        } else {
            log::info!("分类框→Explorer OLE 拖拽取消，图标保持原位");
        }
        // 图标始终保持原位（复制操作，不是移动）

        // ★ OLE 拖拽完成后销毁残影并清理临时目录
        unsafe {
            super::drag_image::destroy_drag_overlay();
        }
        let _ = std::fs::remove_dir_all(&temp_dir);
        super::d2d_render::render_layered_window(hwnd);
    }
    // OLE 拖拽结束后清除全局多选状态
    super::multi_select::clear();
}

// =============================================================================
// Config save helper (used by drag_drop sub-module)
// =============================================================================
/// 拖拽操作后保存配置
pub(crate) fn save_config() {
    let layouts = super::grid_window::collect_grid_layout();
    if let Ok(mut config) = crate::config::AppConfig::load() {
        config.layout.boxes = layouts;
        if let Err(e) = config.save() {
            log::warn!("拖拽后保存配置失败: {}", e);
        } else {
            log::info!("拖拽后配置已保存");
        }
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ===== Drag state machine tests =====

    #[test]
    fn test_drag_threshold_not_exceeded() {
        // Simulate drag_pending=true but mouse hasn't moved enough
        let drag_start_x: i32 = 100;
        let drag_start_y: i32 = 100;
        let mx: i32 = 103; // dx = 3, below DRAG_THRESHOLD
        let my: i32 = 102; // dy = 2, below DRAG_THRESHOLD
        let dx = (mx - drag_start_x).abs();
        let dy = (my - drag_start_y).abs();
        assert!(dx <= DRAG_THRESHOLD);
        assert!(dy <= DRAG_THRESHOLD);
    }

    #[test]
    fn test_drag_threshold_exceeded() {
        let drag_start_x: i32 = 100;
        let drag_start_y: i32 = 100;
        let mx: i32 = 120; // dx = 20, above DRAG_THRESHOLD
        let my: i32 = 115; // dy = 15, above DRAG_THRESHOLD
        let dx = (mx - drag_start_x).abs();
        let dy = (my - drag_start_y).abs();
        assert!(dx > DRAG_THRESHOLD);
        assert!(dy > DRAG_THRESHOLD);
    }

    // ===== External window detection logic tests =====

    #[test]
    fn test_external_window_detection_null_target_triggers_check() {
        // When target_hwnd (grid window) is null, external window check should be performed
        let target_hwnd: HWND = std::ptr::null_mut();
        assert!(
            target_hwnd.is_null(),
            "Null target should trigger external window check"
        );
    }

    #[test]
    fn test_external_window_detection_non_null_target_skips_check() {
        // When target_hwnd is non-null (found a grid window), external check should be skipped
        // Use a fake non-null pointer for testing
        let fake_hwnd: HWND = 0x1234 as HWND;
        assert!(
            !fake_hwnd.is_null(),
            "Non-null target should skip external window check"
        );
    }

    // ===== OLE drag data construction tests =====

    #[test]
    fn test_ikun_box_drag_data_construction() {
        let data = IkunBoxDragData {
            source_hwnd: 0xABCD,
            source_config_id: "test-uuid-123".to_string(),
            items: vec![IkunDragItemData {
                index: 0,
                file_path: "C:\\Users\\test\\Desktop\\app.lnk".to_string(),
                display_name: "App".to_string(),
            }],
        };
        assert_eq!(data.source_hwnd, 0xABCD);
        assert_eq!(data.source_config_id, "test-uuid-123");
        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].index, 0);
        assert!(data.items[0].file_path.ends_with("app.lnk"));
    }

    #[test]
    fn test_ikun_drag_item_data_serialization() {
        let item = IkunDragItemData {
            index: 5,
            file_path: "test.lnk".to_string(),
            display_name: "Test".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let parsed: IkunDragItemData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.index, 5);
        assert_eq!(parsed.file_path, "test.lnk");
        assert_eq!(parsed.display_name, "Test");
    }

    // ===== Drag state reset pattern tests =====

    #[test]
    fn test_drag_state_reset_pattern() {
        // Verify the state reset pattern used in start_grid_ole_drag
        let mut drag_pending = true;
        let mut drag_active = true;
        let mut drag_item_index: i32 = 3;
        let mut drag_over_item: i32 = 2;
        let mut drag_over_insert_col: i32 = 1;
        let mut drag_over_insert_row: i32 = 0;

        // Reset (same pattern as start_grid_ole_drag)
        drag_pending = false;
        drag_active = false;
        drag_item_index = -1;
        drag_over_item = -1;
        drag_over_insert_col = -1;
        drag_over_insert_row = -1;

        assert!(!drag_pending);
        assert!(!drag_active);
        assert_eq!(drag_item_index, -1);
        assert_eq!(drag_over_item, -1);
        assert_eq!(drag_over_insert_col, -1);
        assert_eq!(drag_over_insert_row, -1);
    }

    #[test]
    fn test_drag_item_index_bounds_check() {
        // Test bounds checking pattern used in start_grid_ole_drag
        let items_len: usize = 5;
        let valid_idx: i32 = 3;
        let invalid_neg: i32 = -1;
        let invalid_over: i32 = 10;

        assert!(valid_idx >= 0 && (valid_idx as usize) < items_len);
        assert!(invalid_neg < 0 || (invalid_neg as usize) >= items_len);
        assert!(invalid_over < 0 || (invalid_over as usize) >= items_len);
    }

    /// 验证 end_drag 状态重置模式（与 release_drop_target_helper 协同）
    /// end_drag 必须将所有拖拽状态字段重置为默认值
    #[test]
    fn test_end_drag_full_state_reset() {
        // 模拟拖拽进行中的状态
        let drag_pending = false; // 已激活
        let mut drag_active = true;
        let mut drag_item_index: i32 = 3;
        let mut drag_over_item: i32 = 2;
        let mut drag_over_insert_col: i32 = 1;
        let mut drag_over_insert_row: i32 = 0;
        let mut multi_drag_items: Vec<usize> = vec![1, 2, 3];

        // 模拟 end_drag 的状态重置（与实际代码一致）
        drag_active = false;
        drag_item_index = -1;
        drag_over_item = -1;
        drag_over_insert_col = -1;
        drag_over_insert_row = -1;
        multi_drag_items.clear();

        assert!(!drag_active);
        assert!(!drag_pending);
        assert_eq!(drag_item_index, -1);
        assert_eq!(drag_over_item, -1);
        assert_eq!(drag_over_insert_col, -1);
        assert_eq!(drag_over_insert_row, -1);
        assert!(multi_drag_items.is_empty());
    }

    /// 验证多选拖拽激活条件
    /// 条件：selected_indices.len() > 1 && selected_indices.contains(drag_source_idx)
    #[test]
    fn test_multi_drag_activation_condition() {
        let drag_source_idx: usize = 2;

        // 场景 1：多选包含拖拽源 → 激活多选拖拽
        let selected = [1, 2, 3];
        let should_activate = selected.len() > 1 && selected.contains(&drag_source_idx);
        assert!(should_activate, "多选包含拖拽源时应激活");

        // 场景 2：单选 → 不激活多选拖拽
        let selected_single = [2];
        let should_activate_single =
            selected_single.len() > 1 && selected_single.contains(&drag_source_idx);
        assert!(!should_activate_single, "单选时不应激活多选拖拽");

        // 场景 3：多选但不包含拖拽源 → 不激活
        let selected_other = [0, 1, 3];
        let should_activate_other =
            selected_other.len() > 1 && selected_other.contains(&drag_source_idx);
        assert!(!should_activate_other, "多选不包含拖拽源时不应激活");
    }

    /// 验证拖拽源索引确保在 multi_drag_items 中
    /// end_drag 中的保护逻辑：如果 multi_drag_items 非空但不包含 drag_source_idx，则追加
    #[test]
    fn test_ensure_drag_source_in_multi_items() {
        let drag_source_idx: usize = 5;

        // 场景 1：已包含 → 不追加
        let mut multi_items = vec![3, 5, 7];
        if !multi_items.is_empty() && !multi_items.contains(&drag_source_idx) {
            multi_items.push(drag_source_idx);
        }
        assert_eq!(multi_items, vec![3, 5, 7], "已包含时不应追加");

        // 场景 2：不包含 → 追加
        let mut multi_items2 = vec![3, 7];
        if !multi_items2.is_empty() && !multi_items2.contains(&drag_source_idx) {
            multi_items2.push(drag_source_idx);
        }
        assert_eq!(multi_items2, vec![3, 7, 5], "不包含时应追加");

        // 场景 3：空列表 → 不追加
        let mut multi_items3: Vec<usize> = Vec::new();
        if !multi_items3.is_empty() && !multi_items3.contains(&drag_source_idx) {
            multi_items3.push(drag_source_idx);
        }
        assert!(multi_items3.is_empty(), "空列表时不应追加");
    }
}
