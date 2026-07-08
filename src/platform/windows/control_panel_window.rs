//! Windows control-panel window helpers.
//!
//! eframe can create the root viewport before app state is ready. These helpers
//! enforce the tray-first lifecycle at the Win32 window level: no taskbar entry
//! while hidden, explicit show/focus when the user opens the control panel.

use super::ffi_imports::{
    FindWindowW, GetCursorPos, GetMonitorInfoW, GetWindowLongPtrW, GetWindowRect, MonitorFromPoint,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow,
};
use super::ffi_types::{
    HWND, MONITORINFO, MONITOR_DEFAULTTONEAREST, POINT, RECT, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_SHOW, WS_EX_TOOLWINDOW,
};
use std::mem::size_of;
use std::ptr;

const GWL_EXSTYLE: i32 = -20;
const WS_EX_APPWINDOW: isize = 0x0004_0000;
const SWP_FRAMECHANGED: u32 = 0x0020;

/// Hide the egui control-panel window and remove its taskbar entry.
pub fn hide_control_panel_window(title: &str) -> bool {
    let Some(hwnd) = find_control_panel_window(title) else {
        return false;
    };

    unsafe {
        configure_as_tool_window(hwnd);
        park_offscreen(hwnd);
    }
    true
}

/// Show and focus the egui control-panel window without creating a taskbar entry.
pub fn show_control_panel_window(title: &str) -> bool {
    let Some(hwnd) = find_control_panel_window(title) else {
        return false;
    };

    unsafe {
        configure_as_tool_window(hwnd);
        center_on_cursor_monitor(hwnd);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
    true
}

fn find_control_panel_window(title: &str) -> Option<HWND> {
    let title_wide = wide_null(title);
    let hwnd = unsafe { FindWindowW(ptr::null(), title_wide.as_ptr()) };
    if hwnd.is_null() {
        log::debug!("未找到控制面板窗口: {}", title);
        None
    } else {
        Some(hwnd)
    }
}

unsafe fn configure_as_tool_window(hwnd: HWND) {
    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    let new_style = (style | WS_EX_TOOLWINDOW as isize) & !WS_EX_APPWINDOW;
    if new_style == style {
        return;
    }

    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
    SetWindowPos(
        hwnd,
        ptr::null_mut(),
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
    );
}

unsafe fn park_offscreen(hwnd: HWND) {
    SetWindowPos(
        hwnd,
        ptr::null_mut(),
        -32000,
        -32000,
        0,
        0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
    log::info!("控制面板窗口已停放到屏幕外");
}

unsafe fn center_on_cursor_monitor(hwnd: HWND) {
    let mut cursor = POINT { x: 0, y: 0 };
    if GetCursorPos(&mut cursor) == 0 {
        log::warn!("获取鼠标位置失败，无法居中控制面板窗口");
        return;
    }

    let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
    if monitor.is_null() {
        log::warn!("获取鼠标所在显示器失败，无法居中控制面板窗口");
        return;
    }

    let mut monitor_info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rcWork: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dwFlags: 0,
    };
    if GetMonitorInfoW(monitor, &mut monitor_info) == 0 {
        log::warn!("获取显示器工作区失败，无法居中控制面板窗口");
        return;
    }

    let mut window_rect = RECT {
        left: 0,
        top: 0,
        right: 320,
        bottom: 380,
    };
    if GetWindowRect(hwnd, &mut window_rect) == 0 {
        log::warn!("获取控制面板窗口尺寸失败，使用默认尺寸居中");
    }

    let (x, y) = centered_window_position(&monitor_info.rcWork, &window_rect);

    SetWindowPos(
        hwnd,
        ptr::null_mut(),
        x,
        y,
        0,
        0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
    log::info!("控制面板窗口已移动到当前显示器中心: {}, {}", x, y);
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn centered_window_position(work_area: &RECT, window_rect: &RECT) -> (i32, i32) {
    let window_w = (window_rect.right - window_rect.left).max(320);
    let window_h = (window_rect.bottom - window_rect.top).max(380);
    let work_w = work_area.right - work_area.left;
    let work_h = work_area.bottom - work_area.top;
    (
        work_area.left + (work_w - window_w) / 2,
        work_area.top + (work_h - window_h) / 2,
    )
}

#[cfg(test)]
mod position_tests {
    use super::*;

    #[test]
    fn test_centered_window_position_uses_current_window_size() {
        let work_area = RECT {
            left: 100,
            top: 50,
            right: 1100,
            bottom: 850,
        };
        let window_rect = RECT {
            left: 0,
            top: 0,
            right: 400,
            bottom: 500,
        };

        assert_eq!(
            centered_window_position(&work_area, &window_rect),
            (400, 200)
        );
    }

    #[test]
    fn test_centered_window_position_enforces_default_minimum_size() {
        let work_area = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let zero_sized_window = RECT {
            left: 10,
            top: 10,
            right: 10,
            bottom: 10,
        };

        assert_eq!(
            centered_window_position(&work_area, &zero_sized_window),
            (800, 350)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wide_null_terminates() {
        let wide = wide_null("Desktop Organizer 控制面板");
        assert_eq!(wide.last().copied(), Some(0));
    }
}
