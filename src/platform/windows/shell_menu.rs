//! Shell 上下文菜单功能
//!
//! 提供文件右键菜单和桌面背景右键菜单的实现。
//! 使用 Windows Shell COM API 获取真实的系统上下文菜单，
//! 并在 COM 调用失败时回退到 ShellExecuteW 自定义菜单。

#![allow(clippy::let_unit_value)]

use super::ffi_imports::*;
use super::ffi_types::*;
use anyhow::Result;

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
        //    CreateViewObject 在桌面文件夹上创建的是 Explorer 视图对象，
        //    它包含了桌面背景菜单所需的所有 shell 扩展。
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
        //    SVGIO_BACKGROUND 表示请求的是文件夹空白区域的上下文菜单，
        //    这正是用户在桌面空白处右键看到的菜单。
        let result: Result<IContextMenu, _> = view.GetItemObject(SVGIO_BACKGROUND);
        match result {
            Ok(cm) => {
                log::debug!("桌面菜单: GetItemObject(SVGIO_BACKGROUND) 成功");
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                let menu = CreatePopupMenu();
                let flags = CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME;
                let hmenu = windows::Win32::UI::WindowsAndMessaging::HMENU(menu);

                match cm.QueryContextMenu(hmenu, 0, FCIDM_SHVIEWFIRST, MENU_CMD_ID_LAST, flags) {
                    Ok(()) => {
                        log::debug!("桌面菜单: QueryContextMenu 成功");
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
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let menu = CreatePopupMenu();

        let open_desktop: Vec<u16> = "打开桌面文件夹(&O)\0".encode_utf16().collect();
        let display: Vec<u16> = "显示设置(&D)\0".encode_utf16().collect();
        let personalize: Vec<u16> = "个性化(&P)\0".encode_utf16().collect();

        AppendMenuW(menu, MF_STRING, 1, open_desktop.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, 2, display.as_ptr());
        AppendMenuW(menu, MF_STRING, 3, personalize.as_ptr());

        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, std::ptr::null());
        match cmd {
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
