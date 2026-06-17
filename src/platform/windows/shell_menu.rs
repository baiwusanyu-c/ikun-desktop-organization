//! Shell 上下文菜单功能
//!
//! 提供文件右键菜单和桌面背景右键菜单的实现。
//! 使用 Windows Shell COM API 获取真实的系统上下文菜单，
//! 并在 COM 调用失败时回退到 ShellExecuteW 自定义菜单。
//!
//! "创建分类框"选项通过注册表注入（`desktop_hook.rs`），
//! QueryContextMenu 会自动合并注册表项，覆盖初始菜单和"显示更多选项"legacy 菜单。
//! 程序退出时清理注册表，确保不运行时无残留。

#![allow(clippy::let_unit_value)]

use super::ffi_imports::*;
use super::ffi_types::*;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};

/// 用户点击"创建分类框"后设置的原子标志，由应用层轮询并消费
/// （仅在 COM 失败的回退菜单路径中使用）
static PENDING_CREATE_BOX: AtomicBool = AtomicBool::new(false);

/// 查询是否有待处理的"创建分类框"请求（消费标志，读取后自动清除）
pub fn take_pending_create_box() -> bool {
    PENDING_CREATE_BOX.swap(false, Ordering::SeqCst)
}

/// 显示文件的 Shell 右键菜单（尝试 windows crate 高级绑定 → ShellExecuteW 回退）
pub fn show_shell_context_menu(hwnd: HWND, file_path: &str) {
    // 尝试使用 windows crate 获取完整 Shell 上下文菜单
    if try_windows_crate_context_menu(hwnd, file_path) {
        return;
    }
    // 回退：ShellExecuteW 增强菜单
    show_shell_fallback_menu(hwnd, file_path);
}

