//! 覆盖区域鼠标钩子模块
//!
//! 由于覆盖窗口被设置为 HWND_BOTTOM（在 9cd8293 中修复了图标覆盖其他窗口的问题），
//! 覆盖窗口无法接收鼠标事件。此模块通过安装全局鼠标钩子（WH_MOUSE_LL）来拦截
//! 覆盖区域的右键点击，实现右键菜单功能。
//!
//! # 工作流程
//! 1. 覆盖窗口创建时调用 `install_overlay_mouse_hook()` 启动独立线程
//! 2. 线程中安装 WH_MOUSE_LL 钩子并运行消息泵（必需！）
//! 3. 钩子回调拦截全局鼠标事件
//! 4. 检查右键点击是否在覆盖区域内（且不在分类框窗口上）
//! 5. 如果是，设置原子标志
//! 6. 主循环通过 `take_pending_overlay_right_click()` 消费事件

use super::super::ffi_imports::{
    CallNextHookEx, DispatchMessageW, GetCurrentThreadId, GetMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WindowFromPoint,
};
use super::super::ffi_types::{
    HHOOK, HC_ACTION, MSG, MSLLHOOKSTRUCT, POINT, WH_MOUSE_LL, WM_QUIT, WM_RBUTTONUP,
};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::thread;

static MOUSE_HOOK: AtomicIsize = AtomicIsize::new(0);
static PENDING_OVERLAY_RIGHT_CLICK: AtomicBool = AtomicBool::new(false);
static HOOK_THREAD_RUNNING: AtomicBool = AtomicBool::new(false);
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

/// 安装覆盖区域鼠标钩子
///
/// 在覆盖窗口创建时调用，启动独立线程安装全局鼠标钩子。
/// WH_MOUSE_LL 钩子需要消息泵来分发回调，因此在独立线程中运行消息泵。
pub fn install_overlay_mouse_hook() {
    // 检查线程是否已在运行
    if HOOK_THREAD_RUNNING.load(Ordering::SeqCst) {
        log::info!("[MOUSE-HOOK] 钩子线程已在运行，跳过重复安装");
        return;
    }

    // 启动独立线程运行钩子和消息泵
    thread::spawn(|| {
        HOOK_THREAD_RUNNING.store(true, Ordering::SeqCst);
        
        // 记录线程 ID 用于后续发送 WM_QUIT
        unsafe {
            HOOK_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
        }
        log::info!("[MOUSE-HOOK] 钩子线程启动，线程ID: {}", HOOK_THREAD_ID.load(Ordering::SeqCst));

        unsafe {
            // 安装钩子
            let hook = SetWindowsHookExW(
                WH_MOUSE_LL,
                mouse_hook_proc,
                ptr::null_mut(),
                0,
            );

            if hook.is_null() {
                log::error!("[MOUSE-HOOK] 安装鼠标钩子失败");
                HOOK_THREAD_RUNNING.store(false, Ordering::SeqCst);
                return;
            }

            MOUSE_HOOK.store(hook as isize, Ordering::SeqCst);
            log::info!("[MOUSE-HOOK] 鼠标钩子安装成功，开始消息泵");

            // 运行消息泵以分发钩子回调（WH_MOUSE_LL 必需！）
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // 清理钩子
            UnhookWindowsHookEx(hook);
            MOUSE_HOOK.store(0, Ordering::SeqCst);
            log::info!("[MOUSE-HOOK] 消息泵退出，钩子已卸载");
        }

        HOOK_THREAD_RUNNING.store(false, Ordering::SeqCst);
    });
}

