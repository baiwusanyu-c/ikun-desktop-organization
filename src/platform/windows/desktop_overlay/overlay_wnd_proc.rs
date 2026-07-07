//! 桌面覆盖窗口消息处理
//!
//! 阶段 3 在图标区域接管鼠标事件：hover、选中、双击打开、右键菜单和拖入分类框。
//! 空白区域继续 `HTTRANSPARENT`，避免遮挡分类框或桌面其他交互。

use super::overlay_render::render_desktop_overlay;
use super::overlay_window::{
    get_overlay_data, overlay_hit_test, overlay_layout_params, physical_to_logical,
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
            let screen_x = get_x_lparam(lparam);
            let screen_y = get_y_lparam(lparam);
            let (local_x, local_y) = screen_to_overlay_client(hwnd, screen_x, screen_y);
            let hit = get_overlay_data(hwnd)
                .map(|data| overlay_hit_test(data, local_x, local_y))
                .unwrap_or(-1);
            if hit >= 0 {
                HTCLIENT as LRESULT
            } else {
                HTTRANSPARENT as LRESULT
            }
        }
        WM_SETCURSOR => {
            let cursor = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as *const u16) };
            unsafe { SetCursor(cursor) };
            1
        }
        WM_MOUSEMOVE => {
            handle_mouse_move(hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            0
        }
        WM_MOUSELEAVE => {
            if let Some(data) = get_overlay_data(hwnd) {
                if data.hovered_item != -1 {
                    data.hovered_item = -1;
                    render_desktop_overlay(hwnd);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            handle_lbutton_down(hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            0
        }
        WM_LBUTTONUP => {
            handle_lbutton_up(hwnd);
            0
        }
        WM_LBUTTONDBLCLK => {
            handle_double_click(hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            0
        }
        WM_RBUTTONUP => {
            handle_right_click(hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            0
        }
        WM_MOUSEACTIVATE => {
            log::debug!("覆盖窗口 WM_MOUSEACTIVATE: 返回 MA_NOACTIVATE");
            MA_NOACTIVATE as LRESULT
        }
        WM_SIZE | WM_DPICHANGED => {
            if let Some(data) = get_overlay_data(hwnd) {
                let layout = overlay_layout_params(data.width, data.dpi);
                data.cell_w = layout.cell_w;
                data.cell_h = layout.cell_h;
                data.icon_size = layout.icon_size;
                render_desktop_overlay(hwnd);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
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

fn handle_mouse_move(hwnd: HWND, x: i32, y: i32) {
    let mut should_render = false;
    let mut should_start_drag = false;
    if let Some(data) = get_overlay_data(hwnd) {
        let hit = overlay_hit_test(data, x, y);
        if data.hovered_item != hit {
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
}

fn handle_lbutton_down(hwnd: HWND, x: i32, y: i32) {
    if let Some(data) = get_overlay_data(hwnd) {
        let hit = overlay_hit_test(data, x, y);
        data.selected_item = hit;
        data.drag_pending = hit >= 0;
        data.drag_active = false;
        data.drag_item_index = -1;
        data.drag_start_x = x;
        data.drag_start_y = y;
        if hit >= 0 {
            log::info!(
                "覆盖窗口图标选中: {}",
                data.icons[hit as usize].display_name
            );
        }
    }
    render_desktop_overlay(hwnd);
}

fn handle_lbutton_up(hwnd: HWND) {
    let was_drag_active = get_overlay_data(hwnd)
        .map(|data| data.drag_active)
        .unwrap_or(false);
    if was_drag_active {
        end_overlay_drag(hwnd);
        return;
    }
    if let Some(data) = get_overlay_data(hwnd) {
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
    let path = match get_overlay_data(hwnd) {
        Some(data) => {
            let hit = overlay_hit_test(data, x, y);
            if hit < 0 || (hit as usize) >= data.icons.len() {
                return;
            }
            data.selected_item = hit;
            data.drag_pending = false;
            data.icons[hit as usize].file_path.clone()
        }
        None => return,
    };
    render_desktop_overlay(hwnd);
    crate::platform::windows::shell_menu::show_shell_context_menu(hwnd, &path);
}

fn start_overlay_drag(hwnd: HWND) {
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
        SetCapture(hwnd);
    }
}

fn end_overlay_drag(hwnd: HWND) {
    let item_idx = match get_overlay_data(hwnd) {
        Some(data) => {
            let item_idx = data.drag_item_index;
            data.drag_active = false;
            data.drag_pending = false;
            data.drag_item_index = -1;
            item_idx
        }
        None => return,
    };

    unsafe {
        ReleaseCapture();
        crate::platform::windows::drag_image::destroy_drag_overlay();
    }

    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    let target_hwnd = unsafe { WindowFromPoint(pt) };
    if !is_grid_window(target_hwnd) {
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

    crate::platform::windows::drop_target::set_pending_add_item(file_path, display_name.clone());
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
        let layout = overlay_layout_params(data.width, data.dpi);
        let local_x = screen_x.saturating_sub(data.x) - layout.cell_w / 2;
        let local_y = screen_y.saturating_sub(data.y) - layout.cell_h / 2;
        let max_x = data.width.saturating_sub(layout.cell_w).max(0);
        let max_y = data.height.saturating_sub(layout.cell_h).max(0);
        data.icons[item_idx as usize].x = physical_to_logical(local_x.clamp(0, max_x), data.dpi);
        data.icons[item_idx as usize].y = physical_to_logical(local_y.clamp(0, max_y), data.dpi);
        data.selected_item = item_idx;
        data.hovered_item = item_idx;
    }
    render_desktop_overlay(hwnd);
    save_overlay_icon_positions();
}

fn highlight_grid_under_cursor(source_hwnd: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    let target_hwnd = unsafe { WindowFromPoint(pt) };
    if !target_hwnd.is_null() && target_hwnd != source_hwnd && is_grid_window(target_hwnd) {
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
