//! 显示器和窗口定位工具函数
//!
//! 提供显示器信息查询、DPI 获取、窗口定位等功能。
//! 所有函数返回物理像素坐标（虚拟桌面坐标系），不做 DPI 转换。

use super::ffi_imports::*;
use super::ffi_types::*;

/// 获取主屏幕分辨率（物理像素）
pub fn get_screen_resolution() -> (i32, i32) {
    let w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    (w, h)
}

/// 获取主显示器的 DPI
///
/// 使用 (0, 0) 点配合 MONITOR_DEFAULTTOPRIMARY 定位主显示器，
/// 通过 GetDpiForMonitor 查询 DPI。
pub fn get_primary_monitor_dpi() -> u32 {
    unsafe {
        let pt = POINT { x: 0, y: 0 };
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
        if hmon.is_null() {
            return 96;
        }
        // 使用 GetDpiForMonitor (Windows 8.1+) 查询 DPI
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        // MDT_EFFECTIVE_DPI = 0
        let hr = GetDpiForMonitor(hmon, 0, &mut dpi_x, &mut dpi_y);
        if hr == 0 && dpi_x > 0 {
            dpi_x
        } else {
            96
        }
    }
}

/// 获取虚拟桌面边界（包含所有显示器的联合区域）。
///
/// 返回值为主显示器坐标系下的虚拟桌面范围，用于跨显示器场景。
pub fn get_virtual_desktop_bounds() -> (i32, i32, i32, i32) {
    unsafe {
        const SM_XVIRTUALSCREEN: i32 = 76;
        const SM_YVIRTUALSCREEN: i32 = 77;
        const SM_CXVIRTUALSCREEN: i32 = 78;
        const SM_CYVIRTUALSCREEN: i32 = 79;
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let cx = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let cy = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if cx > 0 && cy > 0 {
            (x, y, cx, cy)
        } else {
            fallback_work_area()
        }
    }
}

/// 回退：主屏幕全屏范围
fn fallback_work_area() -> (i32, i32, i32, i32) {
    let (w, h) = get_screen_resolution();
    (0, 0, w, h)
}

/// 获取指定屏幕坐标处的 DPI 值（用于多显示器不同 DPI 场景）
pub fn dpi_at_screen_point(screen_x: i32, screen_y: i32) -> u32 {
    use std::ffi::c_void;

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    type HMONITOR = *mut c_void;

    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    const MDT_EFFECTIVE_DPI: u32 = 0;

    extern "system" {
        fn MonitorFromPoint(pt: POINT, dwFlags: u32) -> HMONITOR;
        fn GetDpiForMonitor(
            hmonitor: HMONITOR,
            dpiType: u32,
            dpiX: *mut u32,
            dpiY: *mut u32,
        ) -> i32;
    }

    let pt = POINT {
        x: screen_x,
        y: screen_y,
    };
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return 96;
    }
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } == 0 {
        dpi_x
    } else {
        96
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_screen_resolution_returns_positive() {
        let (w, h) = get_screen_resolution();
        assert!(w > 0, "屏幕宽度应为正数");
        assert!(h > 0, "屏幕高度应为正数");
    }

    #[test]
    fn test_get_primary_monitor_dpi_returns_valid() {
        let dpi = get_primary_monitor_dpi();
        assert!(dpi >= 96, "DPI 应至少为 96");
        assert!(dpi <= 400, "DPI 不应超过 400");
    }

    #[test]
    fn test_fallback_work_area() {
        let (x, y, w, h) = fallback_work_area();
        assert_eq!(x, 0);
        assert_eq!(y, 0);
        assert!(w > 0);
        assert!(h > 0);
    }

    #[test]
    fn test_get_virtual_desktop_bounds_valid() {
        let (_x, _y, w, h) = get_virtual_desktop_bounds();
        assert!(w > 0, "虚拟桌面宽度应为正数");
        assert!(h > 0, "虚拟桌面高度应为正数");
    }
}
