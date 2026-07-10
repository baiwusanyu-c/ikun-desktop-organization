//! Windows 系统托盘入口。

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! 创建隐藏消息窗口承载 `Shell_NotifyIconW` 回调，并把原生菜单点击转换为
//! `SystemTrayAction`，由 `app.rs` 在 egui 主循环中消费。

use crate::platform::SystemTrayAction;

use super::ffi_imports::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, GetModuleHandleW, KillTimer, PostMessageW, RegisterClassExW, SetForegroundWindow,
    SetTimer, TrackPopupMenu,
};
use super::ffi_types::{
    HWND, HWND_MESSAGE, LPARAM, LRESULT, MF_STRING, POINT, TPM_LEFTALIGN, TPM_LEFTBUTTON,
    TPM_RETURNCMD, TPM_TOPALIGN, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP, WM_TIMER,
    WNDCLASSEXW, WPARAM,
};
use anyhow::{anyhow, Context};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{LoadIconW, HICON as WinHicon, IDI_APPLICATION};

const TRAY_CLASS_NAME: &str = "IkunOrganizerTrayWindow";
const TRAY_UID: u32 = 1;
const WM_TRAY_ICON: u32 = 0x8000 + 30;
const WM_NULL: u32 = 0x0000;
const TIMER_SINGLE_CLICK: usize = 1;
const SINGLE_CLICK_DELAY_MS: u32 = 240;

const CMD_OPEN_PANEL: usize = 1001;
const CMD_CREATE_BOX: usize = 1002;
const CMD_AI_CLASSIFY: usize = 1003;
const CMD_QUIT: usize = 1004;

const ACTION_NONE: u8 = 0;
const ACTION_OPEN_PANEL: u8 = 1;
const ACTION_CREATE_BOX: u8 = 2;
const ACTION_AI_CLASSIFY: u8 = 3;
const ACTION_QUIT: u8 = 4;

static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static PENDING_ACTION: AtomicU8 = AtomicU8::new(ACTION_NONE);
static IGNORE_NEXT_LBUTTONUP: AtomicBool = AtomicBool::new(false);
type TrayActionWaker = Box<dyn Fn() + Send + Sync>;
static ACTION_WAKER: OnceLock<Mutex<Option<TrayActionWaker>>> = OnceLock::new();

/// 注册托盘动作唤醒回调。
///
/// 托盘消息来自 Win32 消息窗口。写入 `PENDING_ACTION` 后主动唤醒 egui 主循环，
/// 避免控制面板隐藏时菜单动作长时间不被消费。
pub fn register_tray_action_waker(waker: impl Fn() + Send + Sync + 'static) {
    let slot = ACTION_WAKER.get_or_init(|| Mutex::new(None));
    match slot.lock() {
        Ok(mut guard) => {
            *guard = Some(Box::new(waker));
            log::info!("系统托盘动作唤醒回调已注册");
        }
        Err(e) => {
            log::warn!("注册系统托盘动作唤醒回调失败: {}", e);
        }
    }
}

/// 初始化系统托盘图标。
///
/// 重复调用是安全的：如果托盘窗口已经存在，会直接返回成功。
pub fn setup_system_tray() -> anyhow::Result<()> {
    if TRAY_HWND.load(Ordering::SeqCst) != 0 {
        return Ok(());
    }

    let class_name = wide_null(TRAY_CLASS_NAME);
    let window_name = wide_null("Ikun Organizer Tray");

    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        if instance.is_null() {
            return Err(anyhow!("无法获取模块句柄"));
        }

        let wnd_class = WNDCLASSEXW {
            cb_size: size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfn_wnd_proc: Some(tray_wnd_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: instance,
            h_icon: ptr::null_mut(),
            h_cursor: ptr::null_mut(),
            hbr_background: ptr::null_mut(),
            lpsz_menu_name: ptr::null(),
            lpsz_class_name: class_name.as_ptr(),
            h_icon_sm: ptr::null_mut(),
        };
        RegisterClassExW(&wnd_class);

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            return Err(anyhow!("创建托盘消息窗口失败"));
        }

        if let Err(e) = add_tray_icon(hwnd) {
            DestroyWindow(hwnd);
            return Err(e).context("添加系统托盘图标失败");
        }

        TRAY_HWND.store(hwnd as isize, Ordering::SeqCst);
    }

    log::info!("系统托盘图标已创建");
    Ok(())
}

