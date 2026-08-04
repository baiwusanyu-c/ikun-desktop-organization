//! 覆盖区域拖拽操作与文件操作

use super::overlay_grid::{overlay_grid_to_logical, overlay_snap_to_grid};
use super::overlay_render::render_desktop_overlay;
use super::overlay_window::{
    get_overlay_data, overlay_layout_params, save_overlay_icon_positions,
};
use crate::platform::windows::ffi_imports::*;
use crate::platform::windows::ffi_types::*;

pub(super) fn handle_overlay_drag_timer(hwnd: HWND) {
    log::debug!("handle_overlay_drag_timer called");
    let is_dragging = match get_overlay_data(hwnd) {
        Some(data) => data.drag_active,
        None => return,
    };
    if !is_dragging {
        log::warn!("handle_overlay_drag_timer: drag_active=false, 清理残影");
        unsafe {
            KillTimer(hwnd, TIMER_ID_OVERLAY_DRAG);
            crate::platform::windows::drag_image::destroy_drag_overlay();
        }
        if let Some(data) = get_overlay_data(hwnd) {
            data.multi_drag_items.clear();
        }
        return;
    }

    // 更新拖拽残影位置
    unsafe { crate::platform::windows::drag_image::update_drag_overlay() };

    // 检测光标位置
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };

    // ★ 优先检测外部窗口（Explorer 等）— 必须在分类框检测之前！
    // 分类框的窗口矩形可能覆盖到 Explorer 区域，导致误判。
    let ext_hwnd = crate::platform::windows::zorder::find_external_window_at_point(pt.x, pt.y);
    if !ext_hwnd.is_null() {
        log::info!("[OVERLAY-TIMER] 检测到外部窗口 {:?} at ({}, {}), 启动 OLE 拖拽", ext_hwnd, pt.x, pt.y);
        start_overlay_ole_drag(hwnd, ext_hwnd);
        return;
    }

    // 检测光标下的分类框窗口
    let target_hwnd = crate::platform::windows::zorder::find_grid_window_at_point(pt.x, pt.y);

    if !target_hwnd.is_null() {
        // 光标在分类框上：发送可视化更新消息给目标框，清除overlay的可视化
        if let Some(data) = get_overlay_data(hwnd) {
            let changed = data.drag_over_item != -1
                || data.drag_over_insert_col != -1
                || data.drag_over_insert_row != -1;
            data.drag_over_item = -1;
            data.drag_over_insert_col = -1;
            data.drag_over_insert_row = -1;
            if changed {
                render_desktop_overlay(hwnd);
            }
        }
        let lparam = ((pt.y as isize) << 16) | ((pt.x as isize) & 0xFFFF);
        unsafe {
            PostMessageW(target_hwnd, WM_USER_DRAG_OVER_UPDATE, 0, lparam as LPARAM);
        }
    } else {
        update_overlay_drag_over_visual(hwnd, pt.x, pt.y);
    }

    // 检测鼠标左键是否仍然按下
    let lbutton_down = unsafe { GetAsyncKeyState(VK_LBUTTON) } & 0x8000u16 as i16 != 0;
    if !lbutton_down {
        log::info!("overlay drag timer: 鼠标已释放，结束拖拽");
        end_overlay_drag(hwnd);
    }
}

/// 更新覆盖窗口的拖拽可视化反馈
/// 在定时器中检测到外部窗口时调用，启动 OLE 拖拽

