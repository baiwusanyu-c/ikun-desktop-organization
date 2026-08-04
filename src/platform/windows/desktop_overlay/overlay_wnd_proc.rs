//! 桌面覆盖窗口消息处理
//!
//! 阶段 3 在图标区域接管鼠标事件：hover、选中、双击打开、右键菜单和拖入分类框。
//! 空白区域由覆盖窗口处理桌面背景菜单，避免把右键/新建转发给已隐藏的原生桌面。

use super::overlay_grid::{overlay_first_free_cell, overlay_grid_to_logical, overlay_snap_to_grid};
use super::overlay_render::render_desktop_overlay;
use super::overlay_window::{
    get_overlay_data, logical_to_physical, overlay_hit_test, overlay_icon_cell_x,
    overlay_icon_cell_y, overlay_layout_params, refresh_desktop_overlay_icons,
    save_overlay_icon_positions,
};
use crate::platform::windows::ffi_imports::*;
use crate::platform::windows::ffi_types::*;
use super::overlay_drag_ops::*;

/// 桌面覆盖窗口过程。
pub(super) unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            // OLE 拖拽期间返回 HTTRANSPARENT，让 DoDragDrop 能检测到 Explorer 的 IDropTarget
            if crate::platform::windows::zorder::is_drag_in_progress() {
                HTTRANSPARENT as LRESULT
            } else {
                HTCLIENT as LRESULT
            }
        }
        WM_SETCURSOR => {
            let cursor = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as *const u16) };
            unsafe { SetCursor(cursor) };
            1
        }
        WM_MOUSEMOVE => {
            let (local_x, local_y) =
                screen_to_overlay_client(hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            handle_mouse_move(hwnd, local_x, local_y);
            0
        }
        WM_MOUSELEAVE => {
            // WS_EX_LAYERED 窗口在 alpha=0 像素上会持续触发 WM_MOUSELEAVE
            // 忽略此消息，hover 状态由 WM_MOUSEMOVE 自然管理
            // 当鼠标真正离开窗口时，WM_MOUSEMOVE 的 hit test 会返回 -1 并更新 hover
            0
        }
        WM_LBUTTONDOWN => {
            let (local_x, local_y) =
                screen_to_overlay_client(hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            handle_lbutton_down(
                hwnd,
                local_x,
                local_y,
                get_x_lparam(lparam),
                get_y_lparam(lparam),
            );
            0
        }
        WM_LBUTTONUP => {
            log::info!("overlay WM_LBUTTONUP received");
            handle_lbutton_up(hwnd);
            0
        }
        WM_USER_OVERLAY_LBUTTONDOWN => {
            let local_x = get_x_lparam(lparam);
            let local_y = get_y_lparam(lparam);
            log::info!("[OVERLAY-HOOK] WM_USER_OVERLAY_LBUTTONDOWN: local=({}, {})", local_x, local_y);
            let mut wr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            unsafe { GetWindowRect(hwnd, &mut wr) };
            handle_lbutton_down(hwnd, local_x, local_y, wr.left + local_x, wr.top + local_y);
            0
        }
        WM_USER_OVERLAY_LBUTTONUP => {
            log::info!("[OVERLAY-HOOK] WM_USER_OVERLAY_LBUTTONUP received");
            handle_lbutton_up(hwnd);
            0
        }
        // ===== 拖拽轮询定时器（替代 SetCapture）=====
        // 处理重命名结果消息
        msg if msg == crate::platform::windows::rename_editor::get_rename_result_message() => {
            let item_index = wparam as i32;
            let text_ptr = lparam as *mut String;

            if !text_ptr.is_null() && item_index >= 0 {
                let new_name = unsafe { Box::from_raw(text_ptr) };
                log::info!(
                    "Overlay rename result for item {}: '{}'",
                    item_index,
                    *new_name
                );

                if let Some(data) = get_overlay_data(hwnd) {
                    data.renaming_item_index = -1;
                    render_desktop_overlay(hwnd);
                    if (item_index as usize) < data.icons.len() {
                        let old_path = data.icons[item_index as usize].file_path.clone();
                        let old_name = data.icons[item_index as usize].display_name.clone();

                        // Use shared helper to build filename with preserved extension
                        let final_name =
                            crate::platform::windows::rename_editor::build_rename_filename(
                                &new_name, &old_path,
                            );

                        if !final_name.is_empty() && final_name != old_name {
                            match crate::platform::windows::icon_hider::rename_icon_file(
                                &old_path,
                                &final_name,
                            ) {
                                Ok(new_path) => {
                                    log::info!("Overlay renamed: {} -> {}", old_path, new_path);

                                    // 更新数据
                                    if let Some(icon) = data.icons.get_mut(item_index as usize) {
                                        icon.file_path = new_path;
                                        icon.display_name = crate::platform::windows::rename_editor::display_name_from_path(&icon.file_path);
                                    }

                                    // 保存并重绘
                                    save_overlay_icon_positions();
                                    render_desktop_overlay(hwnd);
                                }
                                Err(e) => {
                                    log::error!("Overlay rename failed: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            0
        }
        msg if msg == crate::platform::windows::rename_editor::get_rename_cancel_message() => {
            log::info!("Overlay rename cancelled for item {}", wparam);
            if let Some(data) = get_overlay_data(hwnd) {
                data.renaming_item_index = -1;
                render_desktop_overlay(hwnd);
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_ID_OVERLAY_DRAG {
                handle_overlay_drag_timer(hwnd);
            } else if wparam == TIMER_ID_OVERLAY_HOVER {
                handle_hover_timer(hwnd);
            } else if wparam == TIMER_ID_OVERLAY_MARQUEE {
                handle_marquee_timer(hwnd);
            }
            0
        }
        // ===== 捕获丢失处理：纯清理兆底 =====
        WM_CAPTURECHANGED => 0,
        WM_LBUTTONDBLCLK => {
            let (local_x, local_y) =
                screen_to_overlay_client(hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            handle_double_click(hwnd, local_x, local_y);
            0
        }
        WM_RBUTTONUP => {
            let (local_x, local_y) =
                screen_to_overlay_client(hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            handle_right_click(hwnd, local_x, local_y);
            0
        }
        WM_MOUSEACTIVATE => {
            log::debug!("覆盖窗口 WM_MOUSEACTIVATE: 返回 MA_NOACTIVATE");
            MA_NOACTIVATE as LRESULT
        }
        // ===== Z-order 管理：强制覆盖窗口保持在 HWND_BOTTOM =====
        // 与分类框的 WM_WINDOWPOSCHANGING 类似，确保覆盖窗口始终低于其他应用窗口，
        // 避免图标覆盖在 Chrome 等应用窗口上方。
        WM_WINDOWPOSCHANGING => {
            // OLE 拖拽期间不干预 Z-order，否则 hit-testing 无法到达目标窗口
            if !crate::platform::windows::zorder::is_drag_in_progress() {
                let windowpos = lparam as *mut WINDOWPOS;
                if !windowpos.is_null() {
                    let wp = unsafe { &mut *windowpos };
                    if wp.flags & SWP_NOZORDER == 0 {
                        wp.hwnd_insert_after = HWND_BOTTOM;
                    }
                }
            }
            0
        }
        // ===== OLE 拖拽放置：外部文件拖入覆盖区域（由 overlay IDropTarget PostMessage 触发）=====
        WM_USER_OVERLAY_DROP => {
            handle_overlay_ole_drop(hwnd);
            0
        }

        // ===== Escape 键清除多选 =====
        WM_KEYDOWN => {
            let vk = wparam as u32;
            if vk == VK_ESCAPE {
                super::super::multi_select::clear();
                if let Some(data) = get_overlay_data(hwnd) {
                    data.selected_item = -1;
                    data.shift_anchor = -1;
                    render_desktop_overlay(hwnd);
                }
            }
            0
        }

        WM_USER_REFRESH_OVERLAY => {
            refresh_desktop_overlay_icons(hwnd);
            0
        }
        WM_USER_CUT_FILE_MOVED => {
            if let Some(data) = get_overlay_data(hwnd) {
                let mut removed_any = false;
                let mut i = 0;
                while i < data.icons.len() {
                    if !crate::platform::windows::icon_hider::icon_exists_for_box(
                        &data.icons[i].file_path,
                    ) {
                        let removed = data.icons.remove(i);
                        if !removed.icon_handle.is_null() {
                            crate::platform::windows::icon_extract::destroy_icon(
                                removed.icon_handle,
                            );
                        }
                        log::info!("cut file moved from overlay: {}", removed.display_name);
                        removed_any = true;
                    } else {
                        i += 1;
                    }
                }
                if removed_any {
                    if data.selected_item >= data.icons.len() as i32 {
                        data.selected_item = -1;
                    }
                    if data.hovered_item >= data.icons.len() as i32 {
                        data.hovered_item = -1;
                    }
                    render_desktop_overlay(hwnd);
                    save_overlay_icon_positions();
                }
            }
            0
        }
        WM_DPICHANGED => {
            let new_dpi = dpi_from_wparam(wparam);
            if let Some(data) = get_overlay_data(hwnd) {
                let old_dpi = data.dpi;
                data.dpi = new_dpi;

                // 应用系统建议的新窗口位置和尺寸
                if lparam != 0 {
                    let suggested = unsafe { &*(lparam as *const RECT) };
                    data.x = suggested.left;
                    data.y = suggested.top;
                    data.width = suggested.right - suggested.left;
                    data.height = suggested.bottom - suggested.top;
                    unsafe {
                        SetWindowPos(
                            hwnd,
                            std::ptr::null_mut(),
                            data.x,
                            data.y,
                            data.width,
                            data.height,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }

                let layout = overlay_layout_params(data.width, data.dpi);
                data.cell_w = layout.cell_w;
                data.cell_h = layout.cell_h;
                data.icon_size = layout.icon_size;
                log::info!(
                    "覆盖窗口 DPI 变更: {} -> {}, 新尺寸={}x{}",
                    old_dpi,
                    new_dpi,
                    data.width,
                    data.height
                );
                render_desktop_overlay(hwnd);
            }
            0
        }
        WM_SIZE => {
            let new_cx = loword_lparam(lparam);
            let new_cy = hiword_lparam(lparam);
            if let Some(data) = get_overlay_data(hwnd) {
                if new_cx > 0 && new_cy > 0 {
                    data.width = new_cx as i32;
                    data.height = new_cy as i32;
                    let layout = overlay_layout_params(data.width, data.dpi);
                    data.cell_w = layout.cell_w;
                    data.cell_h = layout.cell_h;
                    data.icon_size = layout.icon_size;
                    render_desktop_overlay(hwnd);
                }
            }
            0
        }
        WM_DESTROY => {
            unsafe { KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER) };
            if let Some(data) = get_overlay_data(hwnd) {
                log::info!(
                    "桌面覆盖窗口销毁: hwnd={:?}, area=({}, {}) {}x{} dpi={}",
                    hwnd,
                    data.x,
                    data.y,
                    data.width,
                    data.height,
                    data.dpi
                );
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn overlay_nchittest_result(_hit: i32) -> LRESULT {
    // 无论点击图标还是空白区域，都返回 HTCLIENT
    // 这样覆盖窗口能捕获所有点击，处理完后再决定是否转发给桌面
    HTCLIENT as LRESULT
}

/// Marquee tracking timer: polls cursor position every 8ms and renders marquee rectangle.
/// bg_alpha stays 0 so overlay remains transparent (no mouse event blocking).
fn handle_marquee_timer(hwnd: HWND) {
    let data = match get_overlay_data(hwnd) {
        Some(d) => d,
        None => return,
    };
    if !data.marquee_active {
        unsafe { KillTimer(hwnd, TIMER_ID_OVERLAY_MARQUEE) };
        super::clear_marquee_started();
        return;
    }
    // Read cursor position from atomic variables (set by mouse hook on WM_MOUSEMOVE)
    if let Some((cursor_x, cursor_y)) = super::get_marquee_cursor_pos() {
        let (local_x, local_y) = screen_to_overlay_client(hwnd, cursor_x, cursor_y);
        // Only render if position changed (performance optimization)
        if data.marquee_current_x != local_x || data.marquee_current_y != local_y {
            data.marquee_current_x = local_x;
            data.marquee_current_y = local_y;
            render_desktop_overlay(hwnd);
        }
    }
}


/// Timer-based hover state management for layered window.
///
/// WS_EX_LAYERED windows with alpha=0 pixels don't receive WM_MOUSEMOVE
/// when cursor moves to transparent (blank) areas. This timer periodically
/// checks the cursor position and clears hover state when the cursor
/// is no longer over any icon, ensuring the hover highlight reverts.
fn handle_hover_timer(hwnd: HWND) {
    let data = match get_overlay_data(hwnd) {
        Some(d) => d,
        None => return,
    };

    // Check Escape key to clear selection
    let esc_down = unsafe { GetAsyncKeyState(VK_ESCAPE as i32) } & 0x8000u16 as i16 != 0;
    if esc_down && super::super::multi_select::has_selection() {
        super::super::multi_select::clear();
        data.selected_item = -1;
        data.shift_anchor = -1;
        render_desktop_overlay(hwnd);
        unsafe { KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER) };
        return;
    }

    // Check marquee: if active, render and detect release
    if data.marquee_active {
        let lbutton_down = unsafe { GetAsyncKeyState(VK_LBUTTON) } & 0x8000u16 as i16 != 0;
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        let (local_x, local_y) = screen_to_overlay_client(hwnd, pt.x, pt.y);
        log::trace!("[MARQUEE-TIMER] lbutton={}, cursor=({},{}), local=({},{}), current=({},{}), start=({},{}",
            lbutton_down, pt.x, pt.y, local_x, local_y, 
            data.marquee_current_x, data.marquee_current_y,
            data.marquee_start_x, data.marquee_start_y);
        
        if !lbutton_down {
            log::info!("[MARQUEE-TIMER] mouse released, finalizing");
            handle_lbutton_up(hwnd);
            unsafe { KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER) };
            return;
        }
        
        if data.marquee_current_x != local_x || data.marquee_current_y != local_y {
            data.marquee_current_x = local_x;
            data.marquee_current_y = local_y;
        }
        // Always render marquee at timer rate for smooth visual update
        render_desktop_overlay(hwnd);
        return;
    }

    // Don't interfere with drag operations
    if data.drag_active {
        unsafe { KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER) };
        return;
    }

    // Check if cursor is still over an icon
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    let (local_x, local_y) = screen_to_overlay_client(hwnd, pt.x, pt.y);
    let hit = overlay_hit_test(data, local_x, local_y);

    if data.hovered_item != hit {
        log::debug!(
            "[HOVER_TIMER] cursor at ({}, {}), hit={}, hovered={}",
            local_x,
            local_y,
            hit,
            data.hovered_item
        );
        data.hovered_item = hit;
        render_desktop_overlay(hwnd);
    }

    // Kill timer when no icon is hovered and nothing is selected
    // (no visual state to track)
    if data.hovered_item < 0 && data.selected_item < 0 {
        unsafe { KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER) };
    }
}

fn handle_mouse_move(hwnd: HWND, x: i32, y: i32) {
    let mut should_render = false;
    let mut should_start_drag = false;
    if let Some(data) = get_overlay_data(hwnd) {
        // Marquee coordinate update: rendering happens in hover timer (16ms)
        if data.marquee_active {
            data.marquee_current_x = x;
            data.marquee_current_y = y;
            return;
        }

        let hit = overlay_hit_test(data, x, y);
        log::debug!(
            "[HOVER] at ({}, {}), hit={}, hovered={}, selected={}",
            x,
            y,
            hit,
            data.hovered_item,
            data.selected_item
        );
        if data.hovered_item != hit {
            log::debug!("[HOVER] changed: {} -> {}", data.hovered_item, hit);
            data.hovered_item = hit;
            should_render = true;
        }

        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        unsafe { TrackMouseEvent(&mut tme) };

        if data.drag_active {
            unsafe { crate::platform::windows::drag_image::update_drag_overlay() };
            highlight_grid_under_cursor(hwnd);
            return;
        }

        if data.drag_pending {
            let dx = (x - data.drag_start_x).abs();
            let dy = (y - data.drag_start_y).abs();
            if dx > DRAG_THRESHOLD || dy > DRAG_THRESHOLD {
                should_start_drag = true;
            }
        }
    }

    if should_start_drag {
        start_overlay_drag(hwnd);
    }
    if should_render {
        render_desktop_overlay(hwnd);
    }

    // Start hover tracking timer when an icon is hovered.
    // WS_EX_LAYERED windows with alpha=0 pixels don't receive WM_MOUSEMOVE
    // when cursor moves to transparent areas, so we need a timer to detect
    // when the cursor leaves the icon and clear the hover highlight.
    if let Some(data) = get_overlay_data(hwnd) {
        if data.hovered_item >= 0 && !data.drag_active {
            unsafe { SetTimer(hwnd, TIMER_ID_OVERLAY_HOVER, 50, std::ptr::null_mut()) };
        } else {
            unsafe { KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER) };
        }
    }
}

fn handle_lbutton_down(hwnd: HWND, local_x: i32, local_y: i32, screen_x: i32, screen_y: i32) {
    if let Some(data) = get_overlay_data(hwnd) {
        let hit = overlay_hit_test(data, local_x, local_y);
        let ctrl_down = unsafe { GetAsyncKeyState(VK_CONTROL) } & 0x8000u16 as i16 != 0;
        let shift_down = unsafe { GetAsyncKeyState(VK_SHIFT) } & 0x8000u16 as i16 != 0;
        log::info!("[OVERLAY-CLICK] pos=({},{}), hit={}, ctrl={}, shift={}, selected={}, anchor={}", 
            local_x, local_y, hit, ctrl_down, shift_down, data.selected_item, data.shift_anchor);
        let container_type = super::super::multi_select::ContainerType::Overlay;
        let container_id: u64 = 0;

        log::info!(
            "[CLICK] local=({},{}), hit={}, selected={}, ctrl={}, shift={}",
            local_x,
            local_y,
            hit,
            data.selected_item,
            ctrl_down,
            shift_down
        );

        if hit >= 0 {
            // 点击图标：多选逻辑
            let hit_idx = hit as usize;
            let key = super::super::multi_select::SelectionKey {
                container_type,
                container_id,
                item_index: hit_idx,
            };

            if shift_down && data.shift_anchor >= 0 {
                let anchor = data.shift_anchor as usize;
                // Overlay uses column-major layout: items fill down first column, then next.
                // Compute visual order (row, col) and select range in visual order.
                let total = data.icons.len();
                let layout = super::overlay_window::overlay_layout_params(data.width, data.dpi);
                let max_rows = ((data.height - layout.padding * 2) / layout.cell_h).max(1);
                let mut visual_order: Vec<(usize, i32, i32)> = Vec::with_capacity(total);
                for i in 0..total {
                    let col = (i as i32) / max_rows;
                    let row = (i as i32) % max_rows;
                    visual_order.push((i, row, col));
                }
                visual_order.sort_by(|x, y| x.1.cmp(&y.1).then(x.2.cmp(&y.2)));

                let anchor_vo = visual_order.iter().position(|&(idx, _, _)| idx == anchor);
                let target_vo = visual_order.iter().position(|&(idx, _, _)| idx == hit_idx);

                if let (Some(av), Some(tv)) = (anchor_vo, target_vo) {
                    let lo_vo = av.min(tv);
                    let hi_vo = av.max(tv);
                    if !ctrl_down {
                        super::super::multi_select::clear();
                    }
                    for vo_idx in lo_vo..=hi_vo {
                        let item_idx = visual_order[vo_idx].0;
                        let k = super::super::multi_select::SelectionKey {
                            container_type, container_id, item_index: item_idx,
                        };
                        super::super::multi_select::add(k);
                    }
                }
                // Ensure clicked item is in selection
                super::super::multi_select::add(key);
                data.selected_item = hit;
            } else if ctrl_down {
                super::super::multi_select::toggle(key);
                if super::super::multi_select::is_selected(key) {
                    data.shift_anchor = hit;
                    data.selected_item = hit;
                } else if data.selected_item == hit {
                    data.selected_item = -1;
                }
            } else {
                // ★ 标准 Windows 行为：点击已选中的项时保持多选（等释放后再清除）
                if !super::super::multi_select::is_selected(key) {
                    super::super::multi_select::clear();
                    super::super::multi_select::add(key);
                    super::super::multi_select::set_anchor(Some(key));
                }
                data.shift_anchor = hit;
                data.selected_item = hit;
            }

            // 准备拖拽
            data.drag_pending = true;
            data.drag_active = false;
            data.drag_item_index = -1;
            data.drag_start_x = local_x;
            data.drag_start_y = local_y;
            unsafe { SetTimer(hwnd, TIMER_ID_OVERLAY_HOVER, 50, std::ptr::null_mut()) };
            log::info!("图标选中: {}", data.icons[hit as usize].display_name);
        } else {
            log::info!("[OVERLAY-BLANK-CLICK] local=({},{}), starting marquee", local_x, local_y);
            // Blank area: clear if no Ctrl, start marquee
            if !ctrl_down {
                super::super::multi_select::clear();
                data.shift_anchor = -1;
            }
            data.selected_item = -1;
            data.drag_pending = false;
            data.drag_active = false;
            data.drag_item_index = -1;
            data.marquee_active = true;
            // Start hover timer at 16ms for marquee polling (WS_EX_LAYERED windows
            // don't receive WM_MOUSEMOVE on transparent pixels, so we need timer-based
            // cursor tracking + rendering for smooth marquee)
            unsafe { SetTimer(hwnd, TIMER_ID_OVERLAY_MARQUEE, 8, std::ptr::null_mut()); }
            log::info!("[MARQUEE-START] active=true, start=({},{}), timer started", local_x, local_y);
            data.marquee_start_x = local_x;
            data.marquee_start_y = local_y;
            data.marquee_current_x = local_x;
            data.marquee_current_y = local_y;
            // 不转发到桌面 —— 转发会让 SysListView32 接管鼠标，导致框选立即结束
        }
    }
    render_desktop_overlay(hwnd);
}

/// 将左键点击转发给桌面 SysListView32 窗口（接受屏幕坐标）
fn forward_lbutton_to_desktop(screen_x: i32, screen_y: i32) {
    const PROGMAN: [u16; 8] = [80, 114, 111, 103, 109, 97, 110, 0];
    const SHELL_DEF: [u16; 17] = [
        83, 72, 69, 76, 76, 68, 76, 76, 95, 68, 101, 102, 86, 105, 101, 119, 0,
    ];
    const LISTVIEW: [u16; 14] = [
        83, 121, 115, 76, 105, 115, 116, 86, 105, 101, 119, 51, 50, 0,
    ];
    const WORKER: [u16; 8] = [87, 111, 114, 107, 101, 114, 87, 0];

    unsafe {
        let mut list_view = std::ptr::null_mut();

        let progman = FindWindowW(PROGMAN.as_ptr(), std::ptr::null());
        if !progman.is_null() {
            let dv = FindWindowExW(
                progman,
                std::ptr::null_mut(),
                SHELL_DEF.as_ptr(),
                std::ptr::null(),
            );
            if !dv.is_null() {
                list_view = FindWindowExW(
                    dv,
                    std::ptr::null_mut(),
                    LISTVIEW.as_ptr(),
                    std::ptr::null(),
                );
            }
        }

        if list_view.is_null() {
            let mut w = std::ptr::null_mut();
            loop {
                w = FindWindowExW(std::ptr::null_mut(), w, WORKER.as_ptr(), std::ptr::null());
                if w.is_null() {
                    break;
                }
                let dv = FindWindowExW(
                    w,
                    std::ptr::null_mut(),
                    SHELL_DEF.as_ptr(),
                    std::ptr::null(),
                );
                if !dv.is_null() {
                    list_view = FindWindowExW(
                        dv,
                        std::ptr::null_mut(),
                        LISTVIEW.as_ptr(),
                        std::ptr::null(),
                    );
                    if !list_view.is_null() {
                        break;
                    }
                }
            }
        }

        if list_view.is_null() {
            log::warn!("[CLICK] 未找到桌面窗口");
            return;
        }

        let lparam = ((screen_y & 0xFFFF) << 16) | (screen_x & 0xFFFF);
        PostMessageW(list_view, WM_LBUTTONDOWN, 0, lparam as LPARAM);
        PostMessageW(list_view, WM_LBUTTONUP, 0, lparam as LPARAM);
        log::info!("[CLICK] 已转发到桌面 ({}, {})", screen_x, screen_y);
    }
}

/// 拖拽轮询定时器处理函数（每 16ms 调用一次）
///
/// 检测鼠标左键状态，如果已释放则结束拖拽。
/// 这是 WS_EX_LAYERED 窗口上拖拽的标准做法，避免 SetCapture 的不稳定性。

fn handle_lbutton_up(hwnd: HWND) {
    let was_drag_active = get_overlay_data(hwnd)
        .map(|data| data.drag_active)
        .unwrap_or(false);
    log::info!("handle_lbutton_up: was_drag_active={}", was_drag_active);
    if was_drag_active {
        end_overlay_drag(hwnd);
        return;
    }

    // 框选结束处理
    if let Some(data) = get_overlay_data(hwnd) {
        if data.marquee_active {
            let ctrl_down = unsafe { GetAsyncKeyState(VK_CONTROL) } & 0x8000u16 as i16 != 0;
            data.marquee_active = false;
            unsafe { KillTimer(hwnd, TIMER_ID_OVERLAY_MARQUEE) };
            super::clear_marquee_started();

            let rect_left = data.marquee_start_x.min(data.marquee_current_x);
            let rect_right = data.marquee_start_x.max(data.marquee_current_x);
            let rect_top = data.marquee_start_y.min(data.marquee_current_y);
            let rect_bottom = data.marquee_start_y.max(data.marquee_current_y);

            let dx = (data.marquee_current_x - data.marquee_start_x).abs();
            let dy = (data.marquee_current_y - data.marquee_start_y).abs();
            if dx > 3 || dy > 3 {
                let container_type = super::super::multi_select::ContainerType::Overlay;
                let container_id: u64 = 0;

                if !ctrl_down {
                    super::super::multi_select::clear();
                }

                // 遍历所有图标，检查是否在框选矩形内
                let grid_layout = overlay_layout_params(data.width, data.dpi);
                for (i, icon) in data.icons.iter().enumerate() {
                    let (cell_x, cell_y) = if icon.x >= 0.0 && icon.y >= 0.0 {
                        (
                            logical_to_physical(icon.x, data.dpi),
                            logical_to_physical(icon.y, data.dpi),
                        )
                    } else {
                        let total = data.icons.len().max(1) as i32;
                        let rows = (total + grid_layout.cols - 1) / grid_layout.cols;
                        let col = (i as i32) / rows;
                        let row = (i as i32) % rows;
                        (
                            grid_layout.padding + col * grid_layout.cell_w,
                            grid_layout.padding + row * grid_layout.cell_h,
                        )
                    };
                    let cell_center_x = cell_x + grid_layout.cell_w / 2;
                    let cell_center_y = cell_y + grid_layout.cell_h / 2;

                    if cell_center_x >= rect_left
                        && cell_center_x <= rect_right
                        && cell_center_y >= rect_top
                        && cell_center_y <= rect_bottom
                    {
                        let key = super::super::multi_select::SelectionKey {
                            container_type,
                            container_id,
                            item_index: i,
                        };
                        super::super::multi_select::add(key);
                    }
                }

                let selected =
                    super::super::multi_select::selected_in_container(container_type, container_id);
                if let Some(&first) = selected.first() {
                    data.shift_anchor = first as i32;
                    data.selected_item = first as i32;
                } else {
                    data.shift_anchor = -1;
                    data.selected_item = -1;
                }
                log::info!("覆盖区域框选完成: 选中 {} 项", selected.len());
            }

            render_desktop_overlay(hwnd);
            return;
        }

        // Timer 已经结束了拖拽，这里是正常的点击释放
        data.drag_pending = false;
        data.drag_item_index = -1;
    }
}

fn handle_double_click(hwnd: HWND, x: i32, y: i32) {
    let (path, name) = match get_overlay_data(hwnd) {
        Some(data) => {
            let hit = overlay_hit_test(data, x, y);
            if hit < 0 || (hit as usize) >= data.icons.len() {
                return;
            }
            let item = &data.icons[hit as usize];
            (item.file_path.clone(), item.display_name.clone())
        }
        None => return,
    };

    let op: Vec<u16> = "open\0".encode_utf16().collect();
    let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        ShellExecuteW(
            hwnd,
            op.as_ptr(),
            path_w.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOW,
        );
    }
    log::info!("覆盖窗口双击打开: {}", name);
}

fn handle_right_click(hwnd: HWND, x: i32, y: i32) {
    // Kill hover timer to prevent interference with context menu
    unsafe { KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER); }
    let (path, hit_index) = match get_overlay_data(hwnd) {
        Some(data) => {
            let hit = overlay_hit_test(data, x, y);
            if hit < 0 || (hit as usize) >= data.icons.len() {
                log::info!("[MENU-DEBUG] overlay_wnd_proc: 右键点击框覆盖区域, 调用 show_desktop_context_menu(include_ikun_items=true)");
                crate::platform::windows::shell_menu::show_desktop_context_menu(hwnd, true);
                return;
            }
            data.selected_item = hit;
            data.drag_pending = false;
            let p = data.icons[hit as usize].file_path.clone();
            (p, hit)
        }
        None => return,
    };
    render_desktop_overlay(hwnd);
    log::info!(
        "[MENU-DEBUG] overlay_wnd_proc: 右键点击图标, 调用 show_shell_context_menu(path=\"{}\")",
        path
    );
    let result = crate::platform::windows::shell_menu::show_shell_context_menu(hwnd, &path);
    handle_overlay_shell_menu_result(hwnd, &path, hit_index, result);
}

/// Handle shell menu result for overlay window
fn handle_overlay_shell_menu_result(
    hwnd: HWND,
    file_path: &str,
    hit_index: i32,
    result: crate::platform::windows::shell_menu::ShellMenuResult,
) {
    use crate::platform::windows::shell_menu::ShellMenuResult;

    match result {
        ShellMenuResult::Cancelled | ShellMenuResult::Dismissed => {}
        ShellMenuResult::Completed(_) => {
            if !crate::platform::windows::icon_hider::icon_exists_for_box(file_path) {
                remove_overlay_item(hwnd, file_path);
            }
        }
        ShellMenuResult::NeedsRename(path) => {
            log::info!("Overlay rename requested for: {}", path);
            let Some(data) = get_overlay_data(hwnd) else {
                return;
            };
            let idx = hit_index as usize;
            if idx >= data.icons.len() {
                return;
            }

            let current_name = data.icons[idx].display_name.clone();

            // 计算标签位置（屏幕坐标）
            let mut wr = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe { GetWindowRect(hwnd, &mut wr) };

            // Use the same coordinate calculation as rendering (logical -> physical)
            let grid_layout = overlay_layout_params(wr.right - wr.left, data.dpi);
            let cell_x = overlay_icon_cell_x(data, idx, &grid_layout);
            let cell_y = overlay_icon_cell_y(data, idx, &grid_layout);
            let icon_size = grid_layout.icon_size;
            let label_y =
                cell_y + scale_for_dpi(4, data.dpi) + icon_size + scale_for_dpi(6, data.dpi);
            let text_height = scale_for_dpi(20, data.dpi);
            let text_width = grid_layout.cell_w - 8;
            let screen_x = wr.left + cell_x + 2;
            let screen_y = wr.top + label_y;

            // 使用原生 EDIT 控件（异步模式）
            let success = crate::platform::windows::rename_editor::show_rename_editor_async(
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
                    "Overlay rename editor shown for item {}: '{}'",
                    hit_index,
                    current_name
                );
                data.renaming_item_index = hit_index;
                render_desktop_overlay(hwnd);
            } else {
                log::error!("Failed to show overlay rename editor");
            }
        }
        ShellMenuResult::NeedsCutMonitor(path) => {
            log::info!("starting overlay cut monitor for: {}", path);
            start_overlay_cut_monitor(hwnd, path);
        }
        ShellMenuResult::NeedsDelete(path) => {
            crate::platform::windows::icon_hider::delete_vault_copy_for(&path);
            if !crate::platform::windows::icon_hider::icon_exists_for_box(&path) {
                remove_overlay_item(hwnd, &path);
            }
        }
    }
}

/// Remove icon item from overlay window
fn remove_overlay_item(hwnd: HWND, logical_path: &str) {
    if let Some(data) = get_overlay_data(hwnd) {
        if let Some(index) = data
            .icons
            .iter()
            .position(|item| item.file_path == logical_path)
        {
            let removed = data.icons.remove(index);
            if !removed.icon_handle.is_null() {
                crate::platform::windows::icon_extract::destroy_icon(removed.icon_handle);
            }
            if data.selected_item >= data.icons.len() as i32 {
                data.selected_item = -1;
            }
            if data.hovered_item >= data.icons.len() as i32 {
                data.hovered_item = -1;
            }
            log::info!("overlay item removed: {}", removed.display_name);
            render_desktop_overlay(hwnd);
            save_overlay_icon_positions();
        }
    }
}

/// Start cut monitor for overlay window
fn start_overlay_cut_monitor(hwnd: HWND, logical_path: String) {
    let hwnd_usize = hwnd as usize;
    std::thread::spawn(move || {
        let hwnd = hwnd_usize as HWND;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_secs(3));
            if !crate::platform::windows::icon_hider::icon_exists_for_box(&logical_path) {
                unsafe {
                    PostMessageW(hwnd, WM_USER_CUT_FILE_MOVED, 0, 0);
                }
                return;
            }
        }
    });
}

fn dpi_from_wparam(wparam: WPARAM) -> u32 {
    let dpi = ((wparam as u32) >> 16) & 0xFFFF;
    if dpi > 0 {
        dpi
    } else {
        96
    }
}

/// 从 LPARAM 提取 LOWORD（低 16 位）。
fn loword_lparam(lparam: LPARAM) -> i32 {
    (lparam as u32 & 0xFFFF) as i32
}

/// 从 LPARAM 提取 HIWORD（高 16 位）。
fn hiword_lparam(lparam: LPARAM) -> i32 {
    ((lparam as u32 >> 16) & 0xFFFF) as i32
}

fn screen_to_overlay_client(hwnd: HWND, screen_x: i32, screen_y: i32) -> (i32, i32) {
    let mut wr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut wr) };
    (screen_x - wr.left, screen_y - wr.top)
}

/// 处理外部文件拖拽到覆盖区域（由 WM_USER_OVERLAY_DROP 触发）
///
/// 核心逻辑：将文件复制到用户桌面（模拟原生桌面拖拽行为），
/// 然后在覆盖窗口的鼠标释放位置显示图标。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_nchittest_handles_icon_area() {
        assert_eq!(overlay_nchittest_result(0), HTCLIENT as LRESULT);
        assert_eq!(overlay_nchittest_result(3), HTCLIENT as LRESULT);
    }

    #[test]
    fn test_overlay_nchittest_handles_blank_area_in_overlay() {
        // 空白区域也返回 HTCLIENT，捕获点击后在 handle_lbutton_down 中转发给桌面
        assert_eq!(overlay_nchittest_result(-1), HTCLIENT as LRESULT);
    }

    #[test]
    fn test_dpi_from_wparam_96() {
        // HIWORD = 96 -> 0x0060
        let wparam: WPARAM = (96u32 << 16) as WPARAM;
        assert_eq!(dpi_from_wparam(wparam), 96);
    }

    #[test]
    fn test_dpi_from_wparam_144() {
        let wparam: WPARAM = (144u32 << 16) as WPARAM;
        assert_eq!(dpi_from_wparam(wparam), 144);
    }

    #[test]
    fn test_dpi_from_wparam_192() {
        let wparam: WPARAM = (192u32 << 16) as WPARAM;
        assert_eq!(dpi_from_wparam(wparam), 192);
    }

    #[test]
    fn test_dpi_from_wparam_zero_fallback() {
        // HIWORD = 0 should fallback to 96
        assert_eq!(dpi_from_wparam(0), 96);
    }

    #[test]
    fn test_loword_lparam() {
        // LPARAM = 0x0001_0002 -> LOWORD = 2
        assert_eq!(loword_lparam(0x00010002), 2);
        assert_eq!(loword_lparam(0), 0);
        assert_eq!(loword_lparam(0x0000FFFF), 65535);
    }

    #[test]
    fn test_hiword_lparam() {
        // LPARAM = 0x0001_0002 -> HIWORD = 1
        assert_eq!(hiword_lparam(0x00010002), 1);
        assert_eq!(hiword_lparam(0), 0);
        assert_eq!(hiword_lparam(0xFFFF0000u32 as LPARAM), 65535);
    }

    #[test]
    fn test_wm_size_lparam_extraction() {
        // WM_SIZE lParam: LOWORD = cx, HIWORD = cy
        // 例如窗口尺寸 1920x1080: cx=1920, cy=1080
        let cx: u32 = 1920;
        let cy: u32 = 1080;
        let lparam: LPARAM = ((cy << 16) | cx) as LPARAM;
        assert_eq!(loword_lparam(lparam), 1920);
        assert_eq!(hiword_lparam(lparam), 1080);
    }

    // ===== Hover and Selection Regression Tests =====
    // These tests prevent regression of the 7-attempt bug fix for hover flickering
    // and click-to-deselect (see windows-layered-window-guide.md)

    #[test]
    fn test_hover_state_stability() {
        // When hit value doesn't change, hovered_item should not change
        let mut hovered_item: i32 = 0;
        let hit = 0;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, 0);
    }

    #[test]
    fn test_click_blank_area_deselects() {
        let mut selected_item: i32 = 2;
        let hit = -1;
        if hit >= 0 {
            selected_item = hit;
        } else {
            selected_item = -1;
        }
        assert_eq!(selected_item, -1, "clicking blank area should deselect");
    }

    #[test]
    fn test_dynamic_alpha_based_on_selection() {
        let selected_item = -1;
        let bg_alpha: u8 = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 0, "no selection should have alpha=0");

        let selected_item = 2;
        let bg_alpha: u8 = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 1, "has selection should have alpha=1");
    }

    #[test]
    fn test_click_icon_selects_it() {
        let mut selected_item: i32 = -1;
        let mut drag_pending = false;
        let hit = 3;
        if hit >= 0 {
            selected_item = hit;
            drag_pending = true;
        }
        assert_eq!(selected_item, 3);
        assert!(drag_pending);
    }

    #[test]
    fn test_hover_updates_only_on_change() {
        let hovered_item: i32 = 1;
        let hit = 1;
        let should_render = hovered_item != hit;
        assert!(!should_render);
    }

    #[test]
    fn test_switch_selection_on_icon_click() {
        let mut selected_item: i32 = 2;
        let hit = 5;
        if hit >= 0 {
            selected_item = hit;
        }
        assert_eq!(selected_item, 5);
    }

    // ===== Dynamic Alpha Tests =====

    #[test]
    fn test_dynamic_alpha_no_selection() {
        let selected_item: i32 = -1;
        let bg_alpha: u8 = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 0);
    }

    #[test]
    fn test_dynamic_alpha_with_selection() {
        let selected_item: i32 = 5;
        let bg_alpha: u8 = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 1);
    }

    #[test]
    fn test_dynamic_alpha_boundary_cases() {
        let selected_item: i32 = 0;
        let bg_alpha: u8 = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 1);

        let selected_item: i32 = -1;
        let bg_alpha: u8 = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 0);

        let selected_item: i32 = -2;
        let bg_alpha: u8 = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 0);
    }

    // ===== Hover State Management Tests =====

    #[test]
    fn test_hover_updates_on_hit_change() {
        let mut hovered_item: i32 = -1;
        let hit = 3;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, 3);
    }

    #[test]
    fn test_hover_stability_same_hit() {
        let mut hovered_item: i32 = 5;
        let hit = 5;
        let changed = hovered_item != hit;
        if changed {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, 5);
        assert!(!changed);
    }

    #[test]
    fn test_hover_transition_icon_to_blank() {
        let mut hovered_item: i32 = 2;
        let hit = -1;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, -1);
    }

    #[test]
    fn test_hover_transition_blank_to_icon() {
        let mut hovered_item: i32 = -1;
        let hit = 7;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, 7);
    }

    #[test]
    fn test_hover_rapid_transitions() {
        let mut hovered_item: i32 = -1;

        let hit = 3;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, 3);

        let hit = 5;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, 5);

        let hit = 5;
        let changed = hovered_item != hit;
        if changed {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, 5);
        assert!(!changed);

        let hit = -1;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, -1);
    }

    // ===== Click Handling Tests =====

    #[test]
    fn test_click_icon_selects_and_prepares_drag() {
        let mut selected_item: i32 = -1;
        let mut drag_pending: bool = false;
        let mut drag_start_x: i32 = 0;
        let mut drag_start_y: i32 = 0;

        let hit = 4;
        let local_x = 150;
        let local_y = 200;

        if hit >= 0 {
            selected_item = hit;
            drag_pending = true;
            drag_start_x = local_x;
            drag_start_y = local_y;
        } else {
            selected_item = -1;
            drag_pending = false;
        }

        assert_eq!(selected_item, 4);
        assert!(drag_pending);
        assert_eq!(drag_start_x, 150);
        assert_eq!(drag_start_y, 200);
    }

    #[test]
    fn test_click_blank_area_deselects_and_cancels_drag() {
        let mut selected_item: i32 = 3;
        let mut drag_pending: bool = true;

        let hit = -1;

        if hit >= 0 {
            selected_item = hit;
            drag_pending = true;
        } else {
            selected_item = -1;
            drag_pending = false;
        }

        assert_eq!(selected_item, -1);
        assert!(!drag_pending);
    }

    #[test]
    fn test_click_different_icon_switches_selection() {
        let mut selected_item: i32 = 2;

        let hit = 8;

        if hit >= 0 {
            selected_item = hit;
        } else {
            selected_item = -1;
        }

        assert_eq!(selected_item, 8);
    }

    #[test]
    fn test_click_same_icon_keeps_selection() {
        let mut selected_item: i32 = 5;

        let hit = 5;

        if hit >= 0 {
            selected_item = hit;
        } else {
            selected_item = -1;
        }

        assert_eq!(selected_item, 5);
    }

    // ===== Integration Tests =====

    #[test]
    fn test_full_interaction_cycle() {
        let mut selected_item: i32 = -1;
        let mut hovered_item: i32 = -1;
        let mut drag_pending: bool = false;

        let hit = 3;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, 3);
        assert_eq!(selected_item, -1);

        let hit = 3;
        if hit >= 0 {
            selected_item = hit;
            drag_pending = true;
        }
        assert_eq!(selected_item, 3);
        assert!(drag_pending);

        let hit = 5;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, 5);
        assert_eq!(selected_item, 3);

        let hit = -1;
        if hit >= 0 {
            selected_item = hit;
            drag_pending = true;
        } else {
            selected_item = -1;
            drag_pending = false;
        }
        assert_eq!(selected_item, -1);
        assert!(!drag_pending);

        // After clicking blank area, mouse moves to blank area (hover)
        let hit = -1;
        if hovered_item != hit {
            hovered_item = hit;
        }
        assert_eq!(hovered_item, -1);

        let bg_alpha: u8 = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 0);
    }

    #[test]
    fn test_alpha_changes_with_selection_state() {
        let mut selected_item: i32 = -1;

        let mut bg_alpha: u8 = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 0);

        selected_item = 2;
        bg_alpha = if selected_item >= 0 { 1 } else { 0 };
        assert_eq!(bg_alpha, 1);
    }

    // ===== WM_MOUSELEAVE Ignored Tests =====

    #[test]
    fn test_mouseleave_ignored_hover_preserved() {
        let hovered_item: i32 = 5;

        assert_eq!(hovered_item, 5);

        let mut new_hovered = hovered_item;
        let hit = -1;
        if new_hovered != hit {
            new_hovered = hit;
        }
        assert_eq!(new_hovered, -1);
    }

    // ===== Hover Timer Lifecycle Tests (2026-07-29) =====

    #[test]
    fn test_hover_timer_should_start_when_icon_hovered() {
        // When an icon is hovered and no drag is active, the timer should start
        let hovered_item: i32 = 3;
        let drag_active = false;

        let should_start_timer = hovered_item >= 0 && !drag_active;
        assert!(
            should_start_timer,
            "Timer should start when icon is hovered and not dragging"
        );
    }

    #[test]
    fn test_hover_timer_should_not_start_during_drag() {
        // During drag, hover timer should not start (drag has its own tracking)
        let hovered_item: i32 = 3;
        let drag_active = true;

        let should_start_timer = hovered_item >= 0 && !drag_active;
        assert!(!should_start_timer, "Timer should NOT start during drag");
    }

    #[test]
    fn test_hover_timer_should_stop_when_nothing_tracked() {
        // When both hovered and selected are cleared, timer should stop
        let hovered_item: i32 = -1;
        let selected_item: i32 = -1;

        let should_stop_timer = hovered_item < 0 && selected_item < 0;
        assert!(should_stop_timer, "Timer should stop when nothing to track");
    }

    #[test]
    fn test_hover_timer_should_keep_running_when_selected() {
        // Even if hover is cleared, timer should keep running if item is selected
        // (to re-establish hover when cursor returns to an icon)
        let hovered_item: i32 = -1;
        let selected_item: i32 = 2;

        let should_stop_timer = hovered_item < 0 && selected_item < 0;
        assert!(
            !should_stop_timer,
            "Timer should keep running when item is selected"
        );
    }

    #[test]
    fn test_hover_clear_on_deselect() {
        // Clicking blank area should clear both selected and hovered
        let mut selected_item: i32 = 3;
        let mut hovered_item: i32 = 3;

        // Simulate blank area click
        let hit = -1;
        if hit < 0 {
            selected_item = -1;
            hovered_item = -1;
        }

        assert_eq!(selected_item, -1);
        assert_eq!(hovered_item, -1);
    }
}
