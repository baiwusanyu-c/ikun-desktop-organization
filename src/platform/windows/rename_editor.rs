//! 使用原生 Windows EDIT 控件实现内联重命名
//!
//! 在独立线程上创建 EDIT 控件并运行标准消息循环。
//! 这样即使 GPUI 的消息循环不分发，EDIT 控件也能正常接收键盘输入。

use super::ffi_imports::*;

extern "system" {
    fn PostQuitMessage(nExitCode: i32);
}
use super::ffi_types::*;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;

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
    if DONE.swap(true, Ordering::SeqCst) {
        return;
    }

    unsafe {
        let icex = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: 0x4000, // ICC_STANDARD_CLASSES
        };
        InitCommonControlsEx(&icex);

        extern "system" {
            fn LoadLibraryW(name: *const u16) -> HINSTANCE;
        }
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
        screen_x,
        screen_y,
        width,
        height,
        item_index,
    };

    log::info!("[rename] Spawning edit thread for '{}'", current_name);

    std::thread::spawn(move || unsafe {
        edit_thread_main(params);
    });

    true
}

unsafe fn edit_thread_main(params: ThreadParams) {
    let ThreadParams {
        parent_hwnd,
        current_name,
        screen_x,
        screen_y,
        width,
        height,
        item_index,
    } = params;

    // 创建容器窗口
    let ex_style = 0x00000080 | 0x00000008; // WS_EX_TOOLWINDOW | WS_EX_TOPMOST
    let style = 0x80000000_u32 | 0x10000000; // WS_POPUP | WS_VISIBLE

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
        log::error!("[rename] Failed to create EDIT control in thread");
        DestroyWindow(container_hwnd);
        EDIT_THREAD_ACTIVE.store(false, Ordering::SeqCst);
        return;
    }

    log::info!(
        "[rename] Thread: Container={:?}, EDIT={:?}",
        container_hwnd,
        edit_hwnd
    );

    // 设置字体（与图标标签字体一致：Microsoft YaHei UI，13px at 96 DPI）
    let dpi = GetDpiForWindow(parent_hwnd);
    let dpi = if dpi > 0 { dpi } else { 96 };
    let font_height = -scale_for_dpi(13, dpi);
    let font_name_w: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
    let font_handle = CreateFontW(
        font_height,
        0,
        0,
        0,
        400, // FW_NORMAL
        0,
        0,
        0,
        1, // DEFAULT_CHARSET
        0,
        5,
        0,           // OUT_TT_PRECIS, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS
        0x02 | 0x20, // FF_SWISS | VARIABLE_PITCH
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
    let text_wide: Vec<u16> = current_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    SendMessageW(
        edit_hwnd,
        0x000C, /* WM_SETTEXT */
        0,
        text_wide.as_ptr() as LPARAM,
    );

    // 选中文件名主体
    let stem_len = get_name_stem_len(&current_name);
    SendMessageW(
        edit_hwnd,
        0x00B1, /* EM_SETSEL */
        0,
        stem_len as LPARAM,
    );

    // 子类化 EDIT 控件来拦截 Enter/Escape
    let orig_proc = SetWindowLongPtrW(
        edit_hwnd,
        -4, /* GWLP_WNDPROC */
        edit_subclass as *const () as isize,
    );

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
    let orig = EDIT_ORIG_PROC.load(Ordering::SeqCst);
    if orig == 0 {
        return 0;
    }

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

unsafe fn cleanup_and_notify(
    parent_hwnd: HWND,
    item_index: i32,
    edit_hwnd: HWND,
    container_hwnd: HWND,
) {
    // 获取最终文本
    let text = {
        let len = SendMessageW(edit_hwnd, 0x000E, 0, 0) as i32;
        if len > 0 {
            let mut buf: Vec<u16> = vec![0; (len + 1) as usize];
            SendMessageW(
                edit_hwnd,
                0x000D,
                buf.len() as WPARAM,
                buf.as_mut_ptr() as LPARAM,
            );
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..end])
        } else {
            String::new()
        }
    };

    log::info!("[rename] Final text: '{}'", text);

    // 检查是否是取消
    let cancelled = EDIT_RESULT
        .lock()
        .unwrap()
        .take()
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
        PostMessageW(
            parent_hwnd,
            WM_USER_RENAME_RESULT,
            item_index as WPARAM,
            ptr as LPARAM,
        );
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

/// Build the final filename for a rename operation, preserving the original file extension.
///
/// The rename editor pre-selects only the stem (via EM_SETSEL),
/// so user input is always intended as a new stem name.
///
/// - If the old path has an extension and the user didn't type it, append it automatically.
/// - If the user already typed the correct extension, don't double-append.
/// - If the old path has no extension, return the new name as-is.
pub fn build_rename_filename(new_name: &str, old_path: &str) -> String {
    if let Some(ext) = std::path::Path::new(old_path).extension() {
        let ext_str = ext.to_string_lossy();
        let with_ext = format!("{}.{}", new_name, ext_str);
        // If user already typed the correct extension, don't double-append
        if new_name.ends_with(&format!(".{}", ext_str)) {
            new_name.to_string()
        } else {
            with_ext
        }
    } else {
        new_name.to_string()
    }
}