/// 使用 windows crate 高级 COM 绑定获取完整 Shell 菜单
///
/// 关键修复：使用 SHBindToParent 获取父文件夹和相对 PIDL，
/// 而非将绝对 PIDL 直接传给桌面文件夹的 GetUIObjectOf。
/// 使用 windows crate 自带的 QueryContextMenu/InvokeCommand 方法，
/// 避免手动 vtable 调用导致的 ABI 兼容性问题。
fn try_windows_crate_context_menu(hwnd: HWND, file_path: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{
        IContextMenu, ILFree, IShellFolder, SHBindToParent, SHParseDisplayName,
    };

    unsafe {
        let path_wide: Vec<u16> = file_path.encode_utf16().chain(std::iter::once(0)).collect();

        // 1. 解析路径为绝对 PIDL
        let mut full_pidl = std::ptr::null_mut();
        if SHParseDisplayName(PCWSTR(path_wide.as_ptr()), None, &mut full_pidl, 0, None).is_err()
            || full_pidl.is_null()
        {
            log::warn!("windows crate: SHParseDisplayName 失败");
            return false;
        }

        // 2. 获取父文件夹和相对 PIDL（关键修复）
        //    旧代码直接将绝对 PIDL 传给桌面文件夹的 GetUIObjectOf，
        //    但 GetUIObjectOf 期望的是相对 PIDL（父文件夹的子项）。
        //    对于深层嵌套路径，这会导致 GetUIObjectOf 返回损坏的 IContextMenu，
        //    后续 QueryContextMenu 调用时崩溃 (STATUS_ACCESS_VIOLATION)。
        let mut rel_pidl: *mut windows::Win32::UI::Shell::Common::ITEMIDLIST = std::ptr::null_mut();
        let parent: Result<IShellFolder, _> =
            SHBindToParent(full_pidl as *const _, Some(&mut rel_pidl));
        let parent = match parent {
            Ok(f) => f,
            Err(e) => {
                log::warn!("windows crate: SHBindToParent 失败: {:?}", e);
                let _ = ILFree(Some(full_pidl));
                return false;
            }
        };

        if rel_pidl.is_null() {
            log::warn!("windows crate: SHBindToParent 返回空相对 PIDL");
            let _ = ILFree(Some(full_pidl));
            return false;
        }

        // 3. 使用父文件夹 + 相对 PIDL 获取 IContextMenu
        log::debug!("windows crate: 调用 GetUIObjectOf (父文件夹 + 相对 PIDL)");
        let pidl_array: [*const windows::Win32::UI::Shell::Common::ITEMIDLIST; 1] =
            [rel_pidl as *const _];
        let result: Result<IContextMenu, _> = parent.GetUIObjectOf(
            windows::Win32::Foundation::HWND(std::ptr::null_mut()),
            &pidl_array,
            None,
        );

        match result {
            Ok(cm) => {
                log::debug!("windows crate: GetUIObjectOf 成功!");

                // 4. 使用 windows crate 的 QueryContextMenu（安全方法调用，非手动 vtable）
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                let menu = CreatePopupMenu();
                let flags =
                    CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME | CMF_EXTENDED_VERBS | CMF_ITEMMENU;

                let hmenu = windows::Win32::UI::WindowsAndMessaging::HMENU(menu);
                match cm.QueryContextMenu(hmenu, 0, FCIDM_SHVIEWFIRST, MENU_CMD_ID_LAST, flags) {
                    Ok(()) => {
                        log::debug!("windows crate: QueryContextMenu 成功");
                        let cmd = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        if cmd > 0 {
                            // 5. 使用 windows crate 的 InvokeCommand
                            let invoke_info = windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
                                cbSize: std::mem::size_of::<
                                    windows::Win32::UI::Shell::CMINVOKECOMMANDINFO,
                                >() as u32,
                                fMask: 0,
                                hwnd: windows::Win32::Foundation::HWND(hwnd),
                                lpVerb: windows::core::PCSTR(
                                    ((cmd as u32 - FCIDM_SHVIEWFIRST) as usize) as *const u8,
                                ),
                                lpParameters: windows::core::PCSTR(std::ptr::null()),
                                lpDirectory: windows::core::PCSTR(std::ptr::null()),
                                nShow: SW_SHOW,
                                dwHotKey: 0,
                                hIcon: windows::Win32::Foundation::HANDLE(std::ptr::null_mut()),
                            };
                            if let Err(e) = cm.InvokeCommand(&invoke_info) {
                                log::warn!("windows crate: InvokeCommand 失败: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("windows crate: QueryContextMenu 失败: {:?}", e);
                    }
                }
                DestroyMenu(menu);
                drop(cm); // Release COM object
                let _ = ILFree(Some(full_pidl));
                true
            }
            Err(e) => {
                log::warn!("windows crate: GetUIObjectOf 失败: {:?}", e);
                let _ = ILFree(Some(full_pidl));
                false
            }
        }
    }
}

/// ShellExecuteW 回退菜单
fn show_shell_fallback_menu(hwnd: HWND, file_path: &str) {
    unsafe {
        let path_wide: Vec<u16> = file_path.encode_utf16().chain(std::iter::once(0)).collect();

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let menu = CreatePopupMenu();

        let open_text: Vec<u16> = "打开(&O)\0".encode_utf16().collect();
        let open_dir: Vec<u16> = "打开文件所在位置(&I)\0".encode_utf16().collect();
        let runas_text: Vec<u16> = "以管理员身份运行(&A)\0".encode_utf16().collect();
        let delete_text: Vec<u16> = "删除(&D)\0".encode_utf16().collect();
        let props_text: Vec<u16> = "属性(&R)\0".encode_utf16().collect();

        AppendMenuW(menu, MF_STRING, 1, open_text.as_ptr());
        AppendMenuW(menu, MF_STRING, 2, open_dir.as_ptr());
        let lower = file_path.to_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".lnk") {
            AppendMenuW(menu, MF_STRING, 3, runas_text.as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, 4, delete_text.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, 5, props_text.as_ptr());

        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, std::ptr::null());
        match cmd {
            1 => {
                let op: Vec<u16> = "open\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            2 => {
                let op: Vec<u16> = "explore\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            3 => {
                let op: Vec<u16> = "runas\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            4 => {
                let op: Vec<u16> = "delete\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            5 => {
                let op: Vec<u16> = "properties\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            _ => {}
        }
        DestroyMenu(menu);
    }
}

/// 显示桌面背景右键菜单（使用 IShellView::GetItemObject(SVGIO_BACKGROUND) 获取真实桌面菜单）
///
/// 桌面背景右键菜单由 shell 视图 (defview) 提供，而非 IShellFolder 直接提供。
/// 正确路径：IShellFolder → CreateViewObject::<IShellView> → GetItemObject::<IContextMenu>(SVGIO_BACKGROUND)
/// 这会返回与桌面右键完全一致的菜单：查看、排序方式、刷新、新建、显示设置、个性化等。
///
/// 「创建分类框」选项通过注册表注入（`desktop_hook.rs`），Shell 会自动将其合并到此菜单中，
/// 同时覆盖初始菜单和"显示更多选项"legacy 菜单。程序退出时清理注册表，确保无残留。
pub fn show_desktop_context_menu(hwnd: HWND) {
    use windows::Win32::UI::Shell::{
        IContextMenu, IShellFolder, IShellView, SHGetDesktopFolder, SVGIO_BACKGROUND,
    };

    unsafe {
        // 1. 获取桌面 IShellFolder
        let desktop: Result<IShellFolder, _> = SHGetDesktopFolder();
        let desktop = match desktop {
            Ok(f) => f,
            Err(e) => {
                log::warn!("桌面菜单: SHGetDesktopFolder 失败: {:?}", e);
                show_desktop_fallback_menu(hwnd);
                return;
            }
        };

        // 2. 创建 shell 视图 (IShellView)
        let view: Result<IShellView, _> =
            desktop.CreateViewObject(windows::Win32::Foundation::HWND(hwnd));
        let view = match view {
            Ok(v) => v,
            Err(e) => {
                log::warn!("桌面菜单: CreateViewObject(IShellView) 失败: {:?}", e);
                show_desktop_fallback_menu(hwnd);
                return;
            }
        };

        // 3. 从 shell 视图获取背景区域的 IContextMenu
        //    QueryContextMenu 填充原生菜单后，通过 InsertMenuW 编程式追加「创建分类框」。
        let result: Result<IContextMenu, _> = view.GetItemObject(SVGIO_BACKGROUND);
        match result {
            Ok(cm) => {
                log::debug!("桌面菜单: GetItemObject(SVGIO_BACKGROUND) 成功");
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                let menu = CreatePopupMenu();
                let flags = CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME | CMF_EXTENDED_VERBS;
                let hmenu = windows::Win32::UI::WindowsAndMessaging::HMENU(menu);

                match cm.QueryContextMenu(hmenu, 0, FCIDM_SHVIEWFIRST, MENU_CMD_ID_LAST, flags) {
                    Ok(()) => {
                        log::debug!("桌面菜单: QueryContextMenu 成功");

                        // 注：注册表项 IkunCreateBox 会被 Shell 自动合并到 QueryContextMenu 结果中，
                        // 覆盖初始菜单和"显示更多选项"legacy 菜单，无需编程式追加。
                        // 程序退出时 cleanup_desktop_registry() 清理注册表，确保不运行时无残留。

                        let cmd = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        if cmd > 0 && cmd as u32 >= FCIDM_SHVIEWFIRST {
                            let invoke_info = windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
                                cbSize: std::mem::size_of::<
                                    windows::Win32::UI::Shell::CMINVOKECOMMANDINFO,
                                >() as u32,
                                fMask: 0,
                                hwnd: windows::Win32::Foundation::HWND(hwnd),
                                lpVerb: windows::core::PCSTR(
                                    ((cmd as u32 - FCIDM_SHVIEWFIRST) as usize) as *const u8,
                                ),
                                lpParameters: windows::core::PCSTR(std::ptr::null()),
                                lpDirectory: windows::core::PCSTR(std::ptr::null()),
                                nShow: SW_SHOW,
                                dwHotKey: 0,
                                hIcon: windows::Win32::Foundation::HANDLE(std::ptr::null_mut()),
                            };
                            if let Err(e) = cm.InvokeCommand(&invoke_info) {
                                log::warn!("桌面菜单: InvokeCommand 失败: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("桌面菜单: QueryContextMenu 失败: {:?}", e);
                    }
                }
                DestroyMenu(menu);
                drop(cm);
            }
            Err(e) => {
                log::warn!("桌面菜单: GetItemObject(SVGIO_BACKGROUND) 失败: {:?}", e);
                show_desktop_fallback_menu(hwnd);
            }
        }
    }
}

/// 桌面背景右键菜单回退方案（ShellExecuteW 自定义菜单）
fn show_desktop_fallback_menu(hwnd: HWND) {
    /// 回退菜单中"创建分类框"的命令 ID（不与 CMD_CREATE_BOX 共享作用域）
    const FALLBACK_CMD_CREATE_BOX: i32 = 4;

    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let menu = CreatePopupMenu();

        let create_box: Vec<u16> = "➕ 创建分类框(&N)\0".encode_utf16().collect();
        let open_desktop: Vec<u16> = "打开桌面文件夹(&O)\0".encode_utf16().collect();
        let display: Vec<u16> = "显示设置(&D)\0".encode_utf16().collect();
        let personalize: Vec<u16> = "个性化(&P)\0".encode_utf16().collect();

        AppendMenuW(
            menu,
            MF_STRING,
            FALLBACK_CMD_CREATE_BOX as usize,
            create_box.as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, 1, open_desktop.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, 2, display.as_ptr());
        AppendMenuW(menu, MF_STRING, 3, personalize.as_ptr());

        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, std::ptr::null());
        match cmd {
            c if c == FALLBACK_CMD_CREATE_BOX => {
                log::info!("桌面菜单（回退）: 用户点击「创建分类框」");
                PENDING_CREATE_BOX.store(true, Ordering::SeqCst);
            }
            1 => {
                // 打开桌面文件夹
                let desktop_path = dirs::desktop_dir().unwrap_or_default();
                let path_wide: Vec<u16> = desktop_path
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let op: Vec<u16> = "explore\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            2 => {
                // 显示设置
                let uri: Vec<u16> = "ms-settings:display\0".encode_utf16().collect();
                let op: Vec<u16> = "open\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    uri.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            3 => {
                // 个性化
                let uri: Vec<u16> = "ms-settings:personalization\0".encode_utf16().collect();
                let op: Vec<u16> = "open\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    uri.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            _ => {}
        }
        DestroyMenu(menu);
    }
}

// ===== 注册表管理 =====

/// 注册桌面右键菜单项（通过 Windows 注册表）
///
/// 在 `HKCU\Software\Classes\Directory\Background\shell\IkunCreateBox` 写入注册表项，
/// 使 Windows 原生桌面右键菜单（包括"显示更多选项"legacy 菜单）显示「创建分类框」选项。
/// Shell 会自动将注册表项合并到 `QueryContextMenu` 返回的菜单中。
///
/// **必须在程序退出时调用 `cleanup_desktop_registry()` 清理**。
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

        RegSetValueExW(
            hkey,
            std::ptr::null(),
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

/// 清理桌面右键菜单注册表项
///
/// 删除 `HKCU\Software\Classes\Directory\Background\shell\IkunCreateBox`（含 command 子键）。
/// 键不存在时静默忽略，不阻塞启动。
///
/// 调用时机：
/// - 程序启动时（清理上次崩溃残留）
/// - 程序退出时（确保不运行时无残留）
pub fn cleanup_desktop_registry() {
    let cmd_key: Vec<u16> =
        "Software\\Classes\\Directory\\Background\\shell\\IkunCreateBox\\command\0"
            .encode_utf16()
            .collect();
    let key: Vec<u16> = "Software\\Classes\\Directory\\Background\\shell\\IkunCreateBox\0"
        .encode_utf16()
        .collect();

    unsafe {
        let ret1 = RegDeleteKeyW(HKEY_CURRENT_USER, cmd_key.as_ptr());
        let ret2 = RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr());

        if ret1 == 0 || ret2 == 0 {
            log::info!("已清理注册表项 IkunCreateBox");
        } else {
            log::debug!("注册表清理: 无注册表项（正常）");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_take_pending_create_box_default_is_false() {
        // 确保初始状态为 false
        // 注意：如果其他测试设置了标志，此测试可能不可靠
        // 但 take 是消费性的（读后清零），所以连续两次调用第二次一定为 false
        let _ = take_pending_create_box(); // 先消费掉可能的残留
        assert!(!take_pending_create_box());
    }

    #[test]
    fn test_pending_create_box_set_then_take() {
        // 先清零
        let _ = take_pending_create_box();

        // 设置标志
        PENDING_CREATE_BOX.store(true, Ordering::SeqCst);

        // 第一次 take 应返回 true
        assert!(take_pending_create_box());

        // 第二次 take 应返回 false（已被消费）
        assert!(!take_pending_create_box());
    }

    #[test]
    fn test_pending_create_box_take_is_idempotent() {
        let _ = take_pending_create_box(); // 清零

        // 多次 take 不会出错
        assert!(!take_pending_create_box());
        assert!(!take_pending_create_box());
        assert!(!take_pending_create_box());
    }
}