pub(super) fn start_overlay_ole_drag(hwnd: HWND, external_hwnd: HWND) {
    log::info!("start_overlay_ole_drag: 外部窗口 {:?}", external_hwnd);
    unsafe {
        KillTimer(hwnd, TIMER_ID_OVERLAY_DRAG);
    }

    // Read drag state including multi_drag_items before clearing
    let (item_idx, multi_drag_items) = match get_overlay_data(hwnd) {
        Some(data) => {
            let idx = data.drag_item_index;
            let multi = data.multi_drag_items.clone();
            data.drag_pending = false;
            data.drag_active = false;
            data.drag_item_index = -1;
            data.drag_over_item = -1;
            data.drag_over_insert_col = -1;
            data.drag_over_insert_row = -1;
            data.multi_drag_items.clear();
            (idx, multi)
        }
        None => return,
    };

    let is_multi = multi_drag_items.len() > 1;

    if is_multi {
        // Multi-select OLE drag: collect all file paths and metadata
        let (file_paths, drag_items, icon_handle) = match get_overlay_data(hwnd) {
            Some(data) => {
                let mut paths = Vec::new();
                let mut items = Vec::new();
                let mut primary_icon = std::ptr::null_mut();
                for &idx in &multi_drag_items {
                    if idx < data.icons.len() {
                        let item = &data.icons[idx];
                        paths.push(item.file_path.clone());
                        items.push(IkunDragItemData {
                            index: idx,
                            file_path: item.file_path.clone(),
                            display_name: item.display_name.clone(),
                        });
                        if idx == item_idx as usize {
                            primary_icon = item.icon_handle;
                        }
                    }
                }
                if primary_icon.is_null() && !data.icons.is_empty() && item_idx >= 0 {
                    if (item_idx as usize) < data.icons.len() {
                        primary_icon = data.icons[item_idx as usize].icon_handle;
                    }
                }
                (paths, items, primary_icon)
            }
            None => {
                unsafe { crate::platform::windows::drag_image::destroy_drag_overlay(); }
                render_desktop_overlay(hwnd);
                return;
            }
        };

        unsafe { crate::platform::windows::drag_image::destroy_drag_overlay(); }
        log::info!("覆盖窗口→Explorer OLE (multi): {} items", file_paths.len());

        let box_drag_data = IkunBoxDragData {
            source_hwnd: hwnd as usize,
            source_config_id: String::new(),
            items: drag_items,
        };

        unsafe { ShowWindow(hwnd, SW_HIDE); }
        let drop_succeeded = crate::platform::windows::ole_dnd::start_box_drag(
            file_paths, box_drag_data, icon_handle,
        );
        unsafe { ShowWindow(hwnd, SW_SHOWNA); }

        if drop_succeeded {
            log::info!("OLE 多文件复制成功，图标保持原位");
        } else {
            log::info!("OLE 多文件取消，图标保持原位");
        }
        render_desktop_overlay(hwnd);
    } else {
        // Single-item OLE drag (existing logic)
        let (file_path, display_name, icon_handle) = match get_overlay_data(hwnd) {
            Some(data) if item_idx >= 0 && (item_idx as usize) < data.icons.len() => {
                let item = &data.icons[item_idx as usize];
                (
                    item.file_path.clone(),
                    item.display_name.clone(),
                    item.icon_handle,
                )
            }
            _ => {
                unsafe {
                    crate::platform::windows::drag_image::destroy_drag_overlay();
                }
                render_desktop_overlay(hwnd);
                return;
            }
        };

        unsafe {
            crate::platform::windows::drag_image::destroy_drag_overlay();
        }
        log::info!(
            "覆盖窗口→Explorer OLE: '{}' (path={})",
            display_name,
            file_path
        );

        let box_drag_data = IkunBoxDragData {
            source_hwnd: hwnd as usize,
            source_config_id: String::new(),
            items: vec![IkunDragItemData {
                index: item_idx as usize,
                file_path: file_path.clone(),
                display_name: display_name.clone(),
            }],
        };

        // 隐藏覆盖窗口，让 OLE hit-testing 能到达 Explorer
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        let drop_succeeded = crate::platform::windows::ole_dnd::start_box_drag(
            vec![file_path],
            box_drag_data,
            icon_handle,
        );
        unsafe {
            ShowWindow(hwnd, SW_SHOWNA);
        }

        if drop_succeeded {
            log::info!("OLE 复制成功，图标保持原位");
        } else {
            log::info!("OLE 取消，图标保持原位");
        }
        // 图标始终保持原位（复制操作，不是移动）
        render_desktop_overlay(hwnd);
    }
    // OLE 拖拽结束后清除全局多选状态
    super::super::multi_select::clear();
}