/// Extract display name from a file path (filename stem without extension).
///
/// Used after rename to update the display_name from the new file_path.
/// Falls back to the full path string if file_stem() returns None.
pub fn display_name_from_path(file_path: &str) -> String {
    std::path::Path::new(file_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== get_name_stem_len tests =====

    #[test]
    fn test_stem_len_regular_file() {
        // "document.txt" -> stem is "document" -> 8 UTF-16 units
        assert_eq!(get_name_stem_len("document.txt"), 8);
    }

    #[test]
    fn test_stem_len_no_extension() {
        // "README" -> no extension, use full name -> 6 UTF-16 units
        assert_eq!(get_name_stem_len("README"), 6);
    }

    #[test]
    fn test_stem_len_dotfile() {
        // ".gitignore" -> file_stem() returns Some(".gitignore") on Rust (no extension)
        // Path::new(".gitignore").extension() returns None, file_stem returns ".gitignore"
        let len = get_name_stem_len(".gitignore");
        assert_eq!(len, 10); // ".gitignore" = 10 UTF-16 code units
    }

    #[test]
    fn test_stem_len_cjk_filename() {
        // "文档.pdf" -> stem is "文档" -> 2 UTF-16 units (CJK chars are 1 UTF-16 unit each)
        assert_eq!(get_name_stem_len("文档.pdf"), 2);
    }

    #[test]
    fn test_stem_len_cjk_filename_no_extension() {
        // "桌面快捷方式" -> no extension -> 6 UTF-16 units
        assert_eq!(get_name_stem_len("桌面快捷方式"), 6);
    }

    #[test]
    fn test_stem_len_multiple_dots() {
        // "archive.tar.gz" -> stem is "archive.tar", ext is "gz" -> 11 UTF-16 units
        assert_eq!(get_name_stem_len("archive.tar.gz"), 11);
    }

    #[test]
    fn test_stem_len_empty_string() {
        assert_eq!(get_name_stem_len(""), 0);
    }

    #[test]
    fn test_stem_len_trailing_dot() {
        // "file." -> stem is "file", ext is "" -> 4 UTF-16 units
        // Actually Path::new("file.").extension() returns Some(""), file_stem is "file"
        assert_eq!(get_name_stem_len("file."), 4);
    }

    // ===== build_rename_filename tests =====

    #[test]
    fn test_build_rename_preserves_extension() {
        // User types "new_name", old file was "old.txt" -> "new_name.txt"
        assert_eq!(
            build_rename_filename("new_name", "C:\\Desktop\\old.txt"),
            "new_name.txt"
        );
    }

    #[test]
    fn test_build_rename_no_double_append() {
        // User types "new_name.txt", old file was "old.txt" -> "new_name.txt" (not "new_name.txt.txt")
        assert_eq!(
            build_rename_filename("new_name.txt", "C:\\Desktop\\old.txt"),
            "new_name.txt"
        );
    }

    #[test]
    fn test_build_rename_no_extension_in_old() {
        // User types "README", old file was "README" -> "README"
        assert_eq!(
            build_rename_filename("README", "C:\\Desktop\\README"),
            "README"
        );
    }

    #[test]
    fn test_build_rename_cjk_stem() {
        // User types "新文档", old file was "文档.pdf" -> "新文档.pdf"
        assert_eq!(
            build_rename_filename("新文档", "C:\\Desktop\\文档.pdf"),
            "新文档.pdf"
        );
    }

    #[test]
    fn test_build_rename_different_extension_typed() {
        // User types "file.md", old file was "file.txt" -> "file.md.txt"
        // (extension doesn't match, so .txt gets appended)
        assert_eq!(
            build_rename_filename("file.md", "C:\\Desktop\\file.txt"),
            "file.md.txt"
        );
    }

    #[test]
    fn test_build_rename_multiple_dots_in_old() {
        // old: "archive.tar.gz" -> ext is "gz", stem is "archive.tar"
        // User types "new_archive" -> "new_archive.gz"
        assert_eq!(
            build_rename_filename("new_archive", "C:\\Desktop\\archive.tar.gz"),
            "new_archive.gz"
        );
    }

    #[test]
    fn test_build_rename_user_types_correct_ext_for_multi_dot() {
        // old: "archive.tar.gz" -> ext is "gz"
        // User types "new_archive.gz" -> "new_archive.gz" (no double append)
        assert_eq!(
            build_rename_filename("new_archive.gz", "C:\\Desktop\\archive.tar.gz"),
            "new_archive.gz"
        );
    }

    #[test]
    fn test_build_rename_lnk_shortcut() {
        // .lnk files are very common on Windows desktop
        assert_eq!(
            build_rename_filename("Chrome浏览器", "C:\\Desktop\\Chrome.lnk"),
            "Chrome浏览器.lnk"
        );
    }

    #[test]
    fn test_build_rename_case_sensitive_extension() {
        // Extensions are case-sensitive in comparison
        assert_eq!(
            build_rename_filename("file.TXT", "C:\\Desktop\\file.txt"),
            "file.TXT.txt"
        );
    }

    // ===== display_name_from_path tests =====

    #[test]
    fn test_display_name_regular_file() {
        assert_eq!(display_name_from_path(r"C:\Desktop\Chrome.lnk"), "Chrome");
    }

    #[test]
    fn test_display_name_no_extension() {
        assert_eq!(display_name_from_path(r"C:\Desktop\README"), "README");
    }

    #[test]
    fn test_display_name_cjk() {
        assert_eq!(display_name_from_path(r"C:\Desktop\文档.pdf"), "文档");
    }

    #[test]
    fn test_display_name_multiple_dots() {
        assert_eq!(
            display_name_from_path(r"C:\Desktop\archive.tar.gz"),
            "archive.tar"
        );
    }

    #[test]
    fn test_display_name_dotfile() {
        // .gitignore has no extension in Rust's Path, file_stem = ".gitignore"
        assert_eq!(
            display_name_from_path(r"C:\Desktop\.gitignore"),
            ".gitignore"
        );
    }

    #[test]
    fn test_display_name_empty_path() {
        assert_eq!(display_name_from_path(""), "");
    }
}
