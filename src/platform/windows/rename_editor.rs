//! 使用原生 Windows EDIT 控件实现内联重命名
//!
//! 在独立线程上创建 EDIT 控件并运行标准消息循环。
//! 这样即使 GPUI 的消息循环不分发，EDIT 控件也能正常接收键盘输入。

use super::ffi_imports::*;

extern "system" { fn PostQuitMessage(nExitCode: i32); }
use super::ffi_types::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

const WM_USER_RENAME_RESULT: u32 = 0x8100;
const WM_USER_RENAME_CANCEL: u32 = 0x8102;
const WM_USER_QUIT_EDIT: u32 = 0x8101;

struct RenameState {
    edit_hwnd: HWND,
    container_hwnd: HWND,
    parent_hwnd: HWND,
    item_index: i32,
    original_name: String,
    font_handle: HGDIOBJ,
}

unsafe impl Send for RenameState {}

static RENAME_STATE: Mutex<Option<RenameState>> = Mutex::new(None);
static EDIT_THREAD_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 确保 EDIT 窗口类已注册
fn ensure_edit_class() {
    use std::sync::atomic::AtomicBool as AB;
    static DONE: AB = AB::new(false);
    if DONE.swap(true, Ordering::SeqCst) { return; }
    
    unsafe {
        let icex = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: 0x4000, // ICC_STANDARD_CLASSES
        };
        InitCommonControlsEx(&icex);
        
        extern "system" { fn LoadLibraryW(name: *const u16) -> HINSTANCE; }
        let dll: Vec<u16> = "comctl32.dll\0".encode_utf16().collect();
        let hmod = LoadLibraryW(dll.as_ptr());
        if !hmod.is_null() {
            InitCommonControlsEx(&icex);
        }
        log::info!("[rename] Edit class ensured");
    }
}

struct ThreadParams {
    parent_hwnd: HWND,
    current_name: String,
    screen_x: i32,
    screen_y: i32,
    width: i32,
    height: i32,
    item_index: i32,
}

unsafe impl Send for ThreadParams {}

/// 创建内联重命名编辑器（非阻塞，在独立线程上运行）
pub fn show_rename_editor_async(
    parent_hwnd: HWND,
    current_name: &str,
    screen_x: i32,
    screen_y: i32,
    width: i32,
    height: i32,
    item_index: i32,
) -> bool {
    if EDIT_THREAD_ACTIVE.swap(true, Ordering::SeqCst) {
        log::warn!("[rename] Thread already active, skipping");
        return false;
    }
    
    ensure_edit_class();
    
    let params = ThreadParams {
        parent_hwnd,
        current_name: current_name.to_string(),
        screen_x, screen_y, width, height,
        item_index,
    };
    
    log::info!("[rename] Spawning edit thread for '{}'", current_name);
    
    std::thread::spawn(move || {
        unsafe { edit_thread_main(params); }
    });
    
    true
}

