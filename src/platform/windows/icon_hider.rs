//! 隐藏/恢复原生桌面图标
//!
//! 通过 ShowWindow(SW_HIDE) 隐藏整个 SysListView32 窗口，
//! 原子操作，无闪烁。退出时 ShowWindow(SW_SHOW) 恢复。

use anyhow::Result;
use std::ffi::c_void;
use std::sync::Mutex;

// ===== Raw FFI =====
type HWND = *mut c_void;

const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;

extern "system" {
    fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> HWND;
    fn FindWindowExW(
        hWndParent: HWND,
        hWndChildAfter: HWND,
        lpClassName: *const u16,
        lpWindowName: *const u16,
    ) -> HWND;
    fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> i32;
    fn IsWindow(hWnd: HWND) -> i32;
}

static PROG_CLASS: &[u16] = &[80, 114, 111, 103, 109, 97, 110, 0]; // "Progman"
static WORKER_CLASS: &[u16] = &[87, 111, 114, 107, 101, 114, 87, 0]; // "WorkerW"
static DEFVIEW_CLASS: &[u16] = &[
    83, 72, 69, 76, 76, 68, 76, 76, 95, 68, 101, 102, 86, 105, 101, 119, 0,
]; // "SHELLDLL_DefView"
static LISTVIEW_CLASS: &[u16] = &[
    83, 121, 115, 76, 105, 115, 116, 86, 105, 101, 119, 51, 50, 0,
]; // "SysListView32"

/// 查找桌面 SysListView32 句柄
fn find_desktop_listview() -> Result<HWND> {
    unsafe {
        // 尝试 Progman
        let progman = FindWindowW(PROG_CLASS.as_ptr(), std::ptr::null());
        if !progman.is_null() {
            let defview = FindWindowExW(
                progman,
                std::ptr::null_mut(),
                DEFVIEW_CLASS.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    LISTVIEW_CLASS.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Ok(lv);
                }
            }
        }

        // 尝试 WorkerW
        let mut worker = FindWindowW(WORKER_CLASS.as_ptr(), std::ptr::null());
        while !worker.is_null() {
            let defview = FindWindowExW(
                worker,
                std::ptr::null_mut(),
                DEFVIEW_CLASS.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    LISTVIEW_CLASS.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Ok(lv);
                }
            }
            worker = FindWindowExW(
                std::ptr::null_mut(),
                worker,
                WORKER_CLASS.as_ptr(),
                std::ptr::null(),
            );
        }

        anyhow::bail!("无法找到 SysListView32")
    }
}

/// 图标隐藏器全局状态
struct IconHider {
    listview_hwnd: HWND,
    hidden: bool,
}

// Safety: HWND 是 Send 的（只在同一线程使用）
unsafe impl Send for IconHider {}

static ICON_HIDER: Mutex<Option<IconHider>> = Mutex::new(None);

/// 隐藏所有原生桌面图标
///
/// 通过 ShowWindow(SW_HIDE) 隐藏整个 SysListView32 窗口，
/// 原子操作，不会产生闪烁。
pub fn hide_native_icons() -> Result<()> {
    let lv_hwnd = find_desktop_listview()?;

    unsafe {
        ShowWindow(lv_hwnd, SW_HIDE);
    }

    log::info!("已隐藏桌面 SysListView32 窗口");

    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;
    *guard = Some(IconHider {
        listview_hwnd: lv_hwnd,
        hidden: true,
    });

    Ok(())
}

/// 恢复原生桌面图标
///
/// 通过 ShowWindow(SW_SHOW) 恢复 SysListView32 窗口。
pub fn restore_native_icons() -> Result<()> {
    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;

    if let Some(hider) = guard.take() {
        if !hider.hidden {
            return Ok(());
        }

        // 确认窗口仍然有效
        let valid = unsafe { IsWindow(hider.listview_hwnd) };
        if valid != 0 {
            unsafe {
                ShowWindow(hider.listview_hwnd, SW_SHOW);
            }
            log::info!("已恢复桌面 SysListView32 窗口");
        } else {
            log::warn!("SysListView32 窗口已失效，跳过恢复");
        }
    }

    Ok(())
}