/// 清理系统托盘图标和隐藏消息窗口。
pub fn cleanup_system_tray() {
    let hwnd = TRAY_HWND.swap(0, Ordering::SeqCst) as HWND;
    if hwnd.is_null() {
        return;
    }

    unsafe {
        let data = notify_data(hwnd);
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        DestroyWindow(hwnd);
    }
    log::info!("系统托盘图标已清理");
}

/// 获取并清空一个待处理托盘动作。
pub fn take_pending_tray_action() -> Option<SystemTrayAction> {
    match PENDING_ACTION.swap(ACTION_NONE, Ordering::SeqCst) {
        ACTION_OPEN_PANEL => Some(SystemTrayAction::OpenControlPanel),
        ACTION_CREATE_BOX => Some(SystemTrayAction::CreateBox),
        ACTION_AI_CLASSIFY => Some(SystemTrayAction::AiClassify),
        ACTION_QUIT => Some(SystemTrayAction::Quit),
        _ => None,
    }
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY_ICON => {
            log::debug!("收到系统托盘鼠标消息: {:#X}", l_param);
            handle_tray_message(hwnd, l_param as u32);
            0
        }
        WM_TIMER if w_param == TIMER_SINGLE_CLICK => {
            KillTimer(hwnd, TIMER_SINGLE_CLICK);
            log::debug!("系统托盘单击延迟计时结束，打开菜单");
            show_tray_menu(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w_param, l_param),
    }
}

fn handle_tray_message(hwnd: HWND, mouse_msg: u32) {
    unsafe {
        match mouse_msg {
            WM_LBUTTONDBLCLK => {
                KillTimer(hwnd, TIMER_SINGLE_CLICK);
                IGNORE_NEXT_LBUTTONUP.store(true, Ordering::SeqCst);
                log::info!("系统托盘图标双击: 打开控制面板");
                set_pending_action(SystemTrayAction::OpenControlPanel);
            }
            WM_LBUTTONUP => {
                if IGNORE_NEXT_LBUTTONUP.swap(false, Ordering::SeqCst) {
                    log::debug!("忽略双击后的左键抬起消息");
                    return;
                }
                log::debug!("系统托盘图标单击: 准备打开菜单");
                SetTimer(
                    hwnd,
                    TIMER_SINGLE_CLICK,
                    SINGLE_CLICK_DELAY_MS,
                    ptr::null_mut(),
                );
            }
            WM_RBUTTONUP => {
                KillTimer(hwnd, TIMER_SINGLE_CLICK);
                log::debug!("系统托盘图标右键单击: 打开菜单");
                show_tray_menu(hwnd);
            }
            _ => {}
        }
    }
}

fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            log::warn!("托盘菜单创建失败");
            return;
        }

        append_menu_item(menu, CMD_OPEN_PANEL, "打开控制面板");
        append_menu_item(menu, CMD_CREATE_BOX, "创建分类框");
        append_menu_item(menu, CMD_AI_CLASSIFY, "一键AI分类");
        append_menu_item(menu, CMD_QUIT, "退出应用");

        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) == 0 {
            log::warn!("获取托盘菜单弹出位置失败");
            DestroyMenu(menu);
            return;
        }

        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_LEFTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            ptr::null(),
        );
        DestroyMenu(menu);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        log::info!("系统托盘菜单返回命令: {}", cmd);

        match cmd as usize {
            CMD_OPEN_PANEL => set_pending_action(SystemTrayAction::OpenControlPanel),
            CMD_CREATE_BOX => set_pending_action(SystemTrayAction::CreateBox),
            CMD_AI_CLASSIFY => set_pending_action(SystemTrayAction::AiClassify),
            CMD_QUIT => set_pending_action(SystemTrayAction::Quit),
            _ => {}
        }
    }
}

fn append_menu_item(menu: *mut c_void, id: usize, text: &str) {
    let label = wide_null(text);
    let ok = unsafe { AppendMenuW(menu, MF_STRING, id, label.as_ptr()) };
    if ok == 0 {
        log::warn!("添加托盘菜单项失败: {}", text);
    }
}

pub(crate) fn set_pending_action(action: SystemTrayAction) {
    log::info!("系统托盘动作已入队: {:?}", action);
    PENDING_ACTION.store(action_to_u8(action), Ordering::SeqCst);
    wake_action_consumer();
}