pub(super) fn update_overlay_drag_over_visual(hwnd: HWND, screen_x: i32, screen_y: i32) {
    let data = match get_overlay_data(hwnd) {
        Some(d) => d,
        None => return,
    };

    let dragged_idx = data.drag_item_index;
    // dragged_idx == -1 表示外部 OLE 拖拽，仍需显示可视化反馈

    // 存储光标屏幕坐标（用于渲染时获取本地 DPI）
    data.drag_cursor_screen_x = screen_x;
    data.drag_cursor_screen_y = screen_y;

    // 转换为overlay窗口坐标
    let local_x = screen_x - data.x;
    let local_y = screen_y - data.y;

    // 使用光标所在显示器的本地 DPI 计算网格参数
    let local_dpi = crate::platform::windows::desktop_overlay::overlay_window::dpi_at_screen_point(
        screen_x, screen_y,
    );
    let grid_layout = super::overlay_window::overlay_layout_params(data.width, local_dpi);
    let padding = grid_layout.padding;
    let cell_w = grid_layout.cell_w;
    let cell_h = grid_layout.cell_h;
    let cols = grid_layout.cols;

    // 光标是否在网格区域内？
    if local_x < padding || local_y < padding {
        let changed = data.drag_over_item != -1
            || data.drag_over_insert_col != -1
            || data.drag_over_insert_row != -1;
        data.drag_over_item = -1;
        data.drag_over_insert_col = -1;
        data.drag_over_insert_row = -1;
        if changed {
            render_desktop_overlay(hwnd);
        }
        return;
    }

    // 计算光标所在的网格列和行（列优先布局）
    let total = data.icons.len().max(1) as i32;
    let rows = (total + cols - 1) / cols;
    // 行的上限基于覆盖窗口的实际高度（而非图标数量），
    // 确保分隔线能跟随光标在全屏范围内的 Y 位置移动。
    let max_rows = ((data.height - padding * 2) / cell_h).max(1);
    let col = ((local_x - padding) / cell_w).min(cols - 1).max(0);
    let row = ((local_y - padding) / cell_h).min(max_rows - 1).max(0);

    // 计算光标在单元格内的相对X位置（0.0 ~ 1.0）
    let cell_offset_x = local_x - padding - col * cell_w;
    let ratio_x = cell_offset_x as f32 / cell_w as f32;

    // 查找该单元格上是否有图标（列优先：index = col * rows + row）
    let item_at_cell = {
        let idx = (col * rows + row) as usize;
        if idx < data.icons.len() && (dragged_idx < 0 || (idx as i32) != dragged_idx) {
            idx as i32
        } else {
            -1
        }
    };

    const INSERT_THRESHOLD: f32 = 0.30;

    let (new_over_item, new_insert_col, new_insert_row) = if ratio_x < INSERT_THRESHOLD {
        (-1, col, row)
    } else if ratio_x > 1.0 - INSERT_THRESHOLD {
        if col + 1 < cols {
            (-1, col + 1, row)
        } else {
            (item_at_cell, -1, -1)
        }
    } else {
        (item_at_cell, -1, -1)
    };

    let changed = data.drag_over_item != new_over_item
        || data.drag_over_insert_col != new_insert_col
        || data.drag_over_insert_row != new_insert_row;

    data.drag_over_item = new_over_item;
    data.drag_over_insert_col = new_insert_col;
    data.drag_over_insert_row = new_insert_row;

    if changed {
        render_desktop_overlay(hwnd);
    }
}