unsafe fn edit_thread_main(params: ThreadParams) {
    let ThreadParams { parent_hwnd, current_name, screen_x, screen_y, width, height, item_index } = params;
    
    // 创建容器窗口
    let ex_style = 0x00000080 | 0x00000008; // WS_EX_TOOLWINDOW | WS_EX_TOPMOST
    let style = 0x80000000_u32 | 0x10000000; // WS_POPUP | WS_VISIBLE
    
    let static_class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let container_hwnd = CreateWindowExW(
        ex_style,
        static_class.as_ptr(),
        std::ptr::null(),
        style,
        screen_x, screen_y, width, height,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    
    if container_hwnd.is_null() {
        log::error!("[rename] Failed to create container window in thread");
        EDIT_THREAD_ACTIVE.store(false, Ordering::SeqCst);
        return;
    }
    
    // 创建 EDIT 子控件
    let edit_ex_style = 0x00000200; // WS_EX_CLIENTEDGE
    let edit_style = 0x40000000_u32 | 0x10000000 | 0x00800000 | 0x0080;
    // WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL
    
    let edit_name: Vec<u16> = "Edit\0".encode_utf16().collect();
    let edit_hwnd = CreateWindowExW(
        edit_ex_style,
        edit_name.as_ptr(),
        std::ptr::null(),
        edit_style,
        0, 0, width, height,
        container_hwnd,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    
    if edit_hwnd.is_null() {
        log::error!("[rename] Failed to create EDIT control in thread");
        DestroyWindow(container_hwnd);
        EDIT_THREAD_ACTIVE.store(false, Ordering::SeqCst);
        return;
    }
    
    log::info!("[rename] Thread: Container={:?}, EDIT={:?}", container_hwnd, edit_hwnd);
    
    // 设置字体（与图标标签字体一致：Microsoft YaHei UI，13px at 96 DPI）
    let dpi = GetDpiForWindow(parent_hwnd);
    let dpi = if dpi > 0 { dpi } else { 96 };
    let font_height = -scale_for_dpi(13, dpi);
    let font_name_w: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
    let font_handle = CreateFontW(
        font_height, 0, 0, 0,
        400,  // FW_NORMAL
        0, 0, 0,
        1,    // DEFAULT_CHARSET
        0, 5, 0,  // OUT_TT_PRECIS, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS
        0x02 | 0x20,  // FF_SWISS | VARIABLE_PITCH
        font_name_w.as_ptr(),
    );
    if !font_handle.is_null() {
        // WM_SETFONT = 0x0030, wParam = HFONT, lParam = TRUE (redraw)
        SendMessageW(edit_hwnd, 0x0030, font_handle as WPARAM, 1);
        log::info!("[rename] Font set: height={}, dpi={}", font_height, dpi);
    } else {
        log::warn!("[rename] Failed to create font");
    }
    
    // 保存状态
    *RENAME_STATE.lock().unwrap() = Some(RenameState {
        edit_hwnd,
        container_hwnd,
        parent_hwnd,
        item_index,
        original_name: current_name.clone(),
        font_handle,
    });
    
    // 设置文本
    let text_wide: Vec<u16> = current_name.encode_utf16().chain(std::iter::once(0)).collect();
    SendMessageW(edit_hwnd, 0x000C /* WM_SETTEXT */, 0, text_wide.as_ptr() as LPARAM);
    
    // 选中文件名主体
    let stem_len = get_name_stem_len(&current_name);
    SendMessageW(edit_hwnd, 0x00B1 /* EM_SETSEL */, 0, stem_len as LPARAM);
    
    // 子类化 EDIT 控件来拦截 Enter/Escape
    let orig_proc = SetWindowLongPtrW(edit_hwnd, -4 /* GWLP_WNDPROC */, edit_subclass as *const () as isize);
    
    // 将原始窗口过程存储在额外数据中（用全局变量）
    EDIT_ORIG_PROC.store(orig_proc, Ordering::SeqCst);
    
    // 设置焦点
    SetForegroundWindow(container_hwnd);
    SetFocus(edit_hwnd);
    
    log::info!("[rename] Thread: EDIT ready, entering message loop");
    
    // 运行标准消息循环
    let mut msg: MSG = std::mem::zeroed();
    loop {
        let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
        if ret == 0 || ret == -1 {
            break;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
        
        // 检查是否应该退出
        if !EDIT_THREAD_ACTIVE.load(Ordering::SeqCst) {
            break;
        }
    }
    
    // 清理
    cleanup_and_notify(parent_hwnd, item_index, edit_hwnd, container_hwnd);
    EDIT_THREAD_ACTIVE.store(false, Ordering::SeqCst);
    log::info!("[rename] Thread: message loop exited");
}

static EDIT_ORIG_PROC: AtomicIsize = AtomicIsize::new(0);
static EDIT_RESULT: Mutex<Option<Option<String>>> = Mutex::new(None);

unsafe extern "system" fn edit_subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let orig = EDIT_ORIG_PROC.load(Ordering::SeqCst) as isize;
    if orig == 0 { return 0; }
    
    match msg {
        0x0100 /* WM_KEYDOWN */ => {
            match wparam {
                0x0D /* VK_RETURN */ => {
                    log::info!("[rename] subclass: Enter pressed");
                    EDIT_THREAD_ACTIVE.store(false, Ordering::SeqCst);
                    // 让消息循环退出
                    PostQuitMessage(0);
                    return 0;
                }
                0x1B /* VK_ESCAPE */ => {
                    log::info!("[rename] subclass: Escape pressed");
                    // 标记取消
                    *EDIT_RESULT.lock().unwrap() = Some(None);
                    EDIT_THREAD_ACTIVE.store(false, Ordering::SeqCst);
                    PostQuitMessage(0);
                    return 0;
                }
                _ => {}
            }
        }
        0x0008 /* WM_KILLFOCUS */ => {
            log::info!("[rename] subclass: WM_KILLFOCUS");
            // 延迟退出，让用户有时间点击回来
            // 使用 PostMessage 延迟处理
            PostMessageW(hwnd, WM_USER_QUIT_EDIT, 0, 0);
        }
        WM_USER_QUIT_EDIT => {
            // 检查焦点是否还在 EDIT 上
            let focus = GetFocus();
            if focus != hwnd {
                log::info!("[rename] subclass: focus still lost, exiting");
                EDIT_THREAD_ACTIVE.store(false, Ordering::SeqCst);
                PostQuitMessage(0);
                return 0;
            }
        }
        _ => {}
    }
    
    type WndProcFn = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
    let proc_fn: WndProcFn = std::mem::transmute(orig);
    proc_fn(hwnd, msg, wparam, lparam)
}

unsafe fn cleanup_and_notify(parent_hwnd: HWND, item_index: i32, edit_hwnd: HWND, container_hwnd: HWND) {
    // 获取最终文本
    let text = {
        let len = SendMessageW(edit_hwnd, 0x000E, 0, 0) as i32;
        if len > 0 {
            let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
            SendMessageW(edit_hwnd, 0x000D, buf.len() as WPARAM, buf.as_mut_ptr() as LPARAM);
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..end])
        } else {
            String::new()
        }
    };
    
    log::info!("[rename] Final text: '{}'", text);
    
    // 检查是否是取消
    let cancelled = EDIT_RESULT.lock().unwrap().take()
        .map(|r| r.is_none())
        .unwrap_or(false);
    
    // 销毁窗口
    if IsWindow(container_hwnd) != 0 {
        DestroyWindow(container_hwnd);
    }
    
    // 清理状态和字体资源
    if let Some(state) = RENAME_STATE.lock().unwrap().take() {
        if !state.font_handle.is_null() {
            DeleteObject(state.font_handle);
            log::info!("[rename] Font deleted");
        }
    }
    
    // 发送结果给父窗口
    if !cancelled && !text.is_empty() && item_index >= 0 {
        let boxed = Box::new(text);
        let ptr = Box::into_raw(boxed);
        PostMessageW(parent_hwnd, WM_USER_RENAME_RESULT, item_index as WPARAM, ptr as LPARAM);
    } else if item_index >= 0 {
        // 重命名取消或文本为空，通知父窗口清除重命名状态
        PostMessageW(parent_hwnd, WM_USER_RENAME_CANCEL, item_index as WPARAM, 0);
    }
}



pub fn get_rename_result_message() -> u32 {
    WM_USER_RENAME_RESULT
}

pub fn get_rename_cancel_message() -> u32 {
    WM_USER_RENAME_CANCEL
}

fn get_name_stem_len(name: &str) -> i32 {
    let path = std::path::Path::new(name);
    if let Some(stem) = path.file_stem() {
        stem.to_string_lossy().encode_utf16().count() as i32
    } else {
        name.encode_utf16().count() as i32
    }
}
