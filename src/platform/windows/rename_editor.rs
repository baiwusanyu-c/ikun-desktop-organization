//! Inline rename editor for icon labels
//!
//! Creates a top-level popup window with an EDIT control for F2-style rename.
//! This approach avoids z-order issues with layered windows.
//!
//! Behavior:
//! - Enter: confirm rename
//! - Escape: cancel
//! - Focus lost: confirm rename
//! - Text selection: filename stem (without extension)

use super::ffi_imports::*;
use super::ffi_types::*;
use std::sync::Mutex;

static EDIT_CLASS: &[u16] = &[69, 68, 73, 74, 0]; // "EDIT"

const GWLP_WNDPROC: i32 = -4;

struct RenameState {
    edit_hwnd: HWND,
    container_hwnd: HWND,
    result: Option<Option<String>>,
    orig_wnd_proc: isize,
}

unsafe impl Send for RenameState {}

static RENAME_STATE: Mutex<Option<RenameState>> = Mutex::new(None);

/// Show inline rename editor and wait for user input.
///
/// Returns `Some(new_name)` if confirmed, `None` if cancelled.
pub fn show_rename_editor(
    parent_hwnd: HWND,
    current_name: &str,
    screen_x: i32,
    screen_y: i32,
    width: i32,
    height: i32,
    _dpi: u32,
) -> Option<String> {
    unsafe {
        log::info!("Creating rename EDIT control at ({}, {}), size {}x{}", 
            screen_x, screen_y, width, height);
        
        // Create a top-level popup window
        // WS_POPUP = 0x80000000, WS_VISIBLE = 0x10000000
        // WS_EX_TOOLWINDOW = 0x00000080 (no taskbar icon)
        // WS_EX_TOPMOST = 0x00000008 (always on top)
        let ex_style = 0x00000080 | 0x00000008;
        let style = 0x80000000_u32 | 0x10000000;
        
        let static_class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let container_hwnd = CreateWindowExW(
            ex_style,
            static_class.as_ptr(),
            std::ptr::null(),
            style,
            screen_x,
            screen_y,
            width,
            height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        
        if container_hwnd.is_null() {
            log::warn!("Failed to create container window");
            return None;
        }
        
        // Create EDIT control as child of container
        // ES_AUTOHSCROLL = 0x0080, WS_CHILD = 0x40000000, WS_VISIBLE = 0x10000000
        let edit_style = 0x40000000_u32 | 0x10000000 | 0x00800000 | 0x0080;
        let edit_hwnd = CreateWindowExW(
            0x00000200, // WS_EX_CLIENTEDGE
            EDIT_CLASS.as_ptr(),
            std::ptr::null(),
            edit_style,
            0,
            0,
            width,
            height,
            container_hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );

        if edit_hwnd.is_null() {
            log::warn!("Failed to create rename EDIT control");
            DestroyWindow(container_hwnd);
            return None;
        }
        log::info!("Rename EDIT control created: {:?}", edit_hwnd);

        let orig_proc = SetWindowLongPtrW(edit_hwnd, GWLP_WNDPROC, rename_edit_subclass as *const () as isize);

        *RENAME_STATE.lock().unwrap() = Some(RenameState {
            edit_hwnd,
            container_hwnd,
            result: None,
            orig_wnd_proc: orig_proc,
        });

        // Set text
        let text_wide: Vec<u16> = current_name.encode_utf16().chain(std::iter::once(0)).collect();
        SendMessageW(edit_hwnd, 0x000C /* WM_SETTEXT */, 0, text_wide.as_ptr() as LPARAM);

        // Select filename stem (without extension)
        let stem_len = get_name_stem_len(current_name);
        SendMessageW(edit_hwnd, 0x00B1 /* EM_SETSEL */, 0, stem_len as LPARAM);

        SetFocus(edit_hwnd);
        SetForegroundWindow(container_hwnd);

        // Message loop
        let mut msg: MSG = std::mem::zeroed();
        loop {
            if GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) == 0 {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);

            let state = RENAME_STATE.lock().unwrap();
            if state.as_ref().map(|s| s.result.is_some()).unwrap_or(true) {
                break;
            }
        }

        let mut state = RENAME_STATE.lock().unwrap();
        let result = state.as_ref().and_then(|s| s.result.clone()).flatten();

        if let Some(ref s) = *state {
            if IsWindow(s.edit_hwnd) != 0 {
                DestroyWindow(s.edit_hwnd);
            }
            if IsWindow(s.container_hwnd) != 0 {
                DestroyWindow(s.container_hwnd);
            }
        }
        *state = None;

        SetForegroundWindow(parent_hwnd);
        result
    }
}

fn get_name_stem_len(name: &str) -> i32 {
    let path = std::path::Path::new(name);
    if let Some(stem) = path.file_stem() {
        stem.to_string_lossy().encode_utf16().count() as i32
    } else {
        name.encode_utf16().count() as i32
    }
}

unsafe fn get_edit_text(hwnd: HWND) -> String {
    let len = SendMessageW(hwnd, 0x000E /* WM_GETTEXTLENGTH */, 0, 0) as i32;
    if len <= 0 {
        return String::new();
    }
    let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
    SendMessageW(hwnd, 0x000D /* WM_GETTEXT */, buf.len() as WPARAM, buf.as_mut_ptr() as LPARAM);
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn destroy_container() {
    if let Ok(mut state) = RENAME_STATE.lock() {
        if let Some(ref s) = *state {
            unsafe {
                if IsWindow(s.container_hwnd) != 0 {
                    DestroyWindow(s.container_hwnd);
                }
            }
        }
    }
}

unsafe extern "system" fn rename_edit_subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let orig_proc = RENAME_STATE
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.orig_wnd_proc)
        .unwrap_or(0);

    if orig_proc == 0 {
        return 0;
    }

    match msg {
        0x0100 /* WM_KEYDOWN */ => {
            match wparam {
                0x0D /* VK_RETURN */ => {
                    let text = get_edit_text(hwnd);
                    log::info!("Rename confirmed with: {}", text);
                    if let Ok(mut state) = RENAME_STATE.lock() {
                        if let Some(ref mut s) = *state {
                            s.result = Some(Some(text));
                        }
                    }
                    destroy_container();
                    return 0;
                }
                0x1B /* VK_ESCAPE */ => {
                    log::info!("Rename cancelled");
                    if let Ok(mut state) = RENAME_STATE.lock() {
                        if let Some(ref mut s) = *state {
                            s.result = Some(None);
                        }
                    }
                    destroy_container();
                    return 0;
                }
                _ => {}
            }
        }
        0x0008 /* WM_KILLFOCUS */ => {
            let text = get_edit_text(hwnd);
            if !text.is_empty() {
                log::info!("Rename lost focus, confirming with: {}", text);
                if let Ok(mut state) = RENAME_STATE.lock() {
                    if let Some(ref mut s) = *state {
                        if s.result.is_none() {
                            s.result = Some(Some(text));
                        }
                    }
                }
            }
            destroy_container();
            return 0;
        }
        _ => {}
    }

    type WndProcFn = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
    let proc_fn: WndProcFn = std::mem::transmute(orig_proc);
    proc_fn(hwnd, msg, wparam, lparam)
}
