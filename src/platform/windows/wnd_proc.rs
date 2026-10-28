//! 格子窗口消息处理（wnd_proc）
//!
//! 包含 `grid_wnd_proc` 窗口过程，处理所有 Win32 消息：
//! - WM_NCHITTEST：标题栏拖拽 + 边缘缩放 + 锁定按钮 + hover_edge 同步 + 编辑模式锁定
//! - WM_SETCURSOR：边缘缩放手柄 + 锁定/更多按钮手型 + 编辑模式 I 型光标
//! - WM_SIZE / WM_DPICHANGED：布局重算 + 重绘
//! - WM_MOUSEMOVE：锁定按钮 hover + 编辑模式清除/保存 hover + 图标 hover + tooltip
//! - WM_MOUSELEAVE：重置所有 hover 状态
//! - WM_LBUTTONDOWN：编辑模式清除/保存按钮 + 锁定按钮切换 + 图标选中 + "更多"菜单
//! - WM_LBUTTONDBLCLK：标题栏双击进入名称编辑 + 双击打开文件
//! - WM_CHAR / WM_KEYDOWN：默认处理（名称编辑由 rename_editor 独立线程 EDIT 控件实现）
//! - WM_IME_COMPOSITION：默认处理
//! - WM_RBUTTONUP：Shell 上下文菜单
//! - WM_DESTROY：释放图标句柄 + D2D/DW/WIC 资源
//! - WM_WINDOWPOSCHANGING：Z-order 管理（调用 zorder 模块，分类框不高于其他应用窗口）

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;

extern "system" {
    fn GetKeyState(nVirtKey: i32) -> i16;
}
const VK_LBUTTON: i32 = 0x01;
use super::ffi_types::*;
use super::grid_layout::{
    grid_hit_test, lock_btn_rect, more_btn_rect, recalculate_layout, reflow_items,
};
use super::grid_window::{
    client_to_content_point_with_padding, get_data, get_grid_content_rect, shadow_padding_for_dpi,
    window_padding, GridWindowData,
};
use super::menu_window::show_menu_window;
use super::shell_menu::{
    show_desktop_context_menu, show_multi_select_shell_context_menu, show_shell_context_menu,
};
use super::zorder::{
    is_drag_in_progress, is_zorder_suppressing, push_other_grids_below, with_zorder_suppressed,
};
use crate::box_core::layout_calc::calculate_grid;
use crate::box_core::view_engine::get_view_params;

// 跟踪拖拽状态，用于检测 Windows 是否 clamp 了窗口位置
thread_local! {
    static IN_DRAG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static IN_MOVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };  // 区分拖拽移动和边缘缩放

    static PROPOSED_X: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static PROPOSED_Y: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

thread_local! { static DRAG_HWND: std::cell::Cell<HWND> = const { std::cell::Cell::new(std::ptr::null_mut()) }; }
thread_local! { static CUSTOM_MOVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
thread_local! { static GRAB_DX: std::cell::Cell<i32> = const { std::cell::Cell::new(0) }; }
thread_local! { static GRAB_DY: std::cell::Cell<i32> = const { std::cell::Cell::new(0) }; }

fn window_dpi(hwnd: HWND) -> u32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi > 0 {
        dpi
    } else {
        96
    }
}

fn content_point_from_lparam(hwnd: HWND, lparam: LPARAM) -> (i32, i32) {
    let client_x = get_x_lparam(lparam);
    let client_y = get_y_lparam(lparam);
    let mut screen_point = POINT {
        x: client_x,
        y: client_y,
    };
    if unsafe { ClientToScreen(hwnd, &mut screen_point) } != 0 {
        if let Some(content_rect) = get_grid_content_rect(hwnd) {
            return (
                screen_point.x - content_rect.left,
                screen_point.y - content_rect.top,
            );
        }
    }

    // 留白随液态玻璃开关与 DPI 变化，必须读窗口当前生效值，
    // 否则刚切换开关时命中测试会整体偏移一个留白的距离。
    let padding = window_padding(hwnd, window_dpi(hwnd));
    client_to_content_point_with_padding(padding, client_x, client_y)
}

fn remove_grid_item_if_missing_after_shell_menu(hwnd: HWND, logical_path: &str) {
    if super::icon_hider::icon_exists_for_box(logical_path) {
        return;
    }

    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return;
    };
    let Some(index) = data
        .items
        .iter()
        .position(|item| item.file_path == logical_path)
    else {
        return;
    };

    let removed = data.items.remove(index);
    if !removed.icon_handle.is_null() {
        super::icon_extract::destroy_icon(removed.icon_handle);
    }
    if data.selected_item >= data.items.len() as i32 {
        data.selected_item = -1;
    }
    if data.hovered_item >= data.items.len() as i32 {
        data.hovered_item = -1;
    }
    log::info!(
        "右键菜单操作后图标文件不存在，已从分类框移除: {}",
        removed.display_name
    );
    render_layered_window(hwnd);
    super::drag_handler::save_config();
}

/// Handle shell menu result for grid window
fn handle_shell_menu_result(
    hwnd: HWND,
    file_path: &str,
    hit_index: i32,
    result: super::shell_menu::ShellMenuResult,
    staging_path: Option<std::path::PathBuf>,
) {
    use super::shell_menu::ShellMenuResult;

    match result {
        ShellMenuResult::Cancelled | ShellMenuResult::Dismissed => {}
        ShellMenuResult::Completed(_) => {
            // Check if file still exists after shell operation
            remove_grid_item_if_missing_after_shell_menu(hwnd, file_path);
        }
        ShellMenuResult::NeedsRename(path) => {
            log::info!("Rename requested for: {}", path);
            let Some(data) = (unsafe { get_data(hwnd) }) else {
                log::warn!("Failed to get grid data for rename");
                return;
            };
            let item_idx = hit_index as usize;
            if item_idx >= data.items.len() {
                return;
            }

            let current_name = data.items[item_idx].display_name.clone();

            // 计算标签位置（屏幕坐标）
            let Some(wr) = get_grid_content_rect(hwnd) else {
                return;
            };
            let dpi = window_dpi(hwnd);
            let view_params = get_view_params(&data.view_mode, dpi);
            let layout = calculate_grid(wr.right - wr.left, data.title_h, &view_params, dpi);

            let (cell_x, cell_y) =
                if data.items[item_idx].grid_col >= 0 && data.items[item_idx].grid_row >= 0 {
                    (
                        layout.grid_left + data.items[item_idx].grid_col * layout.cell_w,
                        layout.grid_top + data.items[item_idx].grid_row * layout.cell_h,
                    )
                } else {
                    let col = (item_idx as i32) % layout.cols;
                    let row = (item_idx as i32) / layout.cols;
                    (
                        layout.grid_left + col * layout.cell_w,
                        layout.grid_top + row * layout.cell_h,
                    )
                };

            let gap = scale_for_dpi(6, dpi);
            let text_y = cell_y + data.icon_size + 4 + gap;
            let text_height = scale_for_dpi(20, dpi);
            let text_width = layout.cell_w - 4;
            let screen_x = wr.left + cell_x + 2;
            let screen_y = wr.top + text_y;

            // 使用原生 EDIT 控件（异步模式）
            let success = super::rename_editor::show_rename_editor_async(
                hwnd,
                &current_name,
                screen_x,
                screen_y,
                text_width,
                text_height,
                hit_index,
            );

            if success {
                log::info!(
                    "Rename editor shown for item {}: '{}'",
                    hit_index,
                    current_name
                );
                // 标记正在重命名的项，隐藏标签文本
                data.renaming_item_index = hit_index;
                super::d2d_render::render_layered_window(hwnd);
            } else {
                log::error!("Failed to show rename editor");
            }
        }
        ShellMenuResult::NeedsCutMonitor(_staging_path) => {
            log::info!(
                "[CUT] starting cut monitor: staging={:?}, original={}",
                staging_path,
                file_path
            );
            if let Some(ref sp) = staging_path {
                start_cut_monitor(
                    hwnd,
                    sp.to_string_lossy().to_string(),
                    file_path.to_string(),
                );
            } else {
                start_cut_monitor(hwnd, file_path.to_string(), file_path.to_string());
            }
        }
        ShellMenuResult::NeedsDelete(_staging_path) => {
            // Shell 已将 staging 文件移入回收站，使用原始 file_path 删除 vault 副本
            log::info!("[DELETE] 删除 vault 副本: {}", file_path);
            super::icon_hider::delete_vault_copy_for(file_path);
            remove_grid_item_if_missing_after_shell_menu(hwnd, file_path);
        }
    }
}

