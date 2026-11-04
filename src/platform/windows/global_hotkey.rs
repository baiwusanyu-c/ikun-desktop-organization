//! Windows 全局热键模块。
//!
//! 创建隐藏消息窗口接收 `WM_HOTKEY` 消息，将快捷键字符串（如 "Ctrl+Alt+D"）
//! 解析为 Win32 修饰符和虚拟键码，通过 `RegisterHotKey` 注册全局热键。
//!
//! # 工作流程
//! 1. 调用 `setup_global_hotkey(hotkey_str)` 注册热键
//! 2. 用户按键时，消息窗口收到 `WM_HOTKEY`
//! 3. `HOTKEY_TOGGLE_PENDING` 原子变量被设置
//! 4. 主循环通过 `take_hotkey_toggle()` 消费事件

use super::ffi_imports::{
    CreateWindowExW, DefWindowProcW, GetModuleHandleW, RegisterClassExW, RegisterHotKey,
    UnregisterHotKey,
};
use super::ffi_types::{
    HWND, HWND_MESSAGE, LPARAM, LRESULT, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    WM_HOTKEY, WNDCLASSEXW, WPARAM,
};
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

const HOTKEY_CLASS_NAME: &str = "IkunOrganizerHotkeyWindow";
const HOTKEY_ID_TOGGLE_PANEL: i32 = 1;

static HOTKEY_HWND: AtomicIsize = AtomicIsize::new(0);
static HOTKEY_TOGGLE_PENDING: AtomicBool = AtomicBool::new(false);
static CURRENT_HOTKEY_ID: AtomicIsize = AtomicIsize::new(0);

/// 热键解析结果
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedHotkey {
    modifiers: u32,
    vk: u32,
}

/// 解析热键字符串为 Win32 修饰符和虚拟键码。
///
/// 支持格式："Ctrl+Alt+D"、"Ctrl+Shift+F1"、"Alt+Space" 等。
/// 修饰符不区分大小写，键名支持字母、数字、F1-F12 和部分特殊键。
fn parse_hotkey(hotkey_str: &str) -> Option<ParsedHotkey> {
    let parts: Vec<&str> = hotkey_str.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers: u32 = MOD_NOREPEAT;
    let mut key_part = None;

    for part in &parts {
        let lower = part.to_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "alt" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "super" => modifiers |= MOD_WIN,
            _ => {
                key_part = Some(*part);
            }
        }
    }

    let key = key_part?;
    let vk = parse_virtual_key(key)?;

    Some(ParsedHotkey { modifiers, vk })
}

/// 将键名转换为 Win32 虚拟键码。
fn parse_virtual_key(key: &str) -> Option<u32> {
    let upper = key.to_uppercase();
    match upper.as_str() {
        s if s.len() == 1 && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) => {
            Some(s.as_bytes()[0] as u32)
        }
        s if s.len() == 1 && s.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
            Some(s.as_bytes()[0] as u32)
        }
        "F1" => Some(0x70),
        "F2" => Some(0x71),
        "F3" => Some(0x72),
        "F4" => Some(0x73),
        "F5" => Some(0x74),
        "F6" => Some(0x75),
        "F7" => Some(0x76),
        "F8" => Some(0x77),
        "F9" => Some(0x78),
        "F10" => Some(0x79),
        "F11" => Some(0x7A),
        "F12" => Some(0x7B),
        "SPACE" => Some(0x20),
        "ENTER" | "RETURN" => Some(0x0D),
        "TAB" => Some(0x09),
        "ESC" | "ESCAPE" => Some(0x1B),
        "BACKSPACE" => Some(0x08),
        "DELETE" | "DEL" => Some(0x2E),
        "INSERT" | "INS" => Some(0x2D),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "PAGEUP" | "PGUP" => Some(0x21),
        "PAGEDOWN" | "PGDN" => Some(0x22),
        "UP" => Some(0x26),
        "DOWN" => Some(0x28),
        "LEFT" => Some(0x25),
        "RIGHT" => Some(0x27),
        _ => None,
    }
}

/// 注册全局热键。
///
/// 如果之前已注册过热键，会先取消注册再重新注册。
/// 热键字符串格式："Ctrl+Alt+D"（修饰符+键名，用 + 分隔）。
pub fn setup_global_hotkey(hotkey_str: &str) -> anyhow::Result<()> {
    let parsed = parse_hotkey(hotkey_str)
        .ok_or_else(|| anyhow::anyhow!("无法解析热键字符串: {}", hotkey_str))?;

    // 确保消息窗口已创建
    ensure_hotkey_window()?;

    let hwnd = HOTKEY_HWND.load(Ordering::SeqCst);
    if hwnd == 0 {
        anyhow::bail!("热键消息窗口未创建");
    }

    // 先取消之前的注册
    let old_id = CURRENT_HOTKEY_ID.load(Ordering::SeqCst);
    if old_id != 0 {
        unsafe {
            UnregisterHotKey(hwnd as HWND, old_id as i32);
        }
        CURRENT_HOTKEY_ID.store(0, Ordering::SeqCst);
    }

    // 注册新的全局热键
    let result = unsafe {
        RegisterHotKey(
            hwnd as HWND,
            HOTKEY_ID_TOGGLE_PANEL,
            parsed.modifiers,
            parsed.vk,
        )
    };

    if result == 0 {
        anyhow::bail!(
            "RegisterHotKey 失败: 热键 {} 可能已被其他程序占用",
            hotkey_str
        );
    }

    CURRENT_HOTKEY_ID.store(HOTKEY_ID_TOGGLE_PANEL as isize, Ordering::SeqCst);
    log::info!(
        "全局热键已注册: {} (modifiers=0x{:X}, vk=0x{:X})",
        hotkey_str,
        parsed.modifiers,
        parsed.vk
    );
    Ok(())
}