pub(super) fn start_overlay_drag(hwnd: HWND) {
    log::info!("start_overlay_drag called");
    let (icon_handle, icon_size, cell_w, display_name) = match get_overlay_data(hwnd) {
        Some(data) => {
            let item_idx = data.selected_item;
            if item_idx < 0 || (item_idx as usize) >= data.icons.len() {
                data.drag_pending = false;
                return;
            }
            data.drag_pending = false;
            data.drag_active = true;
            data.drag_item_index = item_idx;

            // Check multi-select: if multiple items are selected and the clicked
            // item is among them, activate multi-drag mode.
            let container_type = super::super::multi_select::ContainerType::Overlay;
            let container_id: u64 = 0;
            let selected_indices = super::super::multi_select::selected_in_container(container_type, container_id);
            if selected_indices.len() > 1 && selected_indices.contains(&(item_idx as usize)) {
                log::info!("[OVERLAY-DRAG] Multi-drag activated: {} items", selected_indices.len());
                data.multi_drag_items = selected_indices;
            } else {
                data.multi_drag_items.clear();
            }

            let item = &data.icons[item_idx as usize];
            (
                item.icon_handle,
                data.icon_size,
                data.cell_w,
                item.display_name.clone(),
            )
        }
        None => return,
    };

    log::info!("覆盖窗口拖拽激活: {}", display_name);
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    unsafe {
        crate::platform::windows::drag_image::create_drag_overlay(
            icon_handle,
            icon_size,
            cell_w,
            &display_name,
            dpi,
        );
        crate::platform::windows::drag_image::update_drag_overlay();
        // WS_EX_LAYERED 窗口上 SetCapture 不稳定，使用 Timer 轮询检测鼠标释放
        log::info!(
            "start_overlay_drag: before SetTimer, drag_active={}",
            get_overlay_data(hwnd)
                .map(|d| d.drag_active)
                .unwrap_or(false)
        );
        SetTimer(hwnd, TIMER_ID_OVERLAY_DRAG, 8, std::ptr::null_mut());
        log::info!(
            "start_overlay_drag: after SetTimer, drag_active={}",
            get_overlay_data(hwnd)
                .map(|d| d.drag_active)
                .unwrap_or(false)
        );
    }
}


