//! 桌面工具模块
//!
//! 提供与原生桌面 SysListView32 交互的工具函数：
//! - 自动排列检测
//! - 图标精确定位
//!
//! # 设计说明
//!
//! 本模块不再使用覆盖区域窗口，而是直接利用 Windows 原生桌面能力。
//! 未分类图标留在桌面原位由 Windows 渲染，已分类图标从桌面移入仓库后自然不显示。
//!
//! ## 自动排列处理策略
//!
//! 当桌面开启"自动排列图标"时，`position_icon_at_screen` 会临时关闭自动排列、
//! 放置图标、然后恢复自动排列。这个过程对用户透明，无需弹窗提示。
//!
//! 如果用户希望永久关闭自动排列以获得更精确的控制，可通过控制面板设置。

use std::ffi::c_void;

type HWND = *mut c_void;

/// 检测桌面是否开启了"自动排列图标"
///
/// 通过检查 SysListView32 的 LVS_AUTOARRANGE 样式位判断。
/// 返回 `true` 表示自动排列已开启。
pub fn is_desktop_auto_arrange_enabled() -> bool {
    const LVS_AUTOARRANGE: u32 = 0x0100;
    const GWL_STYLE: i32 = -16;

    // 查找桌面 SysListView32
    let listview = match find_desktop_listview() {
        Some(h) => h,
        None => return false,
    };

    extern "system" {
        fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    }

    let style = unsafe { GetWindowLongPtrW(listview, GWL_STYLE) } as u32;
    style & LVS_AUTOARRANGE != 0
}

/// 查找桌面 SysListView32 窗口句柄
fn find_desktop_listview() -> Option<HWND> {
    extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> HWND;
        fn FindWindowExW(parent: HWND, after: HWND, class: *const u16, title: *const u16) -> HWND;
    }

    let progman_class: Vec<u16> = "Progman\0".encode_utf16().collect();
    let defview_class: Vec<u16> = "SHELLDLL_DefView\0".encode_utf16().collect();
    let listview_class: Vec<u16> = "SysListView32\0".encode_utf16().collect();
    let workerw_class: Vec<u16> = "WorkerW\0".encode_utf16().collect();

    unsafe {
        // 尝试 Progman
        let progman = FindWindowW(progman_class.as_ptr(), std::ptr::null());
        if !progman.is_null() {
            let defview = FindWindowExW(
                progman,
                std::ptr::null_mut(),
                defview_class.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    listview_class.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Some(lv);
                }
            }
        }

        // 尝试 WorkerW
        let mut worker = FindWindowW(workerw_class.as_ptr(), std::ptr::null());
        while !worker.is_null() {
            let defview = FindWindowExW(
                worker,
                std::ptr::null_mut(),
                defview_class.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    listview_class.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Some(lv);
                }
            }
            // 找下一个 WorkerW
            extern "system" {
                fn FindWindowExW(
                    parent: HWND,
                    after: HWND,
                    class: *const u16,
                    title: *const u16,
                ) -> HWND;
            }
            let desktop_hwnd = {
                extern "system" {
                    fn GetDesktopWindow() -> HWND;
                }
                GetDesktopWindow()
            };
            worker = FindWindowExW(
                desktop_hwnd,
                worker,
                workerw_class.as_ptr(),
                std::ptr::null(),
            );
        }
    }

    None
}

/// 将图标定位到指定屏幕坐标
///
/// 委托给 `icon_manager::position_icon_at_screen`，该函数内部会：
/// 1. 临时关闭桌面自动排列（如果开启）
/// 2. 将屏幕坐标转换为 SysListView32 客户区坐标
/// 3. 通过 LVM_SETITEMPOSITION 设置图标位置
/// 4. 恢复自动排列状态
pub fn position_icon_at_screen(file_path: &str, screen_x: i32, screen_y: i32) {
    super::icon_manager::position_icon_at_screen(file_path, screen_x, screen_y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_desktop_auto_arrange_does_not_panic() {
        // 在没有桌面环境时不应 panic
        let _ = is_desktop_auto_arrange_enabled();
    }
}
