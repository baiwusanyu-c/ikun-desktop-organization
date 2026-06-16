//! 桌面创建分类框对话框 + 注册表集成
//!
//! 提供独立的 Win32 输入对话框（非 egui），用于创建新分类框。
//! 通过 Windows 注册表将"创建分类框"选项集成到原生桌面右键菜单。
//!
//! # 公共 API
//! - `show_create_box_dialog()` — 显示对话框 → 创建分类框 → 保存配置
//! - `setup_desktop_registry()` — 注册桌面右键菜单项

use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::{create_grid_window, get_grid_count, GridCreateParams};
use super::icon_hider::hide_native_icons;
use crate::config::app_config::{AppConfig, ViewMode};
use crate::config::BoxLayoutData;
use std::ffi::c_void;
use std::sync::Mutex;

// ===== 对话框内部数据 =====

/// 对话框内部数据（存储在 GWLP_USERDATA）
struct DialogData {
    edit_hwnd: HWND,
    font_handle: HGDIOBJ,
}

/// 对话框结果（跨消息泵传递）
static DIALOG_RESULT: Mutex<Option<String>> = Mutex::new(None);

// ===== 公共 API =====

/// 显示独立的 Win32「创建分类框」对话框
///
/// 在桌面直接显示（非 egui），用户输入名称后：
/// 1. 隐藏原生桌面图标
/// 2. 创建空分类框窗口
/// 3. 保存到 `~/.ikun_organizer.json`
///
/// 返回创建成功的新框名称（`None` 表示取消或失败）
pub fn show_create_box_dialog() -> Option<String> {
    unsafe {
        // 初始化通用控件（确保 EDIT/BUTTON 控件可用）
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_WIN95_CLASSES,
        };
        InitCommonControlsEx(&icc);

        if !show_input_dialog() {
            return None;
        }

        // 从 DIALOG_RESULT 读取用户输入
        let name = DIALOG_RESULT.lock().ok().and_then(|mut g| g.take())?;
        if name.trim().is_empty() || name.chars().count() > 16 {
            return None;
        }
        let name = name.trim().to_string();

        // 创建分类框
        create_box_and_save(&name);
        Some(name)
    }
}

/// 注册桌面右键菜单项（通过 Windows 注册表）
///
/// 在 `HKCU\Software\Classes\Directory\Background\shell\IkunCreateBox` 写入注册表项，
/// 使 Windows 原生桌面右键菜单显示「创建分类框」选项。
/// 用户点击时，系统会启动本程序并传递 `--create-box` 参数。
pub fn setup_desktop_registry() {
    let exe_path = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            log::error!("注册表设置失败: 无法获取可执行文件路径: {}", e);
            return;
        }
    };

    let key_path: Vec<u16> = "Software\\Classes\\Directory\\Background\\shell\\IkunCreateBox\0"
        .encode_utf16()
        .collect();
    let cmd_key_path: Vec<u16> =
        "Software\\Classes\\Directory\\Background\\shell\\IkunCreateBox\\command\0"
            .encode_utf16()
            .collect();

    let menu_text: Vec<u16> = "创建分类框(&N)\0".encode_utf16().collect();
    let command: Vec<u16> = format!("\"{}\" --create-box\0", exe_path)
        .encode_utf16()
        .collect();

    unsafe {
        // 1. 创建菜单项键
        let mut hkey: isize = 0;
        let ret = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null_mut(),
            &mut hkey,
            std::ptr::null_mut(),
        );
        if ret != 0 {
            log::error!("注册表: RegCreateKeyExW 失败, 错误码={}", ret);
            return;
        }

        // 设置默认值（菜单文本）
        RegSetValueExW(
            hkey,
            std::ptr::null(), // 默认值
            0,
            REG_SZ,
            menu_text.as_ptr() as *const u8,
            (menu_text.len() * 2) as u32,
        );
        RegCloseKey(hkey);

        // 2. 创建 command 子键
        let mut cmd_hkey: isize = 0;
        let ret = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            cmd_key_path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null_mut(),
            &mut cmd_hkey,
            std::ptr::null_mut(),
        );
        if ret != 0 {
            log::error!("注册表: command 子键创建失败, 错误码={}", ret);
            return;
        }

        // 设置默认值（命令）
        RegSetValueExW(
            cmd_hkey,
            std::ptr::null(),
            0,
            REG_SZ,
            command.as_ptr() as *const u8,
            (command.len() * 2) as u32,
        );
        RegCloseKey(cmd_hkey);

        log::info!("注册表: 桌面右键菜单已注册 ({})", exe_path);
    }
}