pub(super) fn end_overlay_drag(hwnd: HWND) {
    log::info!("end_overlay_drag called");
    let (item_idx, multi_drag_items) = match get_overlay_data(hwnd) {
        Some(data) => {
            let item_idx = data.drag_item_index;
            let multi_drag_items = data.multi_drag_items.clone();
            data.drag_active = false;
            data.drag_pending = false;
            data.drag_item_index = -1;
            // 清除拖拽覆盖可视化状态
            data.drag_over_item = -1;
            data.drag_over_insert_col = -1;
            data.drag_over_insert_row = -1;
            data.multi_drag_items.clear();
            (item_idx, multi_drag_items)
        }
        None => return,
    };

    unsafe {
        KillTimer(hwnd, TIMER_ID_OVERLAY_DRAG);
        crate::platform::windows::drag_image::destroy_drag_overlay();
    }

    let is_multi_drag = multi_drag_items.len() > 1;

    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    // ★ 优先检测外部窗口（Explorer 等），避免分类框窗口矩形误判
    let ext_hwnd = crate::platform::windows::zorder::find_external_window_at_point(pt.x, pt.y);
    let target_hwnd = if !ext_hwnd.is_null() {
        log::info!("[END-OVERLAY-DRAG] 外部窗口优先: {:?}", ext_hwnd);
        std::ptr::null_mut()
    } else {
        crate::platform::windows::zorder::find_grid_window_at_point(pt.x, pt.y)
    };

    if target_hwnd.is_null() {
        // Dropped on overlay (same container) — restore positions
        if is_multi_drag {
            log::info!("Multi-drag to overlay: {} items", multi_drag_items.len());
            for &idx in &multi_drag_items {
                restore_dragged_icon_position(hwnd, idx as i32, pt.x, pt.y);
            }
        } else {
            restore_dragged_icon_position(hwnd, item_idx, pt.x, pt.y);
        }
        log::info!("覆盖窗口内图标位置已更新");
        super::super::multi_select::clear();
        return;
    }

    // Dropped on a grid box — move items from overlay to grid
    if is_multi_drag {
        log::info!("Multi-drag overlay→grid: {} items", multi_drag_items.len());

        // Compute target grid layout once
        let Some(twr) = crate::platform::windows::grid_window::get_grid_content_rect(target_hwnd)
        else {
            log::warn!("overlay→grid: 无法读取目标分类框内容矩形，取消放置");
            return;
        };
        let Some((target_cols, target_cell_w, target_cell_h, target_grid_left, target_grid_top)) = (unsafe {
            crate::platform::windows::grid_window::get_data(target_hwnd).map(|d| {
                let tdpi = GetDpiForWindow(target_hwnd);
                let tdpi = if tdpi > 0 { tdpi } else { 96 };
                let ww = twr.right - twr.left;
                let vp = crate::box_core::view_engine::get_view_params(&d.view_mode, tdpi);
                let layout =
                    crate::box_core::layout_calc::calculate_grid(ww, d.title_h, &vp, tdpi);
                (
                    d.cols,
                    layout.cell_w.max(1),
                    layout.cell_h.max(1),
                    layout.grid_left,
                    layout.grid_top,
                )
            })
        }) else {
            log::warn!("overlay→grid: 目标分类框数据已失效，取消放置");
            return;
        };

        // Compute base grid cell from drop position
        let client_x = pt.x - twr.left;
        let client_y = pt.y - twr.top;
        let base_col = ((client_x - target_grid_left) / target_cell_w).max(0);
        let base_row = ((client_y - target_grid_top) / target_cell_h).max(0);
        let base_lin = base_row * target_cols + base_col;

        // Collect items info from overlay (in sorted order)
        let mut sorted_indices = multi_drag_items.clone();
        sorted_indices.sort();

        struct DragItemInfo {
            file_path: String,
            display_name: String,
            grid_col: i32,
            grid_row: i32,
        }

        let mut items_to_add: Vec<DragItemInfo> = Vec::new();
        for (i, &drag_idx) in sorted_indices.iter().enumerate() {
            let (file_path, display_name) = match get_overlay_data(hwnd) {
                Some(data) if drag_idx < data.icons.len() => {
                    let item = &data.icons[drag_idx];
                    (item.file_path.clone(), item.display_name.clone())
                }
                _ => continue,
            };

            // Each item gets a different linear grid position
            let lin = base_lin + i as i32;
            let item_col = lin % target_cols;
            let item_row = lin / target_cols;

            log::info!("[MULTI-DROP-DIRECT] item {} (idx={}): grid=({}, {}), lin={}",
                i, drag_idx, item_col, item_row, lin);

            items_to_add.push(DragItemInfo {
                file_path,
                display_name,
                grid_col: item_col,
                grid_row: item_row,
            });
        }

        // Public 图标先完成收纳，授权失败时保持覆盖层和分类框原状。
        let public_items: Vec<_> = items_to_add
            .iter()
            .filter(|item| crate::platform::windows::icon_hider::is_public_desktop_path(&item.file_path))
            .collect();
        let mut ingested_public = Vec::new();
        for item in public_items {
            if !crate::platform::windows::drag_drop::hide_single_desktop_icon(
                &item.file_path,
                &item.display_name,
            ) {
                for path in ingested_public {
                    let _ = crate::platform::windows::icon_hider::export_icon(path);
                }
                log::warn!("公共图标收纳失败，取消多选拖入分类框: {}", item.file_path);
                return;
            }
            ingested_public.push(item.file_path.clone());
        }

        // ★ 直接操作目标网格数据（绕过消息队列，避免渲染时自动布局覆盖坐标）
        if let Some(target_data) = unsafe { crate::platform::windows::grid_window::get_data(target_hwnd) } {
            // Materialize all existing auto-layout positions first
            for (idx, item) in target_data.items.iter_mut().enumerate() {
                if item.grid_col < 0 {
                    item.grid_col = (idx as i32) % target_data.cols;
                    item.grid_row = (idx as i32) / target_data.cols;
                }
            }

            // Add all new items directly
            for info in &items_to_add {
                // Skip duplicates
                if target_data.items.iter().any(|i| i.file_path == info.file_path) {
                    log::debug!("[MULTI-DROP-DIRECT] 跳过已存在项: {}", info.file_path);
                    continue;
                }

                let read_path = crate::platform::windows::drag_drop::resolve_drag_item_read_path(&info.file_path);
                let read_path_str = read_path.to_string_lossy().to_string();
                let icon_handle = crate::platform::windows::icon_extract::extract_file_icon(&read_path_str)
                    .map(|ic| ic.hicon)
                    .unwrap_or(std::ptr::null_mut());

                target_data.items.push(crate::platform::windows::grid_window::GridFileItem {
                    file_path: info.file_path.clone(),
                    display_name: info.display_name.clone(),
                    icon_handle,
                    grid_col: info.grid_col,
                    grid_row: info.grid_row,
                });
                log::info!("[MULTI-DROP-DIRECT] 直接添加: '{}' grid=({}, {})",
                    info.display_name, info.grid_col, info.grid_row);

                // Hide desktop icon
                if !crate::platform::windows::icon_hider::is_public_desktop_path(&info.file_path) {
                    crate::platform::windows::drag_drop::hide_single_desktop_icon(&info.file_path, &info.display_name);
                }
            }

            // 验证日志：打印所有项的网格坐标
            log::info!("[VERIFY] 目标框总项数: {}", target_data.items.len());
            for (vi, vitem) in target_data.items.iter().enumerate() {
                log::info!("[VERIFY] item[{}]: '{}' grid=({}, {})",
                    vi, vitem.display_name, vitem.grid_col, vitem.grid_row);
            }

            // Render and save once
            crate::platform::windows::d2d_render::render_layered_window(target_hwnd);
            crate::platform::windows::drag_handler::save_config();
        }

        // Remove items from overlay in reverse order
        for &drag_idx in sorted_indices.iter().rev() {
            if let Some(data) = get_overlay_data(hwnd) {
                if drag_idx < data.icons.len() {
                    let removed = data.icons.remove(drag_idx);
                    crate::platform::windows::icon_extract::destroy_icon(removed.icon_handle);
                }
            }
        }

        if let Some(data) = get_overlay_data(hwnd) {
            data.selected_item = -1;
            data.hovered_item = -1;
        }
        render_desktop_overlay(hwnd);
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_LEAVE, 0, 0) };
        super::super::multi_select::clear();
        log::info!("覆盖窗口→分类框: {} items (multi, direct)", items_to_add.len());
    } else {
        // Single-item drag (existing logic)
        let (file_path, display_name) = match get_overlay_data(hwnd) {
            Some(data) => {
                if item_idx < 0 || (item_idx as usize) >= data.icons.len() {
                    return;
                }
                let item = &data.icons[item_idx as usize];
                (item.file_path.clone(), item.display_name.clone())
            }
            None => return,
        };

        let public_preflight = crate::platform::windows::icon_hider::is_public_desktop_path(&file_path);
        if public_preflight
            && !crate::platform::windows::drag_drop::hide_single_desktop_icon(
                &file_path,
                &display_name,
            )
        {
            log::warn!("公共图标收纳失败，取消拖入分类框: {}", file_path);
            return;
        }

        // 读取目标框的拖拽覆盖状态（用于插入模式）
        let (insert_col, insert_row) = unsafe {
            crate::platform::windows::grid_window::get_data(target_hwnd)
                .map(|d| (d.drag_over_insert_col, d.drag_over_insert_row))
                .unwrap_or((-1, -1))
        };

        crate::platform::windows::drop_target::set_pending_add_item(
            file_path,
            display_name.clone(),
            pt.x,
            pt.y,
            insert_col,
            insert_row,
        );
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_ITEM_ADDED, 0, 0) };

        if let Some(data) = get_overlay_data(hwnd) {
            if item_idx >= 0 && (item_idx as usize) < data.icons.len() {
                let removed = data.icons.remove(item_idx as usize);
                crate::platform::windows::icon_extract::destroy_icon(removed.icon_handle);
                data.selected_item = -1;
                data.hovered_item = -1;
            }
        }
        render_desktop_overlay(hwnd);
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_LEAVE, 0, 0) };
        super::super::multi_select::clear();
        log::info!("覆盖窗口→分类框: {}", display_name);
    }
}