/// 消费待处理的热键切换事件。
///
/// 返回 `true` 表示用户按下了切换面板快捷键。
pub fn take_hotkey_toggle() -> bool {
    HOTKEY_TOGGLE_PENDING.swap(false, Ordering::SeqCst)
}

/// 确保热键消息窗口已创建。
fn ensure_hotkey_window() -> anyhow::Result<()> {
    if HOTKEY_HWND.load(Ordering::SeqCst) != 0 {
        return Ok(());
    }

    let class_name = wide_null(HOTKEY_CLASS_NAME);
    let window_name = wide_null("Ikun Organizer Hotkey");

    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        if instance.is_null() {
            return Err(anyhow::anyhow!("无法获取模块句柄"));
        }

        let wnd_class = WNDCLASSEXW {
            cb_size: size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfn_wnd_proc: Some(hotkey_wnd_proc),
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
            return Err(anyhow::anyhow!("创建热键消息窗口失败"));
        }

        HOTKEY_HWND.store(hwnd as isize, Ordering::SeqCst);
    }

    log::info!("热键消息窗口已创建");
    Ok(())
}

/// 热键消息窗口回调。
extern "system" fn hotkey_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_HOTKEY {
        let hotkey_id = wparam as i32;
        if hotkey_id == HOTKEY_ID_TOGGLE_PANEL {
            HOTKEY_TOGGLE_PENDING.store(true, Ordering::SeqCst);
            log::debug!("收到全局热键事件: toggle_panel");
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hotkey_ctrl_alt_d() {
        let parsed = parse_hotkey("Ctrl+Alt+D").unwrap();
        assert_eq!(parsed.modifiers, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT);
        assert_eq!(parsed.vk, b'D' as u32);
    }

    #[test]
    fn test_parse_hotkey_ctrl_shift_f1() {
        let parsed = parse_hotkey("Ctrl+Shift+F1").unwrap();
        assert_eq!(parsed.modifiers, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT);
        assert_eq!(parsed.vk, 0x70);
    }

    #[test]
    fn test_parse_hotkey_case_insensitive() {
        let parsed = parse_hotkey("ctrl+alt+d").unwrap();
        assert_eq!(parsed.modifiers, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT);
        assert_eq!(parsed.vk, b'D' as u32);
    }

    #[test]
    fn test_parse_hotkey_with_spaces() {
        let parsed = parse_hotkey("Ctrl + Alt + D").unwrap();
        assert_eq!(parsed.modifiers, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT);
        assert_eq!(parsed.vk, b'D' as u32);
    }

    #[test]
    fn test_parse_hotkey_invalid() {
        assert!(parse_hotkey("").is_none());
        assert!(parse_hotkey("Ctrl").is_none());
        assert!(parse_hotkey("Ctrl+InvalidKey").is_none());
    }

    #[test]
    fn test_parse_virtual_key_letters() {
        assert_eq!(parse_virtual_key("A"), Some(b'A' as u32));
        assert_eq!(parse_virtual_key("z"), Some(b'Z' as u32));
    }

    #[test]
    fn test_parse_virtual_key_digits() {
        assert_eq!(parse_virtual_key("0"), Some(b'0' as u32));
        assert_eq!(parse_virtual_key("9"), Some(b'9' as u32));
    }

    #[test]
    fn test_parse_virtual_key_function_keys() {
        assert_eq!(parse_virtual_key("F1"), Some(0x70));
        assert_eq!(parse_virtual_key("F12"), Some(0x7B));
    }

    #[test]
    fn test_parse_virtual_key_special_keys() {
        assert_eq!(parse_virtual_key("Space"), Some(0x20));
        assert_eq!(parse_virtual_key("Enter"), Some(0x0D));
        assert_eq!(parse_virtual_key("Escape"), Some(0x1B));
        assert_eq!(parse_virtual_key("Tab"), Some(0x09));
    }

    #[test]
    fn test_take_hotkey_toggle_default_false() {
        HOTKEY_TOGGLE_PENDING.store(false, Ordering::SeqCst);
        assert!(!take_hotkey_toggle());
    }

    #[test]
    fn test_take_hotkey_toggle_consumes() {
        HOTKEY_TOGGLE_PENDING.store(true, Ordering::SeqCst);
        assert!(take_hotkey_toggle());
        assert!(!take_hotkey_toggle());
    }
}