// ===== Win32 输入对话框 =====

/// 显示输入对话框（模态，阻塞直到用户确认或取消）
///
/// 返回 `true` 表示用户点击了确定
unsafe fn show_input_dialog() -> bool {
    let hinstance = GetModuleHandleW(std::ptr::null());

    let class_name: &[u16] = &[73, 107, 117, 110, 66, 111, 120, 68, 108, 103, 0]; // "IkunBoxDlg"

    let wc = WNDCLASSEXW {
        cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfn_wnd_proc: Some(dialog_wnd_proc),
        cb_cls_extra: 0,
        cb_wnd_extra: 0,
        h_instance: hinstance,
        h_icon: std::ptr::null_mut(),
        h_cursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as *const u16),
        hbr_background: 6 as HBRUSH, // COLOR_WINDOW + 1
        lpsz_menu_name: std::ptr::null(),
        lpsz_class_name: class_name.as_ptr(),
        h_icon_sm: std::ptr::null_mut(),
    };
    RegisterClassExW(&wc);

    // 计算窗口位置（屏幕居中）
    let (screen_w, screen_h) = super::grid_window::get_screen_resolution();
    let dlg_w = 400i32;
    let dlg_h = 200i32;
    let x = (screen_w - dlg_w) / 2;
    let y = (screen_h - dlg_h) / 3; // 偏上

    let title: Vec<u16> = "创建分类框\0".encode_utf16().collect();
    let dialog_hwnd = CreateWindowExW(
        WS_EX_TOPMOST,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW & !0x00040000 & !0x00020000, // 去掉 THICKFRAME + MAXIMIZEBOX
        x,
        y,
        dlg_w,
        dlg_h,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null_mut(),
    );

    if dialog_hwnd.is_null() {
        log::error!("创建对话框窗口失败");
        return false;
    }

    SetForegroundWindow(dialog_hwnd);
    ShowWindow(dialog_hwnd, SW_SHOW);

    // 局部消息泵（模态行为）
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    // 检查是否有结果
    DIALOG_RESULT
        .lock()
        .ok()
        .map(|g| g.is_some())
        .unwrap_or(false)
}

