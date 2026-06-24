//! 隐藏/恢复原生桌面图标
//!
//! 通过 `SetWindowPos` 将 SysListView32 窗口移到屏幕外来隐藏图标，
//! 替代 `ShowWindow(SW_HIDE)` 方案。
//!
//! # 架构决策
//!
//! **不使用 `ShowWindow(SW_HIDE)`**：Windows 10/11 下 `SW_HIDE` 会导致
//! SysListView32 无法被 `FindWindowExW` 找到，从而使 `DesktopIconManager`
//! 初始化失败（拖拽到桌面时无法恢复图标）。
//!
//! **使用 `SetWindowPos` 移到屏幕外**：窗口仍然存在于窗口树中，
//! `FindWindowExW` 可以正常找到，LVM_* 消息正常工作，
//! 但图标在视觉上不可见。

use anyhow::Result;
use std::ffi::c_void;
use std::sync::Mutex;

// ===== Raw FFI =====
type HWND = *mut c_void;

extern "system" {
    fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> HWND;
    fn FindWindowExW(
        hWndParent: HWND,
        hWndChildAfter: HWND,
        lpClassName: *const u16,
        lpWindowName: *const u16,
    ) -> HWND;
    fn SetWindowPos(
        hWnd: HWND,
        hWndInsertAfter: HWND,
        X: i32,
        Y: i32,
        cx: i32,
        cy: i32,
        uFlags: u32,
    ) -> i32;
    fn GetWindowRect(hWnd: HWND, lpRect: *mut Rect) -> i32;
    fn IsWindow(hWnd: HWND) -> i32;
}

/// 简化的 RECT 结构（仅用于 GetWindowRect）
#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

// SetWindowPos 标志
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;

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
    /// 隐藏前的原始窗口位置（用于恢复）
    original_rect: Rect,
    hidden: bool,
}

// Safety: HWND 是 Send 的（只在同一线程使用）
unsafe impl Send for IconHider {}

static ICON_HIDER: Mutex<Option<IconHider>> = Mutex::new(None);

/// 隐藏所有原生桌面图标
///
/// 通过 `SetWindowPos` 将 SysListView32 移到屏幕外（-32000, -32000），
/// 保持窗口在窗口树中可被 `FindWindowExW` 找到。
///
/// 不使用 `ShowWindow(SW_HIDE)`，因为 Windows 10/11 下会导致
/// SysListView32 无法被后续找到（影响 DesktopIconManager 初始化）。
pub fn hide_native_icons() -> Result<()> {
    let lv_hwnd = find_desktop_listview()?;

    // ★ 关键：在隐藏 SysListView32 前保存 HWND 到全局共享位置
    // 隐藏后 FindWindowExW 将无法找到 SysListView32，
    // DesktopIconManager 需要使用此缓存的 HWND
    super::ffi_types::SHARED_LISTVIEW_HWND.store(
        lv_hwnd as isize,
        std::sync::atomic::Ordering::SeqCst,
    );
    log::info!("SysListView32 HWND 已缓存: {:?}", lv_hwnd);

    // 预先初始化 DesktopIconManager（使用缓存的 HWND）
    super::drag_handler::init_icon_manager_early();

    // 保存原始位置（用于恢复）
    let mut original_rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let got_rect = unsafe { GetWindowRect(lv_hwnd, &mut original_rect) };
    if got_rect == 0 {
        anyhow::bail!("GetWindowRect 失败");
    }

    let width = original_rect.right - original_rect.left;
    let height = original_rect.bottom - original_rect.top;

    // 移到屏幕外（-32000 是 Windows 隐藏窗口的标准坐标）
    let result = unsafe {
        SetWindowPos(
            lv_hwnd,
            std::ptr::null_mut(),
            -32000, -32000,
            width, height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };

    if result == 0 {
        anyhow::bail!("SetWindowPos 隐藏 SysListView32 失败");
    }

    log::info!(
        "已将桌面 SysListView32 移到屏幕外 (原始位置: {},{},{},{})",
        original_rect.left, original_rect.top, original_rect.right, original_rect.bottom
    );

    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;
    *guard = Some(IconHider {
        listview_hwnd: lv_hwnd,
        original_rect,
        hidden: true,
    });

    Ok(())
}

/// 恢复原生桌面图标
///
/// 将 SysListView32 移回到原始位置。
pub fn restore_native_icons() -> Result<()> {
    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;

    if let Some(hider) = guard.take() {
        if !hider.hidden {
            return Ok(());
        }

        let valid = unsafe { IsWindow(hider.listview_hwnd) };
        if valid != 0 {
            let width = hider.original_rect.right - hider.original_rect.left;
            let height = hider.original_rect.bottom - hider.original_rect.top;

            unsafe {
                SetWindowPos(
                    hider.listview_hwnd,
                    std::ptr::null_mut(),
                    hider.original_rect.left,
                    hider.original_rect.top,
                    width, height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            log::info!(
                "已恢复桌面 SysListView32 到原始位置 ({},{})",
                hider.original_rect.left, hider.original_rect.top
            );
        } else {
            log::warn!("SysListView32 窗口已失效，跳过恢复");
        }
    }

    Ok(())
}