/// Start cut monitor: background thread checks if file is moved (pasted elsewhere)
fn start_cut_monitor(hwnd: HWND, monitor_path: String, cleanup_path: String) {
    let hwnd_usize = hwnd as usize;
    std::thread::spawn(move || {
        let hwnd = hwnd_usize as HWND;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_secs(3));
            if !std::path::Path::new(&monitor_path).exists() {
                log::info!("[CUT] 文件已被粘贴移走: {}", monitor_path);
                super::icon_hider::delete_vault_copy_for(&cleanup_path);
                unsafe {
                    PostMessageW(hwnd, WM_USER_CUT_FILE_MOVED, 0, 0);
                }
                return;
            }
        }
        log::debug!("cut monitor timeout, file not pasted: {}", monitor_path);
    });
}

/// 处理多选 Shell 菜单结果
///
/// 与单选版本的 handle_shell_menu_result 类似，但处理多个文件。
fn handle_multi_shell_menu_result(hwnd: HWND, response: super::shell_menu::MultiShellMenuResponse) {
    use super::shell_menu::ShellMenuResult;

    match response.result {
        ShellMenuResult::Cancelled | ShellMenuResult::Dismissed => {
            // 清理所有 staging
            for p in response.staging_paths.iter().flatten() {
                super::icon_hider::cleanup_staging(p);
            }
        }
        ShellMenuResult::Completed(_) => {
            // 注意：不清理 staging！复制操作的文件可能仍在剪贴板上，
            // 用户后续粘贴时需要 staging 文件存在。
            // 这与单选版本 handle_shell_menu_result 的行为一致。
            // 对每个文件检查是否还存在，不存在则从 grid 中移除
            for file_path in &response.file_paths {
                remove_grid_item_if_missing_after_shell_menu(hwnd, file_path);
            }
        }
        ShellMenuResult::NeedsRename(_) => {
            // 多选时不支持重命名（try_windows_crate_multi_context_menu 已排除）
            // 清理 staging
            for p in response.staging_paths.iter().flatten() {
                super::icon_hider::cleanup_staging(p);
            }
        }
        ShellMenuResult::NeedsCutMonitor(_) => {
            // 为每个文件启动 cut monitor
            for (i, file_path) in response.file_paths.iter().enumerate() {
                let staging = response.staging_paths.get(i).and_then(|s| s.as_ref());
                log::info!(
                    "[多选菜单][CUT] 启动 cut monitor: staging={:?}, original={}",
                    staging,
                    file_path
                );
                if let Some(sp) = staging {
                    start_cut_monitor(
                        hwnd,
                        sp.to_string_lossy().to_string(),
                        file_path.to_string(),
                    );
                } else {
                    start_cut_monitor(hwnd, file_path.to_string(), file_path.to_string());
                }
            }
        }
        ShellMenuResult::NeedsDelete(_) => {
            // 删除每个文件的 vault 副本
            for file_path in &response.file_paths {
                log::info!("[多选菜单][DELETE] 删除 vault 副本: {}", file_path);
                super::icon_hider::delete_vault_copy_for(file_path);
                remove_grid_item_if_missing_after_shell_menu(hwnd, file_path);
            }
            // 清理 staging
            for p in response.staging_paths.iter().flatten() {
                super::icon_hider::cleanup_staging(p);
            }
        }
    }
}