/// 卸载覆盖区域鼠标钩子
///
/// 在覆盖窗口销毁时调用。通过发送 WM_QUIT 让消息泵退出。
pub fn uninstall_overlay_mouse_hook() {
    if HOOK_THREAD_RUNNING.load(Ordering::SeqCst) {
        let thread_id = HOOK_THREAD_ID.load(Ordering::SeqCst);
        if thread_id != 0 {
            unsafe {
                PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
            }
            log::info!("[MOUSE-HOOK] 已发送 WM_QUIT 到钩子线程 {}", thread_id);
        }
        HOOK_THREAD_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// 查询并消费待处理的覆盖区域右键点击事件
///
/// 返回 true 表示有待处理的右键点击，应显示桌面右键菜单。
pub fn take_pending_overlay_right_click() -> bool {
    PENDING_OVERLAY_RIGHT_CLICK.swap(false, Ordering::SeqCst)
}

/// 鼠标钩子回调函数
///
/// 拦截全局鼠标事件，检查是否在覆盖区域右键点击。
unsafe extern "system" fn mouse_hook_proc(
    n_code: i32,
    w_param: usize,
    l_param: isize,
) -> isize {
    // 快速检查：只处理 WM_RBUTTONUP
    if n_code == HC_ACTION && w_param == WM_RBUTTONUP as usize {
        let ms = &*(l_param as *const MSLLHOOKSTRUCT);
        let pt = ms.pt;

        // 检查点击是否在覆盖窗口区域内
        if is_click_on_overlay(pt) {
            log::info!(
                "[MOUSE-HOOK] 检测到覆盖区域右键点击: ({}, {})",
                pt.x, pt.y
            );
            // 设置标志，主循环会处理
            PENDING_OVERLAY_RIGHT_CLICK.store(true, Ordering::SeqCst);
            // 阻止消息传递到桌面，避免桌面显示原生右键菜单
            return 1;
        }
    }

    // 传递消息到下一个钩子
    CallNextHookEx(
        MOUSE_HOOK.load(Ordering::SeqCst) as HHOOK,
        n_code,
        w_param,
        l_param,
    )
}

/// 检查点击是否在覆盖窗口区域内（且不在分类框窗口上）
unsafe fn is_click_on_overlay(pt: POINT) -> bool {
    // 获取该位置的窗口
    let hwnd_at_point = WindowFromPoint(pt);

    if hwnd_at_point.is_null() {
        return false;
    }

    // 检查是否是覆盖窗口
    if super::is_desktop_overlay_window(hwnd_at_point) {
        return true;
    }

    // 检查是否是桌面窗口（Progman 或 WorkerW）
    // 如果是桌面窗口，也认为是覆盖区域（因为覆盖窗口在桌面下方）
    if is_desktop_window(hwnd_at_point) {
        return true;
    }

    false
}

/// 检查是否是桌面窗口
unsafe fn is_desktop_window(hwnd: super::super::ffi_types::HWND) -> bool {
    use super::super::ffi_imports::GetClassNameW;

    let mut class_name = [0u16; 64];
    let len = GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32);

    if len > 0 {
        let class_str = String::from_utf16_lossy(&class_name[..len as usize]);
        // Progman 是主桌面窗口，WorkerW 是辅助桌面窗口
        class_str == "Progman" || class_str == "WorkerW"
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_take_pending_overlay_right_click_default_is_false() {
        // 消费可能残留的标志
        let _ = take_pending_overlay_right_click();
        // 默认应该返回 false
        assert!(!take_pending_overlay_right_click());
    }

    #[test]
    fn test_take_pending_overlay_right_click_consumes_flag() {
        // 设置标志
        PENDING_OVERLAY_RIGHT_CLICK.store(true, Ordering::SeqCst);
        // 第一次调用应该返回 true
        assert!(take_pending_overlay_right_click());
        // 第二次调用应该返回 false（标志已消费）
        assert!(!take_pending_overlay_right_click());
    }

    #[test]
    fn test_hook_thread_state_transitions() {
        // 初始状态应该是 false（如果之前的测试没有污染）
        // 注意：这个测试依赖于其他测试不修改全局状态
        let initial_state = HOOK_THREAD_RUNNING.load(Ordering::SeqCst);
        
        // 测试状态设置
        HOOK_THREAD_RUNNING.store(true, Ordering::SeqCst);
        assert!(HOOK_THREAD_RUNNING.load(Ordering::SeqCst));
        
        HOOK_THREAD_RUNNING.store(false, Ordering::SeqCst);
        assert!(!HOOK_THREAD_RUNNING.load(Ordering::SeqCst));
        
        // 恢复初始状态
        HOOK_THREAD_RUNNING.store(initial_state, Ordering::SeqCst);
    }

    #[test]
    fn test_mouse_hook_atomic_state() {
        // 测试 MOUSE_HOOK 的原子状态管理
        let initial_hook = MOUSE_HOOK.load(Ordering::SeqCst);
        
        MOUSE_HOOK.store(12345, Ordering::SeqCst);
        assert_eq!(MOUSE_HOOK.load(Ordering::SeqCst), 12345);
        
        MOUSE_HOOK.store(0, Ordering::SeqCst);
        assert_eq!(MOUSE_HOOK.load(Ordering::SeqCst), 0);
        
        // 恢复初始状态
        MOUSE_HOOK.store(initial_hook, Ordering::SeqCst);
    }
}