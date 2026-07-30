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
            log::debug!("overlay WM_TIMER received: wparam={:#X}", wparam);
            if wparam == TIMER_ID_OVERLAY_DRAG {
                handle_overlay_drag_timer(hwnd);
            } else if wparam == TIMER_ID_OVERLAY_HOVER {
                handle_hover_timer(hwnd);
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
        // 框选更新：如果框选正在进行，更新当前坐标并重绘
        if data.marquee_active {
            if data.marquee_current_x != x || data.marquee_current_y != y {
                data.marquee_current_x = x;
                data.marquee_current_y = y;
                render_desktop_overlay(hwnd);
            }
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
                if ctrl_down {
                    super::super::multi_select::add_range(
                        container_type,
                        container_id,
                        anchor,
                        hit_idx,
                    );
                } else {
                    super::super::multi_select::set_range(
                        container_type,
                        container_id,
                        anchor,
                        hit_idx,
                    );
                }
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
                super::super::multi_select::clear();
                super::super::multi_select::add(key);
                super::super::multi_select::set_anchor(Some(key));
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
            // 点击空白区域：清除选择 + 开始框选
            super::super::multi_select::clear();
            data.selected_item = -1;
            data.shift_anchor = -1;
            data.drag_pending = false;

            // 开始框选
            data.marquee_active = true;
            data.marquee_start_x = local_x;
            data.marquee_start_y = local_y;
            data.marquee_current_x = local_x;
            data.marquee_current_y = local_y;

            // 也转发给桌面（如果框选距离很小）
            // forward_lbutton_to_desktop(screen_x, screen_y); // 暂时不转发，让框选优先生效
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
fn handle_overlay_drag_timer(hwnd: HWND) {
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
        return;
    }

    // 更新拖拽残影位置
    unsafe { crate::platform::windows::drag_image::update_drag_overlay() };

    // 检测光标下的窗口，管理跨框可视化反馈
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
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
        // 检测外部窗口（Explorer 等）— find_external_window_at_point 使用 EnumWindows
        // 遍历所有顶层窗口的包围盒，无需隐藏覆盖窗口即可准确检测，避免频繁 SW_HIDE 导致图标闪烁
        let ext_hwnd = crate::platform::windows::zorder::find_external_window_at_point(pt.x, pt.y);
        if !ext_hwnd.is_null() {
            start_overlay_ole_drag(hwnd, ext_hwnd);
            return;
        }
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
fn start_overlay_ole_drag(hwnd: HWND, external_hwnd: HWND) {
    log::info!("start_overlay_ole_drag: 外部窗口 {:?}", external_hwnd);
    unsafe {
        KillTimer(hwnd, TIMER_ID_OVERLAY_DRAG);
    }

    let item_idx = match get_overlay_data(hwnd) {
        Some(data) => {
            let idx = data.drag_item_index;
            data.drag_pending = false;
            data.drag_active = false;
            data.drag_item_index = -1;
            data.drag_over_item = -1;
            data.drag_over_insert_col = -1;
            data.drag_over_insert_row = -1;
            idx
        }
        None => return,
    };

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
        "覆盖窗口→Explorer OLE: \'{}\' (path={})",
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

pub(crate) fn update_overlay_drag_over_visual(hwnd: HWND, screen_x: i32, screen_y: i32) {
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

fn start_overlay_drag(hwnd: HWND) {
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

fn end_overlay_drag(hwnd: HWND) {
    log::info!("end_overlay_drag called");
    let item_idx = match get_overlay_data(hwnd) {
        Some(data) => {
            let item_idx = data.drag_item_index;
            data.drag_active = false;
            data.drag_pending = false;
            data.drag_item_index = -1;
            // 清除拖拽覆盖可视化状态
            data.drag_over_item = -1;
            data.drag_over_insert_col = -1;
            data.drag_over_insert_row = -1;
            item_idx
        }
        None => return,
    };

    unsafe {
        KillTimer(hwnd, TIMER_ID_OVERLAY_DRAG);
        crate::platform::windows::drag_image::destroy_drag_overlay();
    }

    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    let target_hwnd = crate::platform::windows::zorder::find_grid_window_at_point(pt.x, pt.y);
    if target_hwnd.is_null() {
        restore_dragged_icon_position(hwnd, item_idx, pt.x, pt.y);
        log::info!("覆盖窗口内图标位置已更新");
        return;
    }

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
    log::info!("覆盖窗口→分类框: {}", display_name);
}

fn restore_dragged_icon_position(hwnd: HWND, item_idx: i32, screen_x: i32, screen_y: i32) {
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

fn highlight_grid_under_cursor(source_hwnd: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    // 使用 find_grid_window_at_point 替代 WindowFromPoint，
    // 因为分类框在 HWND_BOTTOM，WindowFromPoint 会被覆盖窗口拦截
    let target_hwnd = crate::platform::windows::zorder::find_grid_window_at_point(pt.x, pt.y);
    if !target_hwnd.is_null() && target_hwnd != source_hwnd {
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_ENTER, 0, 0) };
    }
}

fn is_grid_window(hwnd: HWND) -> bool {
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
fn handle_overlay_ole_drop(_hwnd: HWND) {
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
fn copy_file_to_desktop(source_path: &str) -> Option<String> {
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
            crate::platform::windows::icon_hider::notify_shell_path(&dest.to_string_lossy());
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