pub(super) fn restore_dragged_icon_position(hwnd: HWND, item_idx: i32, screen_x: i32, screen_y: i32) {
    if let Some(data) = get_overlay_data(hwnd) {
        if item_idx < 0 || (item_idx as usize) >= data.icons.len() {
            return;
        }
        let local_x = screen_x.saturating_sub(data.x);
        let local_y = screen_y.saturating_sub(data.y);
        // Materialize dragged icon's auto-layout position before swap
        // so the swapped icon gets a concrete position, not (-1, -1)
        if data.icons[item_idx as usize].x < 0.0 || data.icons[item_idx as usize].y < 0.0 {
            let layout = overlay_layout_params(data.width, data.dpi);
            let total = data.icons.len().max(1) as i32;
            let rows = (total + layout.cols - 1) / layout.cols;
            let auto_col = (item_idx as i32) / rows;
            let auto_row = (item_idx as i32) % rows;
            let resolved = overlay_grid_to_logical(data, auto_col, auto_row);
            data.icons[item_idx as usize].x = resolved[0];
            data.icons[item_idx as usize].y = resolved[1];
        }
        let (snap_pos, occupied_by) = overlay_snap_to_grid(data, local_x, local_y, item_idx);
        if let Some(occupied_idx) = occupied_by {
            let drag_x = data.icons[item_idx as usize].x;
            let drag_y = data.icons[item_idx as usize].y;
            data.icons[occupied_idx].x = drag_x;
            data.icons[occupied_idx].y = drag_y;
        }
        data.icons[item_idx as usize].x = snap_pos[0];
        data.icons[item_idx as usize].y = snap_pos[1];
        data.selected_item = item_idx;
        data.hovered_item = item_idx;
    }
    render_desktop_overlay(hwnd);
    save_overlay_icon_positions();
}