/// 对话框窗口过程
unsafe extern "system" fn dialog_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let hinstance = GetModuleHandleW(std::ptr::null());
            let pad = 20i32;

            // 获取客户区宽度
            let mut cr = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            GetClientRect(hwnd, &mut cr);
            let client_w = cr.right - cr.left;

            // 标签
            let label_text: Vec<u16> = "分类框名称（最多16个字符）：\0".encode_utf16().collect();
            let static_class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            CreateWindowExW(
                0,
                static_class.as_ptr(),
                label_text.as_ptr(),
                WS_VISIBLE | WS_CHILD,
                pad,
                pad,
                client_w - pad * 2,
                20,
                hwnd,
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null_mut(),
            );

            // 输入框（带边框样式）
            let edit_class: Vec<u16> = "EDIT\0".encode_utf16().collect();
            let edit_hwnd = CreateWindowExW(
                0x0200, // WS_EX_CLIENTEDGE
                edit_class.as_ptr(),
                std::ptr::null(),
                WS_VISIBLE | WS_CHILD | 0x0080, // ES_AUTOHSCROLL
                pad,
                pad + 28,
                client_w - pad * 2,
                28,
                hwnd,
                1001 as *mut c_void,
                hinstance,
                std::ptr::null_mut(),
            );

            // 设置中文字体
            let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
            let hfont = CreateFontW(
                -16,         // 字符高度
                0,           // 宽度（自动）
                0,           // 角度
                0,           // 方向
                400,         // FW_NORMAL
                0,           // 斜体
                0,           // 下划线
                0,           // 删除线
                134,         // GB2312_CHARSET
                0,           // 输出精度
                0,           // 裁剪精度
                5,           // CLEARTYPE_QUALITY
                0x02 | 0x20, // FF_SWISS | DEFAULT_PITCH
                font_name.as_ptr(),
            );
            if !hfont.is_null() {
                SendMessageW(edit_hwnd, 0x0030, hfont as WPARAM, 1); // WM_SETFONT
            }

            // 按钮
            let btn_class: Vec<u16> = "BUTTON\0".encode_utf16().collect();
            let btn_w = 90i32;
            let btn_h = 32i32;
            let btn_y = pad + 28 + 28 + 16;
            let cancel_x = client_w - pad - btn_w;
            let ok_x = cancel_x - btn_w - 10;

            let ok_text: Vec<u16> = "确定\0".encode_utf16().collect();
            CreateWindowExW(
                0,
                btn_class.as_ptr(),
                ok_text.as_ptr(),
                WS_VISIBLE | WS_CHILD,
                ok_x,
                btn_y,
                btn_w,
                btn_h,
                hwnd,
                2001 as *mut c_void,
                hinstance,
                std::ptr::null_mut(),
            );

            let cancel_text: Vec<u16> = "取消\0".encode_utf16().collect();
            CreateWindowExW(
                0,
                btn_class.as_ptr(),
                cancel_text.as_ptr(),
                WS_VISIBLE | WS_CHILD,
                cancel_x,
                btn_y,
                btn_w,
                btn_h,
                hwnd,
                2002 as *mut c_void,
                hinstance,
                std::ptr::null_mut(),
            );

            // 保存数据
            let data = Box::new(DialogData {
                edit_hwnd,
                font_handle: hfont,
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);

            // 聚焦输入框
            SetForegroundWindow(hwnd);
            0
        }

        WM_COMMAND => {
            let ctrl_id = (wparam & 0xFFFF) as i32;
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DialogData;
            if data_ptr.is_null() {
                return 0;
            }
            let data = &mut *data_ptr;
            match ctrl_id {
                2001 => {
                    // 确定 — 读取输入框文本
                    let mut buf = [0u16; 64];
                    let len = SendMessageW(
                        data.edit_hwnd,
                        0x000D, // WM_GETTEXT
                        buf.len() as WPARAM,
                        buf.as_ptr() as LPARAM,
                    ) as usize;
                    let name = String::from_utf16_lossy(&buf[..len]);
                    let trimmed = name.trim().to_string();
                    if !trimmed.is_empty() && trimmed.chars().count() <= 16 {
                        *DIALOG_RESULT.lock().unwrap() = Some(trimmed);
                        DestroyWindow(hwnd);
                    }
                }
                2002 => {
                    // 取消
                    *DIALOG_RESULT.lock().unwrap() = None;
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }

        WM_CLOSE => {
            *DIALOG_RESULT.lock().unwrap() = None;
            DestroyWindow(hwnd);
            0
        }

        WM_DESTROY => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DialogData;
            if !data_ptr.is_null() {
                let data = Box::from_raw(data_ptr);
                // 释放字体
                if !data.font_handle.is_null() {
                    DeleteObject(data.font_handle);
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            // 退出局部消息泵
            post_quit_message(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// PostQuitMessage FFI
unsafe fn post_quit_message(exit_code: i32) {
    extern "system" {
        fn PostQuitMessage(nExitCode: i32);
    }
    PostQuitMessage(exit_code);
}

// ===== 分类框创建 =====

/// 创建分类框并保存到配置
unsafe fn create_box_and_save(name: &str) {
    log::info!("创建分类框: '{}'", name);

    // 1. 隐藏原生桌面图标
    if let Err(e) = hide_native_icons() {
        log::error!("隐藏桌面图标失败: {}", e);
        return;
    }

    // 2. 计算位置（屏幕中央偏上）
    let (screen_w, screen_h) = super::grid_window::get_screen_resolution();
    let box_w = 680i32;
    let box_h = 800i32;
    let existing = get_grid_count();
    let offset_x = (existing as i32 * 40) % 200;
    let offset_y = (existing as i32 * 30) % 150;
    let x = ((screen_w - box_w) / 2 + offset_x).max(10);
    let y = ((screen_h as f32 * 0.1) as i32 + offset_y).max(10);

    // 3. 创建窗口
    let params = GridCreateParams {
        id: existing,
        title: name.to_string(),
        x,
        y,
        width: box_w,
        height: box_h,
        items: Vec::new(),
        view_mode: ViewMode::Medium,
    };

    match create_grid_window(params) {
        Ok(_) => {
            log::info!("分类框 '{}' 创建成功", name);
            // 4. 保存配置
            let layout = super::grid_window::collect_grid_layout();
            if let Err(e) = save_layout(&layout) {
                log::error!("保存配置失败: {}", e);
            }
        }
        Err(e) => log::error!("创建分类框失败: {}", e),
    }
}

/// 保存布局到 ~/.ikun_organizer.json
fn save_layout(layout: &[BoxLayoutData]) -> anyhow::Result<()> {
    let path = AppConfig::get_config_path()?;
    let mut config = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        serde_json::from_str::<AppConfig>(&content).unwrap_or_default()
    } else {
        AppConfig::default()
    };
    config.layout.boxes = layout.to_vec();
    config.last_modified = chrono::Utc::now();
    let content = serde_json::to_string_pretty(&config)?;
    std::fs::write(&path, content)?;
    Ok(())
}
