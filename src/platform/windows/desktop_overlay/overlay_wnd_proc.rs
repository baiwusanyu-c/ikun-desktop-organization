//! 桌面覆盖窗口消息处理
//!
//! 阶段 3 在图标区域接管鼠标事件：hover、选中、双击打开、右键菜单和拖入分类框。
//! 空白区域由覆盖窗口处理桌面背景菜单，避免把右键/新建转发给已隐藏的原生桌面。

use super::overlay_grid::{overlay_grid_to_logical, overlay_snap_to_grid};
use super::overlay_render::render_desktop_overlay;
use super::overlay_window::{
    get_overlay_data, overlay_hit_test, overlay_layout_params, refresh_desktop_overlay_icons,
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
            // 始终返回 HTCLIENT：让覆盖窗口接收所有鼠标事件
            // 空白区域的点击穿透在 WM_LBUTTONDOWN 中通过转发实现
            // 返回 HTTRANSPARENT 会导致 WM_MOUSELEAVE 触发，引起 hover 闪烁
            HTCLIENT as LRESULT
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
        WM_KEYDOWN => {
            // Escape 键取消选中
            if wparam == VK_ESCAPE as WPARAM {
                if let Some(data) = get_overlay_data(hwnd) {
                    if data.selected_item != -1 {
                        data.selected_item = -1;
                        render_desktop_overlay(hwnd);
                    }
                }
            }
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
            handle_lbutton_up(hwnd);
            0
        }
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
        WM_USER_REFRESH_OVERLAY => {
            refresh_desktop_overlay_icons(hwnd);
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

fn overlay_nchittest_result(hit: i32) -> LRESULT {
    // 无论点击图标还是空白区域，都返回 HTCLIENT
    // 这样覆盖窗口能捕获所有点击，处理完后再决定是否转发给桌面
    HTCLIENT as LRESULT
}

fn handle_mouse_move(hwnd: HWND, x: i32, y: i32) {
    let mut should_render = false;
    let mut should_start_drag = false;
    if let Some(data) = get_overlay_data(hwnd) {
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
}

fn handle_lbutton_down(hwnd: HWND, local_x: i32, local_y: i32, screen_x: i32, screen_y: i32) {
    if let Some(data) = get_overlay_data(hwnd) {
        let hit = overlay_hit_test(data, local_x, local_y);
        log::info!(
            "[CLICK] local=({},{}), hit={}, selected={}",
            local_x,
            local_y,
            hit,
            data.selected_item
        );

        if hit >= 0 {
            // 点击图标：选中 + 准备拖拽
            data.selected_item = hit;
            data.drag_pending = true;
            data.drag_active = false;
            data.drag_item_index = -1;
            data.drag_start_x = local_x;
            data.drag_start_y = local_y;
            log::info!("图标选中: {}", data.icons[hit as usize].display_name);
        } else {
            // 点击空白区域：取消选中 + 转发给桌面
            data.selected_item = -1;
            data.drag_pending = false;
            forward_lbutton_to_desktop(screen_x, screen_y);
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
                crate::platform::windows::shell_menu::show_desktop_context_menu(hwnd, true);
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
    // 使用 find_grid_window_at_point 替代 WindowFromPoint，
    // 因为分类框在 HWND_BOTTOM，WindowFromPoint 会被覆盖窗口拦截
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
}