pub(super) fn highlight_grid_under_cursor(source_hwnd: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    // 使用 find_grid_window_at_point 替代 WindowFromPoint，
    // 因为分类框在 HWND_BOTTOM，WindowFromPoint 会被覆盖窗口拦截
    let target_hwnd = crate::platform::windows::zorder::find_grid_window_at_point(pt.x, pt.y);
    if !target_hwnd.is_null() && target_hwnd != source_hwnd {
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_ENTER, 0, 0) };
    }
}


pub(super) fn is_grid_window(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let mut class_name = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
    if len <= 0 {
        return false;
    }
    String::from_utf16_lossy(&class_name[..len as usize]) == "DesktopOrgGrid"
}

/// 从 WM_DPICHANGED 的 WPARAM 提取新 DPI 值。
///
/// HIWORD(wParam) 包含新 DPI，如果为 0 则回退到 96。

pub(super) fn handle_overlay_ole_drop(_hwnd: HWND) {
    let drop = match super::overlay_drop_target::take_pending_overlay_drop() {
        Some(d) => d,
        None => {
            log::debug!("覆盖窗口 OLE drop: 无待处理数据");
            return;
        }
    };

    log::info!(
        "覆盖窗口 OLE drop: {} 个文件, drop=({}, {})",
        drop.file_paths.len(),
        drop.drop_x,
        drop.drop_y
    );

    let mut added = 0;
    for file_path in &drop.file_paths {
        let is_desktop_file = crate::platform::windows::icon_hider::is_user_desktop_path(file_path)
            || crate::platform::windows::icon_hider::is_public_desktop_path(file_path);

        // 对于非桌面文件：复制到用户桌面（模拟原生桌面拖拽的复制行为）
        let effective_path = if !is_desktop_file {
            match copy_file_to_desktop(file_path) {
                Some(desktop_path) => {
                    log::info!(
                        "覆盖窗口 drop: 非桌面文件已复制到桌面: {} -> {}",
                        file_path,
                        desktop_path
                    );
                    desktop_path
                }
                None => {
                    log::warn!("覆盖窗口 drop: 复制文件到桌面失败: {}", file_path);
                    file_path.clone()
                }
            }
        } else {
            file_path.clone()
        };

        // 提取显示名称和图标
        let icon_info = crate::platform::windows::icon_extract::extract_file_icon(&effective_path);
        let display_name = match &icon_info {
            Some(info) => info.display_name.clone(),
            None => std::path::Path::new(&effective_path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| effective_path.clone()),
        };

        // 添加到覆盖窗口（在鼠标释放位置显示）
        let shown = super::overlay_window::add_icon_to_overlay(
            effective_path.clone(),
            display_name.clone(),
            drop.drop_x,
            drop.drop_y,
        );

        if shown {
            added += 1;
            log::info!("覆盖窗口 drop: '{}' 已添加到覆盖区域", display_name);
        }

        // 对于桌面文件，收入仓库（与现有桌面→框行为一致）
        if is_desktop_file {
            if let Err(e) = crate::platform::windows::icon_hider::ingest_icon(&effective_path) {
                log::warn!("覆盖窗口 drop: 收入仓库失败 '{}': {}", display_name, e);
            }
        }
    }

    if added > 0 {
        log::info!("覆盖窗口 OLE drop: 添加 {} 个图标到覆盖区域", added);
        // 强制刷新覆盖窗口，确保图标立即显示
        super::overlay_window::request_desktop_overlay_refresh();
    }
}