/// 格子窗口消息处理过程
///
/// 处理标题栏拖拽、边缘缩放、锁定按钮、图标选中、双击打开、右键菜单等。
/// 边缘 resize 指示条由 `hover_edge` 驱动，在 WM_NCHITTEST 中与实际 hit test 结果同步更新。
pub(super) unsafe extern "system" fn grid_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // ===== WM_PAINT（分层窗口使用 UpdateLayeredWindow，此处仅验证绘制）=====
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let _hdc = BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }

        // ===== Hit Test：标题栏拖拽 + 边缘缩放（含锁定状态）=====
        WM_NCHITTEST => {
            let screen_x = get_x_lparam(lparam);
            let screen_y = get_y_lparam(lparam);

            let Some(wr) = get_grid_content_rect(hwnd) else {
                return HTTRANSPARENT as LRESULT;
            };
            let dx = screen_x - wr.left;
            let dy = screen_y - wr.top;
            let win_w = wr.right - wr.left;
            let win_h = wr.bottom - wr.top;

            let data_opt = get_data(hwnd);
            let title_h = data_opt.as_ref().map(|d| d.title_h).unwrap_or(40);
            let is_locked = data_opt.as_ref().map(|d| d.locked).unwrap_or(false);
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };

            // The layered window includes transparent padding for the complete shadow.
            // Only the visible content rectangle participates in interaction.
            if dx < 0 || dx >= win_w || dy < 0 || dy >= win_h {
                if let Some(data) = get_data(hwnd) {
                    if data.hover_edge != 0 {
                        data.hover_edge = 0;
                        render_layered_window(hwnd);
                    }
                }
                return HTTRANSPARENT as LRESULT;
            }

            // 锁定按钮区域（标题栏右侧）→ 始终返回 HTCLIENT（可点击，不触发拖拽）
            let (btn_x, btn_y, btn_size) = lock_btn_rect(win_w, title_h, dpi);
            if dy >= btn_y && dy < btn_y + btn_size && dx >= btn_x && dx < btn_x + btn_size {
                if let Some(data) = get_data(hwnd) {
                    if data.hover_edge != 0 {
                        data.hover_edge = 0;
                        render_layered_window(hwnd);
                    }
                }
                return HTCLIENT as LRESULT;
            }

            // "更多"按钮区域 → 始终返回 HTCLIENT
            let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(win_w, title_h, dpi);
            if dy >= mbtn_y && dy < mbtn_y + mbtn_size && dx >= mbtn_x && dx < mbtn_x + mbtn_size {
                if let Some(data) = get_data(hwnd) {
                    if data.hover_edge != 0 {
                        data.hover_edge = 0;
                        render_layered_window(hwnd);
                    }
                }
                return HTCLIENT as LRESULT;
            }

            // 菜单打开时，分类框已锁定，全部返回 HTCLIENT
            if let Some(ref d) = data_opt {
                if d.menu.open {
                    return HTCLIENT as LRESULT;
                }
            }

            // 锁定时禁止拖拽移动和边缘缩放（除锁定按钮外全部返回 HTCLIENT）
            if is_locked {
                if let Some(data) = get_data(hwnd) {
                    if data.hover_edge != 0 {
                        data.hover_edge = 0;
                        render_layered_window(hwnd);
                    }
                }
                return HTCLIENT as LRESULT;
            }

            // 未锁定：正常边缘缩放检测（统一计算 result，最后更新 hover_edge）
            let edge = scale_for_dpi(8, dpi);
            let result: i32;
            let new_edge: i32;

            if dy >= title_h {
                // 主体区域边缘检测
                let on_left = dx >= 0 && dx < edge;
                let on_right = dx > win_w - edge && dx <= win_w;
                let on_bottom = dy > win_h - edge && dy <= win_h;

                if on_left && on_bottom {
                    result = HTBOTTOMLEFT;
                    new_edge = 1;
                } else if on_right && on_bottom {
                    result = HTBOTTOMRIGHT;
                    new_edge = 2;
                } else if on_left {
                    result = HTLEFT;
                    new_edge = 1;
                } else if on_right {
                    result = HTRIGHT;
                    new_edge = 2;
                } else if on_bottom {
                    result = HTBOTTOM;
                    new_edge = 4;
                } else {
                    result = HTCLIENT;
                    new_edge = 0;
                }
            } else if dy >= 0 {
                // 标题栏区域（含顶部边缘缩放）
                let h_edge = scale_for_dpi(8, dpi);
                let on_left = dx >= 0 && dx < h_edge;
                let on_right = dx > win_w - h_edge && dx <= win_w;
                let on_top = dy >= 0 && dy < h_edge;

                if on_top && on_left {
                    result = HTTOPLEFT;
                    new_edge = 1;
                } else if on_top && on_right {
                    result = HTTOPRIGHT;
                    new_edge = 2;
                } else if on_top {
                    result = HTTOP;
                    new_edge = 3;
                } else if on_left {
                    result = HTLEFT;
                    new_edge = 1;
                } else if on_right {
                    result = HTRIGHT;
                    new_edge = 2;
                } else {
                    result = HTCAPTION;
                    new_edge = 0;
                }
            } else {
                result = HTCLIENT;
                new_edge = 0;
            }

            // 更新 hover_edge 并在变化时触发重绘（显示/隐藏边缘指示条）
            if let Some(data) = get_data(hwnd) {
                if data.hover_edge != new_edge {
                    data.hover_edge = new_edge;
                    render_layered_window(hwnd);
                }
            }

            result as LRESULT
        }

        // ===== 光标设置 =====
        WM_SETCURSOR => {
            let hit_test = (lparam & 0xFFFF) as i16 as i32;
            let cursor_id = match hit_test {
                HTLEFT | HTRIGHT => IDC_SIZEWE_ID,
                HTTOP | HTBOTTOM => IDC_SIZENS_ID,
                HTTOPLEFT | HTBOTTOMRIGHT => IDC_SIZENWSE_ID,
                HTTOPRIGHT | HTBOTTOMLEFT => IDC_SIZENESW_ID,
                HTCLIENT => {
                    if let Some(data) = get_data(hwnd) {
                        if data.lock_hovered || data.more_hovered {
                            // 正常模式：按钮上显示手型光标
                            IDC_HAND_ID
                        } else {
                            return DefWindowProcW(hwnd, msg, wparam, lparam);
                        }
                    } else {
                        return DefWindowProcW(hwnd, msg, wparam, lparam);
                    }
                }
                _ => {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
            };
            let cursor = LoadCursorW(std::ptr::null_mut(), cursor_id as *const u16);
            SetCursor(cursor);
            1
        }

        // ===== 窗口缩放 → 重新计算布局并渲染 =====
        WM_SIZE => {
            // Always recalculate layout on resize (wparam == 0 is SIZE_RESTORED = normal resize)
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };
            let (win_w, win_h) = get_grid_content_rect(hwnd)
                .map(|rect| (rect.right - rect.left, rect.bottom - rect.top))
                .unwrap_or((0, 0));

            if win_w > 0 && win_h > 0 {
                if let Some(data) = get_data(hwnd) {
                    // 先重排：把固定位置转回自动布局，保持视觉顺序
                    reflow_items(data);
                    recalculate_layout(data, win_w, win_h, dpi);
                    // Clamp scroll_offset after layout change (content may have reflowed)
                    let view_params = get_view_params(&data.view_mode, dpi);
                    let grid_layout = calculate_grid(win_w, data.title_h, &view_params, dpi);
                    let total_rows =
                        (data.items.len() as i32 + grid_layout.cols - 1) / grid_layout.cols;
                    let total_content_h = total_rows * grid_layout.cell_h;
                    let visible_h = win_h - data.title_h;
                    let max_scroll = (total_content_h - visible_h).max(0);
                    data.scroll_offset = data.scroll_offset.min(max_scroll);
                }
            }
            render_layered_window(hwnd);
            // 尺寸变了 → sink 命中几何快照跟着更新（标题区/按钮矩形随宽高与 DPI 变化）。
            if let Some(data) = get_data(hwnd) {
                if super::glass_engine::has_box(data.config_id) {
                    let (cw, _ch) = get_grid_content_rect(hwnd)
                        .map(|r| (r.right - r.left, r.bottom - r.top))
                        .unwrap_or((0, 0));
                    super::glass_engine::update_block_geom(
                        data.config_id,
                        super::glass_engine::BlockGeom {
                            win_w: cw,
                            title_h: data.title_h,
                            dpi,
                            locked: data.locked,
                        },
                    );
                }
            }
            0
        }

        // ===== DPI 变化（跨显示器移动）=====
        WM_DPICHANGED => {
            let new_dpi = (wparam & 0xFFFF) as u32;
            log::info!("DPI 变化: {} → 重新计算布局", new_dpi);
            let suggested_rect = lparam as *const RECT;
            let outer_w = unsafe { (*suggested_rect).right - (*suggested_rect).left };
            let outer_h = unsafe { (*suggested_rect).bottom - (*suggested_rect).top };
            let padding = shadow_padding_for_dpi(new_dpi);
            let new_w = (outer_w - padding * 2).max(0);
            let new_h = (outer_h - padding * 2).max(0);

            // DPI 变化后留白的物理像素值也变了，必须同步写回窗口，
            // 后面的布局重算与重绘都依赖这个字段。
            if let Some(data) = get_data(hwnd) {
                data.render_padding = padding;
            }
            super::grid_window::update_dcomp_scale(hwnd);

            if let Some(data) = get_data(hwnd) {
                reflow_items(data);
                recalculate_layout(data, new_w, new_h, new_dpi);
            }

            unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    (*suggested_rect).left,
                    (*suggested_rect).top,
                    outer_w,
                    outer_h,
                    SWP_NOZORDER,
                );
            }

            render_layered_window(hwnd);
            0
        }

        // ===== 最小/最大窗口尺寸 =====
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };
            let title_h = get_data(hwnd)
                .map(|d| d.title_h)
                .unwrap_or(scale_for_dpi(40, dpi));
            let padding = window_padding(hwnd, dpi) * 2;
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.pt_min_track_size = POINT {
                x: scale_for_dpi(200, dpi) + padding,
                y: title_h + scale_for_dpi(100, dpi) + padding,
            };
            mmi.pt_max_track_size = POINT {
                x: scale_for_dpi(3000, dpi) + padding,
                y: scale_for_dpi(3000, dpi) + padding,
            };
            0
        }

        // ===== 鼠标移动：hover 效果 + tooltip + 锁定按钮 + 边缘指示 + 拖拽检测 =====
        // ===== 标题拖拽：自定义 SetCapture 移动，不走 Windows 模态循环 =====
        // 模态循环在 125Hz 鼠标消息下会饥饿定时器/渲染（实测快速甩动崩到 3fps）；
        // 自定义移动每个鼠标消息直接 SetWindowPos + 节流渲染，任意速度不掉帧。
        WM_MOUSEMOVE => {
            // 模态移动期间鼠标消息 125Hz 打满模态循环：悬停/阈值/残影
            // 逻辑全部跳过（实测高速甩动时模态循环饥饿、渲染崩到 3fps）。
            if IN_DRAG.get() {
                return 0;
            }
            // 接管式移动会话（SetCapture 后真实 move 到达）：同样跳过悬停逻辑。
            if let Some(data) = get_data(hwnd) {
                if data.box_drag_active {
                    return 0;
                }
            }
            let (mx, my) = content_point_from_lparam(hwnd, lparam);

            // Phase 1: 拖拽阈值检测（超过阈值 → 创建覆盖窗口 + SetCapture）
            if super::drag_handler::check_drag_threshold(hwnd, mx, my) {
                return 0;
            }

            // Phase 2: 拖拽中更新覆盖窗口位置
            if super::drag_handler::update_drag(hwnd) {
                return 0;
            }

            // Marquee selection update: just update coordinates (render happens in timer)
            if let Some(data) = get_data(hwnd) {
                if data.marquee_active {
                    data.marquee_current_x = mx;
                    data.marquee_current_y = my;
                    return 0;
                }
            }

            let mut need_rerender = false;
            let mut show_tooltip = false;
            let mut tooltip_text = String::new();
            let mut tooltip_x: i32 = 0;
            let mut tooltip_y: i32 = 0;

            // 注册 WM_MOUSELEAVE 跟踪 —— **只给回退路径用**，引擎模式必须跳过。
            //
            // 引擎模式下代理窗提交的是全透明帧，真实命中的窗口是库画布，
            // `TrackMouseEvent` 的「光标是否离开本窗」按真实命中链判定 → 我们每次
            // （100ms 轮询）注册都立刻被回一个 `WM_MOUSELEAVE`，把刚设上的 hover
            // 又清掉：实测 hover 图标/锁定/三点高亮以 10Hz 无限抖动，就是用户报的
            // 「背景还在闪」（2026-10-28 逐帧截图 + 日志实锤）。
            // 引擎模式的离开清理由 `poll_glass_hover` 负责（连续 2 拍在块外才补发
            // WM_MOUSELEAVE，天然带去抖）。
            let glass_mode = get_data(hwnd)
                .map(|d| {
                    super::glass_engine::is_active() && super::glass_engine::has_box(d.config_id)
                })
                .unwrap_or(false);
            if !glass_mode {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    TrackMouseEvent(&mut tme);
                }
            }

            if let Some(data) = get_data(hwnd) {
                let Some(wr) = get_grid_content_rect(hwnd) else {
                    return 0;
                };
                let w = wr.right - wr.left;
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };

                // 正常模式：锁定按钮 hover 检测
                let (btn_x, btn_y, btn_size) = lock_btn_rect(w, data.title_h, dpi);
                let over_lock =
                    mx >= btn_x && mx < btn_x + btn_size && my >= btn_y && my < btn_y + btn_size;
                if over_lock != data.lock_hovered {
                    log::info!(
                        "[glass-dbg] hover lock {} -> {} 点=({},{}) 矩形=({},{},{})",
                        data.lock_hovered,
                        over_lock,
                        mx,
                        my,
                        btn_x,
                        btn_y,
                        btn_size
                    );
                    data.lock_hovered = over_lock;
                    need_rerender = true;
                }

                // "更多"按钮 hover 检测
                let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(w, data.title_h, dpi);
                let over_more = mx >= mbtn_x
                    && mx < mbtn_x + mbtn_size
                    && my >= mbtn_y
                    && my < mbtn_y + mbtn_size;
                if over_more != data.more_hovered {
                    log::info!(
                        "[glass-dbg] hover more {} -> {} 点=({},{}) 矩形=({},{},{})",
                        data.more_hovered,
                        over_more,
                        mx,
                        my,
                        mbtn_x,
                        mbtn_y,
                        mbtn_size
                    );
                    data.more_hovered = over_more;
                    need_rerender = true;
                }

                // 边缘 resize hover 已由 WM_NCHITTEST 更新 hover_edge

                // 图标网格 hit test（tooltip + hover 高亮）
                let hit = grid_hit_test(data, w, mx, my, dpi);
                if hit != data.hovered_item {
                    log::info!(
                        "[glass-dbg] hover item {} -> {} 点=({},{}) scroll={} 内容=({},{})",
                        data.hovered_item,
                        hit,
                        mx,
                        my,
                        data.scroll_offset,
                        w,
                        wr.bottom - wr.top
                    );
                    data.hovered_item = hit;
                    need_rerender = true;
                }

                // 仅在分类框有选中项时显示 tooltip
                if hit >= 0
                    && data.selected_item >= 0
                    && (hit as usize) < data.items.len()
                    && !data.tooltip_hwnd.is_null()
                {
                    tooltip_text = data.items[hit as usize].display_name.clone();
                    show_tooltip = true;
                    let mut pt = POINT { x: 0, y: 0 };
                    unsafe { GetCursorPos(&mut pt) };
                    tooltip_x = pt.x + 16;
                    tooltip_y = pt.y + 16;
                }
            }

            if need_rerender {
                render_layered_window(hwnd);
            }

            // 更新 tooltip（在 render 之后，避免 data 借用冲突）
            if let Some(data) = get_data(hwnd) {
                let tt = data.tooltip_hwnd;
                if !tt.is_null() {
                    if show_tooltip {
                        let text_wide: Vec<u16> = tooltip_text.encode_utf16().collect();
                        let mut ti = TOOLINFOW {
                            cbSize: std::mem::size_of::<TOOLINFOW>() as u32,
                            uFlags: TTF_SUBCLASS | TTF_IDISHWND,
                            hwnd,
                            uId: 0,
                            rect: RECT {
                                left: 0,
                                top: 0,
                                right: 0,
                                bottom: 0,
                            },
                            hinst: std::ptr::null_mut(),
                            lpszText: text_wide.as_ptr(),
                            lParam: 0,
                            lpReserved: std::ptr::null_mut(),
                        };
                        unsafe {
                            SendMessageW(
                                tt,
                                TTM_UPDATETIPTEXTW,
                                0,
                                &mut ti as *mut TOOLINFOW as LPARAM,
                            );
                            SendMessageW(
                                tt,
                                TTM_TRACKPOSITION,
                                0,
                                ((tooltip_x as u32 as LPARAM) & 0xFFFF)
                                    | (((tooltip_y as u32 as LPARAM) & 0xFFFF) << 16),
                            );
                            SendMessageW(
                                tt,
                                TTM_TRACKACTIVATE,
                                1,
                                &mut ti as *mut TOOLINFOW as LPARAM,
                            );
                        }
                    } else {
                        let mut ti = TOOLINFOW {
                            cbSize: std::mem::size_of::<TOOLINFOW>() as u32,
                            uFlags: TTF_SUBCLASS | TTF_IDISHWND,
                            hwnd,
                            uId: 0,
                            rect: RECT {
                                left: 0,
                                top: 0,
                                right: 0,
                                bottom: 0,
                            },
                            hinst: std::ptr::null_mut(),
                            lpszText: std::ptr::null(),
                            lParam: 0,
                            lpReserved: std::ptr::null_mut(),
                        };
                        unsafe {
                            SendMessageW(
                                tt,
                                TTM_TRACKACTIVATE,
                                0,
                                &mut ti as *mut TOOLINFOW as LPARAM,
                            );
                        }
                    }
                }
            }
            0
        }

        // ===== 鼠标左键：编辑模式按钮 + 锁定按钮 + "更多"按钮 + 图标选中 =====
        WM_LBUTTONDOWN => {
            let (click_x, click_y) = content_point_from_lparam(hwnd, lparam);
            let mut need_rerender = false;

            // 获取窗口尺寸和 DPI
            let Some(wr) = get_grid_content_rect(hwnd) else {
                return 0;
            };
            let win_w = wr.right - wr.left;
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };

            // 检查是否点击了锁定按钮
            let lock_clicked = if let Some(data) = get_data(hwnd) {
                let (btn_x, btn_y, btn_size) = lock_btn_rect(win_w, data.title_h, dpi);
                click_x >= btn_x
                    && click_x < btn_x + btn_size
                    && click_y >= btn_y
                    && click_y < btn_y + btn_size
            } else {
                false
            };

            // 检查是否点击了"更多"按钮
            let more_clicked = if let Some(data) = get_data(hwnd) {
                let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(win_w, data.title_h, dpi);
                click_x >= mbtn_x
                    && click_x < mbtn_x + mbtn_size
                    && click_y >= mbtn_y
                    && click_y < mbtn_y + mbtn_size
            } else {
                false
            };

            // ===== 引擎路径：标题区（非按钮）按压 =====
            // 拖拽**不在这里起**：库的契约是「sink 返回 Consumed 接管拖拽，
            // Down 记锚点，序列内每次 Move 用 set_rect 驱动」（120Hz，与库会话
            // 同等延迟）。几何推进在 `glass_engine::route_pointer` 里完成，
            // 宿主只通过 `WM_USER_GLASS_DRAG_SYNC` 跟随落位（参考线 + 落盘）。
            // 旧写法（本处调 begin_box_drag + 8ms 定时器轮询光标）节拍被
            // Windows 定时器粒度卡在 ~54Hz，就是用户看到的「移动时图标掉帧」。
            if !lock_clicked && !more_clicked {
                let glass_title = get_data(hwnd).map(|d| {
                    let glass = super::glass_engine::is_active()
                        && super::glass_engine::has_box(d.config_id);
                    (glass && click_y < d.title_h, d.locked)
                });
                if let Some((true, _locked)) = glass_title {
                    return 0;
                }
            }

            if lock_clicked {
                // 切换锁定状态
                if let Some(data) = get_data(hwnd) {
                    data.locked = !data.locked;
                    // 固定（锁定）联动：玻璃块退出生效命中区（库把手不再响应），
                    // 解锁时按当前配置恢复边缘缩放。
                    super::glass_engine::set_box_locked(data.config_id, data.locked);
                    // sink 的标题拖拽判定依赖锁定态，快照同步刷新。
                    if super::glass_engine::has_box(data.config_id) {
                        let (cw, _ch) = get_grid_content_rect(hwnd)
                            .map(|r| (r.right - r.left, r.bottom - r.top))
                            .unwrap_or((0, 0));
                        let dpi_now = unsafe { GetDpiForWindow(hwnd) };
                        let dpi_now = if dpi_now > 0 { dpi_now } else { 96 };
                        super::glass_engine::update_block_geom(
                            data.config_id,
                            super::glass_engine::BlockGeom {
                                win_w: cw,
                                title_h: data.title_h,
                                dpi: dpi_now,
                                locked: data.locked,
                            },
                        );
                    }
                    log::info!(
                        "锁定状态切换: {} → {}",
                        if data.locked {
                            "已锁定"
                        } else {
                            "已解锁"
                        },
                        data.title
                    );
                    need_rerender = true;
                }
            } else if more_clicked {
                // 显示自定义分层窗口菜单（阻塞直到用户选择或取消）
                show_menu_window(hwnd);
            } else if let Some(data) = get_data(hwnd) {
                let hit = grid_hit_test(data, win_w, click_x, click_y, dpi);
                let ctrl_down = unsafe { GetAsyncKeyState(VK_CONTROL) } & 0x8000u16 as i16 != 0;
                let shift_down = unsafe { GetAsyncKeyState(VK_SHIFT) } & 0x8000u16 as i16 != 0;
                log::info!(
                    "[CLICK] pos=({},{}), hit={}, ctrl={}, shift={}, selected={}, anchor={}",
                    click_x,
                    click_y,
                    hit,
                    ctrl_down,
                    shift_down,
                    data.selected_item,
                    data.shift_anchor
                );
                let ct = super::multi_select::ContainerType::GridBox;
                let cid = super::multi_select::container_id_from_uuid(data.config_id);
                if hit >= 0 {
                    let hi = hit as usize;
                    let key = super::multi_select::SelectionKey {
                        container_type: ct,
                        container_id: cid,
                        item_index: hi,
                    };
                    if shift_down && data.shift_anchor >= 0 {
                        let a = data.shift_anchor as usize;
                        // CRITICAL: Compute visual order (row, col) for each item, then
                        // select all items between anchor and target in VISUAL order.
                        // This matches what the user sees on screen, not the scrambled
                        // array index order caused by drag-and-drop reordering.
                        let shift_vp = get_view_params(&data.view_mode, dpi);
                        let shift_layout = calculate_grid(win_w, data.title_h, &shift_vp, dpi);
                        let cols = shift_layout.cols;

                        // Build (item_index, visual_row, visual_col) for every item
                        let mut visual_order: Vec<(usize, i32, i32)> =
                            Vec::with_capacity(data.items.len());
                        for (i, item) in data.items.iter().enumerate() {
                            if item.grid_col >= 0 && item.grid_row >= 0 {
                                visual_order.push((i, item.grid_row, item.grid_col));
                            } else {
                                let idx = i as i32;
                                visual_order.push((i, idx / cols, idx % cols));
                            }
                        }
                        // Sort by (row, col) to get left-to-right, top-to-bottom visual order
                        visual_order.sort_by(|x, y| x.1.cmp(&y.1).then(x.2.cmp(&y.2)));

                        log::debug!(
                            "[SHIFT-CLICK] anchor={}, target={}, cols={}, visual_order:",
                            a,
                            hi,
                            cols
                        );
                        for (vo_idx, &(item_idx, r, c)) in visual_order.iter().enumerate() {
                            log::debug!(
                                "[SHIFT-CLICK]   visual[{}] = item[{}] at ({},{})",
                                vo_idx,
                                item_idx,
                                r,
                                c
                            );
                        }

                        // Find anchor and target positions in the visual order
                        let anchor_vo = visual_order.iter().position(|&(idx, _, _)| idx == a);
                        let target_vo = visual_order.iter().position(|&(idx, _, _)| idx == hi);

                        if let (Some(av), Some(tv)) = (anchor_vo, target_vo) {
                            let lo_vo = av.min(tv);
                            let hi_vo = av.max(tv);
                            log::debug!(
                                "[SHIFT-CLICK] visual range: visual[{}..={}]",
                                lo_vo,
                                hi_vo
                            );

                            if !ctrl_down {
                                super::multi_select::clear();
                            }
                            for vo_idx in lo_vo..=hi_vo {
                                let item_idx = visual_order[vo_idx].0;
                                let k = super::multi_select::SelectionKey {
                                    container_type: ct,
                                    container_id: cid,
                                    item_index: item_idx,
                                };
                                super::multi_select::add(k);
                            }
                        } else {
                            log::debug!(
                                "[SHIFT-CLICK] anchor or target not found in visual_order!"
                            );
                        }

                        // Ensure clicked item is in selection
                        super::multi_select::add(key);
                        let selected_now = super::multi_select::selected_in_container(ct, cid);
                        log::debug!(
                            "[SHIFT-RESULT] Final selection: {} items = {:?}",
                            selected_now.len(),
                            selected_now
                        );
                        data.selected_item = hit;
                    } else if ctrl_down {
                        super::multi_select::toggle(key);
                        if super::multi_select::is_selected(key) {
                            data.shift_anchor = hit;
                            data.selected_item = hit;
                        } else if data.selected_item == hit {
                            data.selected_item = -1;
                        }
                    } else {
                        // ★ 标准 Windows 行为：点击已选中的项时保持多选（等释放后再清除）
                        // 只有点击未选中的项时才清除多选
                        if !super::multi_select::is_selected(key) {
                            super::multi_select::clear();
                            super::multi_select::add(key);
                            super::multi_select::set_anchor(Some(key));
                        }
                        data.shift_anchor = hit;
                        data.selected_item = hit;
                    }
                    need_rerender = true;
                    if !data.locked {
                        data.drag_pending = true;
                        data.drag_start_x = click_x;
                        data.drag_start_y = click_y;
                        // If clicking an already-selected item, prepare multi-drag
                        if super::multi_select::is_selected(key) {
                            data.multi_drag_items =
                                super::multi_select::selected_in_container(ct, cid);
                            log::info!(
                                "[GRID-CLICK] multi_drag_items={:?} ({} items)",
                                data.multi_drag_items,
                                data.multi_drag_items.len()
                            );
                        } else {
                            data.multi_drag_items.clear();
                        }
                    }
                } else {
                    // Blank area click: only clear if Ctrl not held
                    if !ctrl_down {
                        super::multi_select::clear();
                        data.shift_anchor = -1;
                    }
                    if data.selected_item >= 0 {
                        data.selected_item = -1;
                        need_rerender = true;
                    }
                    // Kill any existing hover timer to prevent interference
                    unsafe {
                        KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
                    }
                    // Start marquee (SetTimer for tracking mouse release outside window)
                    data.marquee_active = true;
                    data.marquee_start_x = click_x;
                    data.marquee_start_y = click_y;
                    data.marquee_current_x = click_x;
                    data.marquee_current_y = click_y;
                    unsafe {
                        SetTimer(hwnd, TIMER_ID_GRID_DRAG, 16, std::ptr::null_mut());
                    }
                }
            }

            if need_rerender {
                render_layered_window(hwnd);
            }
            0
        }

        // ===== 鼠标左键双击：标题栏进入名称编辑 / 图标打开文件 =====
        WM_LBUTTONDBLCLK => {
            let (click_x, click_y) = content_point_from_lparam(hwnd, lparam);

            if let Some(data) = get_data(hwnd) {
                let Some(wr) = get_grid_content_rect(hwnd) else {
                    return 0;
                };
                let w = wr.right - wr.left;
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };

                // 检查是否双击了标题栏文字区域（非按钮区域）
                if click_y < data.title_h && !data.menu.open {
                    let (btn_x, btn_y, btn_size) = lock_btn_rect(w, data.title_h, dpi);
                    let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(w, data.title_h, dpi);
                    let on_lock = click_x >= btn_x
                        && click_x < btn_x + btn_size
                        && click_y >= btn_y
                        && click_y < btn_y + btn_size;
                    let on_more = click_x >= mbtn_x
                        && click_x < mbtn_x + mbtn_size
                        && click_y >= mbtn_y
                        && click_y < mbtn_y + mbtn_size;

                    if !on_lock && !on_more {
                        // 调用编辑分类框回调函数
                        if let Some(ref callback) = data.edit_box_callback {
                            let config_id = data.config_id;
                            let title = data.title.clone();
                            callback(config_id, &title);
                            log::info!(
                                "触发编辑分类框对话框: config_id={}, title={}",
                                config_id,
                                title
                            );
                        } else {
                            log::warn!("edit_box_callback 未设置，无法打开编辑对话框");
                        }
                        return 0;
                    }
                }

                // 双击图标打开文件
                let idx = grid_hit_test(data, w, click_x, click_y, dpi);

                if idx >= 0 && (idx as usize) < data.items.len() {
                    let item = &data.items[idx as usize];
                    let op: Vec<u16> = "open\0".encode_utf16().collect();
                    let open_path = super::icon_hider::resolve_icon_read_path(&item.file_path)
                        .map(|path| path.to_string_lossy().to_string())
                        .unwrap_or_else(|| item.file_path.clone());
                    let path: Vec<u16> =
                        open_path.encode_utf16().chain(std::iter::once(0)).collect();
                    ShellExecuteW(
                        hwnd,
                        op.as_ptr(),
                        path.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        SW_SHOW,
                    );
                    log::info!("双击打开: {}", item.display_name);
                }
            }
            0
        }

        // ===== 非客户区双击：标题栏进入名称编辑（未锁定时 HTCAPTION 区域双击）=====
        WM_NCLBUTTONDBLCLK => {
            if wparam as i32 == HTCAPTION {
                // lparam 为屏幕坐标，转换为窗口坐标
                let screen_x = (lparam & 0xFFFF) as i16 as i32;
                let screen_y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

                if let Some(data) = get_data(hwnd) {
                    let Some(wr) = get_grid_content_rect(hwnd) else {
                        return 0;
                    };
                    let w = wr.right - wr.left;
                    let click_x = screen_x - wr.left;
                    let click_y = screen_y - wr.top;
                    let dpi = unsafe { GetDpiForWindow(hwnd) };
                    let dpi = if dpi > 0 { dpi } else { 96 };

                    // 排除锁定/更多按钮区域（理论上不会收到 HTCAPTION，双重保险）
                    let (btn_x, btn_y, btn_size) = lock_btn_rect(w, data.title_h, dpi);
                    let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(w, data.title_h, dpi);
                    let on_lock = click_x >= btn_x
                        && click_x < btn_x + btn_size
                        && click_y >= btn_y
                        && click_y < btn_y + btn_size;
                    let on_more = click_x >= mbtn_x
                        && click_x < mbtn_x + mbtn_size
                        && click_y >= mbtn_y
                        && click_y < mbtn_y + mbtn_size;

                    if !on_lock && !on_more && !data.menu.open {
                        // 调用编辑分类框回调函数（与 WM_LBUTTONDBLCLK 一致）
                        if let Some(ref callback) = data.edit_box_callback {
                            let config_id = data.config_id;
                            let title = data.title.clone();
                            callback(config_id, &title);
                            log::info!(
                                "WM_NCLBUTTONDBLCLK 触发编辑分类框对话框: config_id={}, title={}",
                                config_id,
                                title
                            );
                        } else {
                            log::warn!("edit_box_callback 未设置，无法打开编辑对话框");
                        }
                        return 0;
                    }
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        // ===== 失去焦点：取消名称编辑 =====
        WM_KILLFOCUS => 0,

        // ===== 鼠标右键：弹出上下文菜单（支持多选） =====
        WM_RBUTTONUP => {
            let (click_x, click_y) = content_point_from_lparam(hwnd, lparam);

            // 检查多选状态
            let multi_count = super::multi_select::count();

            let mut file_path: Option<String> = None;
            let mut hit_index: i32 = -1;
            if let Some(data) = get_data(hwnd) {
                let Some(wr) = get_grid_content_rect(hwnd) else {
                    return 0;
                };
                let w = wr.right - wr.left;
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let hit = grid_hit_test(data, w, click_x, click_y, dpi);
                if hit >= 0 {
                    // 多选右键：检查点击的是否是已选中的项
                    if multi_count > 1 {
                        let ct = super::multi_select::ContainerType::GridBox;
                        let cid = super::multi_select::container_id_from_uuid(data.config_id);
                        let key = super::multi_select::SelectionKey {
                            container_type: ct,
                            container_id: cid,
                            item_index: hit as usize,
                        };

                        if super::multi_select::is_selected(key) {
                            // 多选右键：收集所有选中项的路径
                            let selected_paths = super::grid_window::collect_selected_file_paths();
                            if !selected_paths.is_empty() {
                                log::info!(
                                    "[多选菜单] 右键点击已选中项 {}，多选菜单: {} 个文件",
                                    hit,
                                    selected_paths.len()
                                );
                                let response =
                                    show_multi_select_shell_context_menu(hwnd, &selected_paths);
                                handle_multi_shell_menu_result(hwnd, response);
                                return 0;
                            }
                        }
                    }

                    // 单选右键：保持现有逻辑
                    data.selected_item = hit;
                    if (hit as usize) < data.items.len() {
                        file_path = Some(data.items[hit as usize].file_path.clone());
                        hit_index = hit;
                    }
                    render_layered_window(hwnd);
                }
            }

            if let Some(ref path) = file_path {
                let response = show_shell_context_menu(hwnd, path);
                handle_shell_menu_result(
                    hwnd,
                    path,
                    hit_index,
                    response.result,
                    response.staging_path,
                );
            } else {
                log::info!("[MENU-DEBUG] wnd_proc: 右键点击框空白区域, 调用 show_desktop_context_menu(include_ikun_items=true)");
                show_desktop_context_menu(hwnd, true); // 显式注入 Ikun 菜单项
                                                       // shell_menu.rs 会在用户点击「创建分类框」时设置 PENDING_CREATE_BOX 标志，
                                                       // 平台轮询线程消费该标志并打开创建分类框窗口。
            }
            0
        }

        // ===== cut file moved (pasted elsewhere) =====
        WM_USER_CUT_FILE_MOVED => {
            if let Some(data) = get_data(hwnd) {
                let mut removed_any = false;
                let mut i = 0;
                while i < data.items.len() {
                    if !super::icon_hider::icon_exists_for_box(&data.items[i].file_path) {
                        let removed = data.items.remove(i);
                        if !removed.icon_handle.is_null() {
                            super::icon_extract::destroy_icon(removed.icon_handle);
                        }
                        log::info!("cut file moved, removed from box: {}", removed.display_name);
                        removed_any = true;
                    } else {
                        i += 1;
                    }
                }
                if removed_any {
                    if data.selected_item >= data.items.len() as i32 {
                        data.selected_item = -1;
                    }
                    if data.hovered_item >= data.items.len() as i32 {
                        data.hovered_item = -1;
                    }
                    render_layered_window(hwnd);
                    super::drag_handler::save_config();
                }
            }
            0
        }

        // ===== 鼠标滚轮滚动 =====
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) as i16) as i32; // HIWORD(wParam)
            let scroll_amount = 3; // lines per notch
            if let Some(data) = get_data(hwnd) {
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let Some(wr) = get_grid_content_rect(hwnd) else {
                    return 0;
                };
                let window_w = wr.right - wr.left;
                let view_params = get_view_params(&data.view_mode, dpi);
                let grid_layout = calculate_grid(window_w, data.title_h, &view_params, dpi);
                let cell_h = grid_layout.cell_h;
                let cols = grid_layout.cols;
                let total_rows = (data.items.len() as i32 + cols - 1) / cols;
                let total_content_h = total_rows * cell_h;
                let window_h = wr.bottom - wr.top;
                let visible_h = window_h - data.title_h;
                let max_scroll = (total_content_h - visible_h).max(0);
                let old_offset = data.scroll_offset;
                data.scroll_offset =
                    (data.scroll_offset - delta * scroll_amount).clamp(0, max_scroll);
                if data.scroll_offset != old_offset {
                    render_layered_window(hwnd);
                }
            }
            0
        }

        // ===== 鼠标离开窗口：重置所有 hover 状态 =====
        WM_MOUSELEAVE => {
            // 取证日志：hover 闪烁 = 状态在 move/leave 之间来回抖。
            // 2026-10-28 定位过程：修复前当天 2770 条 LEAVE（TrackMouseEvent 误判），
            // 修复后只剩轮询补发的正常 leave。已降回 debug 级。
            log::debug!("[glass-dbg] WM_MOUSELEAVE 到达（hover 清空）");
            let mut need_rerender = false;
            if let Some(data) = get_data(hwnd) {
                if data.hovered_item != -1 {
                    data.hovered_item = -1;
                    need_rerender = true;
                }
                if data.lock_hovered {
                    data.lock_hovered = false;
                    need_rerender = true;
                }
                if data.more_hovered {
                    data.more_hovered = false;
                    need_rerender = true;
                }
                if data.hover_edge != 0 {
                    data.hover_edge = 0;
                    need_rerender = true;
                }
            }
            if need_rerender {
                render_layered_window(hwnd);
            }
            0
        }

        // ===== 窗口销毁 =====
        // 处理重命名结果消息
        msg if msg == super::rename_editor::get_rename_result_message() => {
            let item_index = wparam as i32;
            let text_ptr = lparam as *mut String;

            if !text_ptr.is_null() && item_index >= 0 {
                let new_name = unsafe { Box::from_raw(text_ptr) };
                log::info!("Rename result for item {}: '{}'", item_index, *new_name);

                if let Some(data) = unsafe { get_data(hwnd) } {
                    // 清除重命名状态
                    data.renaming_item_index = -1;
                    if (item_index as usize) < data.items.len() {
                        let old_path = data.items[item_index as usize].file_path.clone();
                        let old_name = data.items[item_index as usize].display_name.clone();

                        // Use shared helper to build filename with preserved extension
                        let final_name =
                            super::rename_editor::build_rename_filename(&new_name, &old_path);

                        if !final_name.is_empty() && final_name != old_name {
                            match super::icon_hider::rename_icon_file(&old_path, &final_name) {
                                Ok(new_path) => {
                                    log::info!("Renamed: {} -> {}", old_path, new_path);

                                    // 更新数据
                                    if let Some(item) = data.items.get_mut(item_index as usize) {
                                        item.file_path = new_path;
                                        item.display_name =
                                            super::rename_editor::display_name_from_path(
                                                &item.file_path,
                                            );
                                    }

                                    // 保存配置并重新渲染
                                    super::drag_handler::save_config();
                                    render_layered_window(hwnd);
                                }
                                Err(e) => {
                                    log::error!("Failed to rename: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            0
        }
        msg if msg == super::rename_editor::get_rename_cancel_message() => {
            log::info!("Rename cancelled for item {}", wparam);
            if let Some(data) = unsafe { get_data(hwnd) } {
                data.renaming_item_index = -1;
                render_layered_window(hwnd);
            }
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_ID_LIQUID_GLASS);
            // 接管式移动会话未收尾即销毁时释放捕获（保存由整体退出路径负责）。
            if let Some(data) = get_data(hwnd) {
                if data.box_drag_active {
                    unsafe {
                        ReleaseCapture();
                    }
                }
            }
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let mut data = Box::from_raw(data_ptr);
                // 解除玻璃底板（引擎未激活/无块时安全 no-op）
                super::glass_engine::detach_box(data.config_id);

                // 注销 OLE 拖拽目标
                if !data.drop_target_ptr.is_null() {
                    super::drop_target::unregister_box_drop_target(hwnd, data.drop_target_ptr);
                }

                // 释放图标句柄
                for item in &data.items {
                    if !item.icon_handle.is_null() {
                        super::icon_extract::destroy_icon(item.icon_handle);
                    }
                }

                // 复用的 DIB 渲染后端必须显式释放，
                // 否则每个销毁的分类框都会漏掉一块 DIB 与一个内存 DC。
                super::d2d_render::release_grid_dib(&mut data);
                // DComp 视觉兑底摘树：正常路径在 DestroyWindow 之前已摘（此时
                // dcomp 为 None）；任何绕过 teardown 的销毁路径在这里补摘，
                // 否则旧视觉冻结在合成树上变成永久残影。
                if let Some(state) = data.dcomp.take() {
                    super::grid_window::teardown_dcomp_state(*state);
                }
                // DC 渲染目标通过 Drop 释放（windows crate COM 包装器）；
                // D2D / DirectWrite / WIC 工厂是渲染线程共享的，不能在这里 drop。
                drop(data.d2d_target);

                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                log::info!("格子窗口 D2D/DW 资源已释放");
            }
            0
        }

        // ===== 默认处理 =====
        // ===== 异步重绘请求（样式配置变更后由 Tauri 线程 PostMessage 触发）=====
        WM_USER_GRID_REDRAW => {
            render_layered_window(hwnd);
            0
        }

        // ===== 库 FocusHint 转发：同线程 SetFocus（代理窗全透明但可见，合法可聚焦）=====
        WM_USER_GLASS_FOCUS => {
            // 例外：这次按压已经把焦点交给了模态 UI（三点菜单 / 图标就地重命名）。
            // 菜单靠 WM_ACTIVATE(WA_INACTIVE) 自杀，代理窗抢回焦点会让菜单
            // 「刚弹出就关掉」，用户侧表现为三点按钮点了没反应
            // （2026-10-28 日志实锤：Down → 菜单建 D2D 目标 → 立刻无下文）。
            let modal_ui = get_data(hwnd)
                .map(|d| d.menu.open || d.renaming_item_index >= 0)
                .unwrap_or(false);
            if modal_ui {
                log::debug!("忽略库 FocusHint：模态菜单/重命名编辑中，焦点不得抢回代理窗");
                return 0;
            }
            unsafe {
                SetFocus(hwnd);
            }
            0
        }

        // ===== 引擎缩放会话开始/结束（渲染线程事件回调转发）=====
        WM_USER_GLASS_RESIZE => {
            if wparam != 0 {
                super::grid_window::begin_glass_resize_session(hwnd);
            } else {
                super::grid_window::end_glass_resize_session(hwnd);
            }
            0
        }

        // ===== 位置/尺寸变化完成：把窗口几何反向同步给玻璃块 =====
        //
        // 只处理「非库会话」的变化（原生标题拖拽由 WM_MOVING 同步、库会话期间
        // 块矩形是真值不得回推），覆盖程序化移动：DPI 切换、菜单位置、折叠等。
        WM_WINDOWPOSCHANGED => {
            if let Some(data) = get_data(hwnd) {
                // 库会话（移动/缩放）期间块矩形是真值，绝不能用窗口几何反推。
                // 除了本地标志，还要**直接问库**：会话起止是 PostMessage 异步送达的，
                // 标志位还没置上的那几拍里反推一次，就会把库的会话锚点搞脏
                // （实测：光标 (+220,+130) 拖出 (+315,+32) 的落位）。
                if !data.glass_session_active
                    && !data.box_drag_active
                    && super::glass_engine::is_active()
                    && super::glass_engine::has_box(data.config_id)
                    && !super::glass_engine::block_in_session(data.config_id)
                {
                    let mut wr = RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    };
                    if unsafe { GetWindowRect(hwnd, &mut wr) } != 0 {
                        let dpi = unsafe { GetDpiForWindow(hwnd) };
                        let dpi = if dpi > 0 { dpi } else { 96 };
                        super::glass_engine::push_geometry_for_window(
                            data.config_id,
                            &wr,
                            data.render_padding,
                            dpi,
                        );
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        // ===== 延迟打开菜单（由 PostMessage 触发，用于菜单互斥切换）=====
        WM_USER_DESTROY_GUIDE => {
            // 延迟销毁全屏参考线 overlay：释放全屏分层窗与 DIB（约 50MB+）。
            super::alignment_guide::destroy_guide_overlay();
            0
        }
        WM_USER_OPEN_MENU => {
            show_menu_window(hwnd);
            0
        }

        // ===== 延迟删除分类框（由菜单 PostMessage 触发，在消息循环正常调度时处理）=====
        WM_USER_DELETE_BOX => {
            // 先读取 config_id 和 title（窗口销毁前）
            let (config_id, title) = get_data(hwnd)
                .map(|d| (d.config_id, d.title.clone()))
                .unwrap_or((uuid::Uuid::nil(), String::new()));
            log::info!("删除分类框: '{}' (id={})", title, config_id);

            // 销毁窗口（从全局列表移除 + DestroyWindow）
            super::grid_window::destroy_grid_window(hwnd);

            // 按 UUID 精确删除配置中的一条记录
            match crate::config::AppConfig::load() {
                Ok(mut config) => {
                    let before = config.layout.boxes.len();
                    config.layout.boxes.retain(|b| b.id != config_id);
                    let after = config.layout.boxes.len();
                    if let Err(e) = config.save() {
                        log::warn!("删除分类框后保存配置失败: {}", e);
                    } else {
                        log::info!("已删除分类框 '{}'，配置: {} → {} 个", title, before, after);
                    }
                }
                Err(e) => {
                    log::warn!("加载配置失败，跳过配置更新: {}", e);
                }
            }

            // 刷新桌面覆盖窗口：删除分类框后，其包含的图标变为“非管理”状态，
            // 需要重新收集并显示在覆盖窗口上（纵向优先布局）
            // 还原仓库中不再被任何剩余分类框管理的图标到桌面
            super::icon_hider::restore_unmanaged_vault_icons();
            // 修复同模板分层窗口 Z-order 异常（DestroyWindow 副作用）
            super::grid_window::rerender_all_grid_windows();

            0
        }

        // ===== 鼠标左键释放：完成自定义拖拽 / 取消拖拽检测 =====
        WM_LBUTTONUP => {
            if let Some(data) = get_data(hwnd) {
                // 接管式移动会话收尾（真实捕获 up 或 posted up 均到达此处）。
                if data.box_drag_active {
                    super::grid_window::end_box_drag(hwnd);
                    return 0;
                }
                if data.marquee_active {
                    let ctrl_down = unsafe { GetAsyncKeyState(VK_CONTROL) } & 0x8000u16 as i16 != 0;
                    data.marquee_active = false;
                    let rl = data.marquee_start_x.min(data.marquee_current_x);
                    let rr = data.marquee_start_x.max(data.marquee_current_x);
                    let rt = data.marquee_start_y.min(data.marquee_current_y);
                    let rb = data.marquee_start_y.max(data.marquee_current_y);
                    let dx = (data.marquee_current_x - data.marquee_start_x).abs();
                    let dy = (data.marquee_current_y - data.marquee_start_y).abs();
                    if dx > 3 || dy > 3 {
                        let Some(wr2) = get_grid_content_rect(hwnd) else {
                            return 0;
                        };
                        let ww2 = wr2.right - wr2.left;
                        let d2 = unsafe { GetDpiForWindow(hwnd) };
                        let d2 = if d2 > 0 { d2 } else { 96 };
                        let ct = super::multi_select::ContainerType::GridBox;
                        let cid = super::multi_select::container_id_from_uuid(data.config_id);
                        if !ctrl_down {
                            super::multi_select::clear();
                        }
                        let vp = get_view_params(&data.view_mode, d2);
                        let lo = calculate_grid(ww2, data.title_h, &vp, d2);
                        for (i, item) in data.items.iter().enumerate() {
                            let (cx, cy) = if item.grid_col >= 0 && item.grid_row >= 0 {
                                (
                                    lo.grid_left + item.grid_col * lo.cell_w,
                                    lo.grid_top + item.grid_row * lo.cell_h - data.scroll_offset,
                                )
                            } else {
                                let c = (i as i32) % lo.cols;
                                let r = (i as i32) / lo.cols;
                                (
                                    lo.grid_left + c * lo.cell_w,
                                    lo.grid_top + r * lo.cell_h - data.scroll_offset,
                                )
                            };
                            let ccx = cx + lo.cell_w / 2;
                            let ccy = cy + lo.cell_h / 2;
                            if ccx >= rl && ccx <= rr && ccy >= rt && ccy <= rb {
                                super::multi_select::add(super::multi_select::SelectionKey {
                                    container_type: ct,
                                    container_id: cid,
                                    item_index: i,
                                });
                            }
                        }
                        let sel = super::multi_select::selected_in_container(ct, cid);
                        if let Some(&first) = sel.first() {
                            data.shift_anchor = first as i32;
                            data.selected_item = first as i32;
                        } else {
                            data.shift_anchor = -1;
                            data.selected_item = -1;
                        }
                        log::info!("Grid marquee: {} items selected", sel.len());
                    }
                    render_layered_window(hwnd);
                    return 0;
                }
                data.drag_pending = false;
            }
            0
        }

        // ===== 拖拽轮询定时器（替代 SetCapture）=====
        WM_TIMER => {
            if wparam == TIMER_ID_GRID_DRAG {
                super::drag_handler::handle_grid_drag_timer(hwnd);
            }
            if wparam == TIMER_ID_LIQUID_GLASS {
                // 库缩放会话期间的跟随 tick（会话外该定时器不会装配）
                super::grid_window::on_grid_refresh_tick(hwnd);
            }
            0
        }
        // ===== 捕获丢失处理：纯清理兆底 =====
        WM_CAPTURECHANGED => 0,

        // ===== 拖拽悬停进入：显示高亮 =====
        WM_USER_DRAG_ENTER => {
            super::drag_handler::handle_drag_enter(hwnd);
            0
        }

        // ===== 跨框拖拽覆盖可视化更新 =====
        WM_USER_DRAG_OVER_UPDATE => {
            let screen_x = (lparam as i32) & 0xFFFF;
            let screen_y = ((lparam as i32) >> 16) & 0xFFFF;
            super::drag_handler::handle_cross_box_drag_over(hwnd, screen_x, screen_y);
            0
        }

        // ===== 拖拽悬停离开：清除高亮 =====
        WM_USER_DRAG_LEAVE => {
            super::drag_handler::handle_drag_leave(hwnd);
            0
        }

        // ===== 框→框移动：目标框添加项（由源框 PostMessage 触发）=====
        WM_USER_DRAG_ITEM_ADDED => {
            super::drag_handler::handle_drag_item_added(hwnd);
            0
        }

        // ===== OLE 拖拽放置：桌面→框 或 框→框（由 IDropTarget PostMessage 触发）=====
        WM_USER_DRAG_DROP => {
            super::drag_handler::handle_ole_drop(hwnd);
            0
        }

        // ===== 异步定位图标到桌面指定位置（避免在 OLE 上下文中调用 Shell 通知）=====
        WM_USER_DEFERRED_POSITION_ICON => {
            let info_ptr = lparam as *mut super::ffi_types::DeferredPositionInfo;
            if !info_ptr.is_null() {
                unsafe {
                    let info = Box::from_raw(info_ptr);
                    log::info!(
                        "[DEFERRED] 处理异步定位: file={}, pos=({}, {})",
                        info.file_path,
                        info.screen_x,
                        info.screen_y
                    );

                    // 调用 position_icon_at_screen 定位图标
                    super::icon_manager::position_icon_at_screen(
                        &info.file_path,
                        info.screen_x,
                        info.screen_y,
                    );
                }
            }
            0
        }

        // ===== 异步文件收入仓库（避免阻塞 OLE 回调导致鼠标释放慢）=====
        WM_USER_DEFERRED_INGEST => {
            super::drag_drop::process_deferred_ingests();
            0
        }

        // ===== 源框通知：项已被拖出（OLE Drop Target 触发，保留兼容）=====
        WM_USER_DRAG_ITEM_REMOVED => {
            // 自定义拖拽模式下，框→框移动在 end_drag 中直接处理
            // 此处理器保留给 OLE Drop Target 的外部拖拽场景
            if let Some(removal) = super::drop_target::take_pending_source_removal() {
                if let Some(data) = get_data(hwnd) {
                    let idx = removal.item_index;
                    if idx < data.items.len() {
                        let removed = data.items.remove(idx);
                        log::info!("OLE 拖拽移除: '{}'", removed.display_name);
                        if data.selected_item >= data.items.len() as i32 {
                            data.selected_item = -1;
                        }
                        render_layered_window(hwnd);
                    }
                }
            }
            0
        }

        // ===== Z-order 管理：确保分类框不高于其他应用窗口 =====
        //
        // 当 Windows 试图改变分类框的 Z-order 时（例如点击激活），
        // 将分类框强制放到 Z-order 底部（HWND_BOTTOM），然后通过延迟消息
        // 将其他分类框也推到底部，使被激活的分类框在分类框组中保持最前。
        WM_WINDOWPOSCHANGING => {
            // 检测 Windows 是否 clamp 了窗口位置（将负坐标强制改为 0）
            // 只在拖拽移动时应用，边缘缩放时不应用
            if IN_DRAG.get() && IN_MOVE.get() {
                let wp_ptr = lparam as *mut WINDOWPOS;
                if !wp_ptr.is_null() {
                    let wp = unsafe { &mut *wp_ptr };
                    if wp.flags & SWP_NOMOVE == 0 {
                        let expected_x = PROPOSED_X.get();
                        let expected_y = PROPOSED_Y.get();
                        if wp.x != expected_x || wp.y != expected_y {
                            wp.x = expected_x;
                            wp.y = expected_y;
                        }
                    }
                }
            }
            // 拖拽进行中不干预 Z-order，否则 OLE hit-testing 无法检测到目标窗口
            if !is_zorder_suppressing() && !is_drag_in_progress() {
                let windowpos = lparam as *mut WINDOWPOS;
                if !windowpos.is_null() {
                    let wp = unsafe { &mut *windowpos };
                    // 仅当 Z-order 正在被改变时干预（resize/move 不涉及 Z-order）
                    if wp.flags & SWP_NOZORDER == 0 {
                        // 将本窗口放到 Z-order 底部（高于 overlay，低于其他应用）
                        wp.hwnd_insert_after = HWND_BOTTOM;
                        // 延迟推送：等当前 Z-order 变化生效后，再把其他分类框推到本窗口之下
                        unsafe {
                            PostMessageW(hwnd, WM_USER_PUSH_GRIDS_BELOW, 0, 0);
                        }
                    }
                }
            }
            0
        }

        // ===== 延迟 Z-order 整理：将其他分类框推到当前窗口之下 =====
        WM_USER_PUSH_GRIDS_BELOW => {
            // 在抑制模式下调整其他分类框 Z-order，防止递归 PostMessage
            with_zorder_suppressed(|| push_other_grids_below(hwnd));
            // 玻璃层同步 Z 序不变量：本框块抬到画布内最前，画布整体重新压回
            // Z 序底部（内容窗口刚被强制 HWND_BOTTOM，若画布比它更底就会
            // 把图标盖住——见 glass_engine 模块头「Z 序不变量」）。
            if let Some(data) = get_data(hwnd) {
                super::glass_engine::raise_box(data.config_id);
            }
            super::glass_engine::reassert_canvas_bottom();
            0
        }
        // ===== Enter move/resize mode: start alignment detection =====
        WM_ENTERSIZEMOVE => {
            IN_DRAG.set(true);
            DRAG_HWND.with(|c| c.set(hwnd));
            super::alignment_guide::on_enter_size_move(hwnd);
            // 引擎架构下这里不再切换宿主重绘档位：玻璃动画由库渲染线程
            // 全权驱动；标题栏拖拽只负责移动窗口 + WM_MOVING 同步块矩形，
            // 内容层本身是静态前景，无需周期重绘。
            0
        }

        // ===== Exit move/resize mode: clear guide lines =====
        WM_EXITSIZEMOVE => {
            IN_DRAG.set(false);
            DRAG_HWND.with(|c| c.set(std::ptr::null_mut()));
            IN_MOVE.set(false); // 重置移动标记
            super::alignment_guide::on_exit_size_move(hwnd);
            // Save layout after move/resize
            super::drag_handler::save_config();
            0
        }

        // ===== Moving: detect alignment and show guide lines =====
        WM_MOVING => {
            IN_MOVE.set(true); // 标记这是拖拽移动
            let proposed_rect = lparam as *mut RECT;
            if !proposed_rect.is_null() {
                let rect = unsafe { &mut *proposed_rect };
                // 保存用户期望的位置
                PROPOSED_X.set(rect.left);
                PROPOSED_Y.set(rect.top);
                super::alignment_guide::on_moving(hwnd, rect);
                // 玻璃块位置同步：引擎激活时把移动后的内容矩形交给库
                // （命令队列去重下发，渲染线程 ≤1 帧内贴上；玻璃动画由库线程
                // 独立驱动，内容层此刻是静态前景，不需要宿主同步重绘）。
                if let Some(data) = get_data(hwnd) {
                    if super::glass_engine::is_active()
                        && super::glass_engine::has_box(data.config_id)
                    {
                        let padding = data.render_padding;
                        let content = RECT {
                            left: rect.left + padding,
                            top: rect.top + padding,
                            right: rect.right - padding,
                            bottom: rect.bottom - padding,
                        };
                        let dpi = unsafe { GetDpiForWindow(hwnd) };
                        let dpi = if dpi > 0 { dpi } else { 96 };
                        super::glass_engine::push_box_rect(data.config_id, &content, dpi);
                    }
                }
            }
            0
        }

        // ===== Sizing: detect alignment and show guide lines =====
        WM_SIZING => {
            let proposed_rect = lparam as *mut RECT;
            if !proposed_rect.is_null() {
                let rect = unsafe { &mut *proposed_rect };
                super::alignment_guide::on_moving(hwnd, rect);
            }
            0
        }

        WM_KEYDOWN => {
            let vk = wparam as u32;
            if vk == VK_ESCAPE {
                super::multi_select::clear();
                if let Some(data) = get_data(hwnd) {
                    data.selected_item = -1;
                    data.shift_anchor = -1;
                    render_layered_window(hwnd);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