fn wake_action_consumer() {
    let Some(slot) = ACTION_WAKER.get() else {
        log::debug!("系统托盘动作唤醒回调尚未注册");
        return;
    };

    match slot.lock() {
        Ok(guard) => {
            if let Some(waker) = guard.as_ref() {
                waker();
            } else {
                log::debug!("系统托盘动作唤醒回调为空");
            }
        }
        Err(e) => {
            log::warn!("唤醒系统托盘动作消费者失败: {}", e);
        }
    }
}

fn action_to_u8(action: SystemTrayAction) -> u8 {
    match action {
        SystemTrayAction::OpenControlPanel => ACTION_OPEN_PANEL,
        SystemTrayAction::CreateBox => ACTION_CREATE_BOX,
        SystemTrayAction::AiClassify => ACTION_AI_CLASSIFY,
        SystemTrayAction::Quit => ACTION_QUIT,
    }
}

unsafe fn add_tray_icon(hwnd: HWND) -> anyhow::Result<()> {
    let mut data = notify_data(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY_ICON;
    // 优先使用配置的项目图标，回退到系统默认图标
    data.hIcon = super::app_icon::get_tray_icon()
        .map(|h| WinHicon(h as _))
        .unwrap_or_else(|| LoadIconW(None, IDI_APPLICATION).unwrap_or(WinHicon::default()));
    copy_wide(&mut data.szTip, "Ikun Organizer");

    if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
        return Err(anyhow!("Shell_NotifyIconW(NIM_ADD) 返回失败"));
    }
    Ok(())
}

fn notify_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: WinHwnd(hwnd),
        uID: TRAY_UID,
        ..Default::default()
    }
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide<const N: usize>(target: &mut [u16; N], text: &str) {
    let encoded: Vec<u16> = text.encode_utf16().collect();
    let len = encoded.len().min(N.saturating_sub(1));
    target[..len].copy_from_slice(&encoded[..len]);
    if len < N {
        target[len] = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex, OnceLock};

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn drain_pending_action() {
        while take_pending_tray_action().is_some() {}
    }

    #[test]
    fn test_action_to_u8_mapping() {
        assert_eq!(action_to_u8(SystemTrayAction::OpenControlPanel), 1);
        assert_eq!(action_to_u8(SystemTrayAction::CreateBox), 2);
        assert_eq!(action_to_u8(SystemTrayAction::AiClassify), 3);
        assert_eq!(action_to_u8(SystemTrayAction::Quit), 4);
    }

    #[test]
    fn test_wide_null_terminates() {
        let wide = wide_null("abc");
        assert_eq!(wide.last().copied(), Some(0));
    }

    #[test]
    fn test_copy_wide_truncates_and_terminates() {
        let mut buf = [0u16; 4];
        copy_wide(&mut buf, "abcdef");
        assert_eq!(buf[0], 'a' as u16);
        assert_eq!(buf[2], 'c' as u16);
        assert_eq!(buf[3], 0);
    }

    #[test]
    fn test_pending_action_round_trip_and_clears() {
        let _guard = test_lock();
        drain_pending_action();

        set_pending_action(SystemTrayAction::CreateBox);

        assert_eq!(
            take_pending_tray_action(),
            Some(SystemTrayAction::CreateBox)
        );
        assert_eq!(take_pending_tray_action(), None);
    }

    #[test]
    fn test_pending_action_keeps_latest_menu_command() {
        let _guard = test_lock();
        drain_pending_action();

        set_pending_action(SystemTrayAction::OpenControlPanel);
        set_pending_action(SystemTrayAction::Quit);

        assert_eq!(take_pending_tray_action(), Some(SystemTrayAction::Quit));
        assert_eq!(take_pending_tray_action(), None);
    }

    #[test]
    fn test_set_pending_action_wakes_registered_consumer() {
        let _guard = test_lock();
        drain_pending_action();
        let wake_count = Arc::new(AtomicUsize::new(0));
        let wake_count_for_callback = wake_count.clone();
        register_tray_action_waker(move || {
            wake_count_for_callback.fetch_add(1, Ordering::SeqCst);
        });

        set_pending_action(SystemTrayAction::AiClassify);

        assert_eq!(wake_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            take_pending_tray_action(),
            Some(SystemTrayAction::AiClassify)
        );
        assert_eq!(take_pending_tray_action(), None);
    }
}