/// 将文件复制到用户桌面，返回桌面路径

pub(super) fn copy_file_to_desktop(source_path: &str) -> Option<String> {
    use std::path::Path;

    let source = Path::new(source_path);
    if !source.exists() {
        log::warn!("copy_file_to_desktop: 源文件不存在: {}", source_path);
        return None;
    }

    let file_name = source.file_name()?.to_string_lossy().to_string();
    let desktop = dirs::desktop_dir()?;
    let dest = desktop.join(&file_name);

    // 如果桌面已有同名文件，生成唯一名称
    let dest = if dest.exists() {
        let stem = source.file_stem()?.to_string_lossy().to_string();
        let ext = source
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut counter = 1;
        loop {
            let new_name = format!("{} ({}){}", stem, counter, ext);
            let new_dest = desktop.join(&new_name);
            if !new_dest.exists() {
                break new_dest;
            }
            counter += 1;
            if counter > 100 {
                log::warn!("copy_file_to_desktop: 无法生成唯一文件名");
                return None;
            }
        }
    } else {
        dest
    };

    match std::fs::copy(source, &dest) {
        Ok(_) => {
            crate::platform::windows::icon_hider::notify_shell_create(&dest.to_string_lossy());
            Some(dest.to_string_lossy().to_string())
        }
        Err(e) => {
            log::warn!(
                "copy_file_to_desktop: 复制失败 {} -> {}: {}",
                source_path,
                dest.display(),
                e
            );
            None
        }
    }
}

