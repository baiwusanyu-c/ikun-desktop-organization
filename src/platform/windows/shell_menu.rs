//! Shell 上下文菜单功能
//!
//! 提供文件右键菜单和桌面背景右键菜单的实现。
//! 使用 Windows Shell COM API 获取真实的系统上下文菜单，
//! 并在 COM 调用失败时回退到 ShellExecuteW 自定义菜单。
//!
//! "创建分类框"选项通过注册表注入（本模块 `setup_desktop_registry`），
//! QueryContextMenu 会自动合并注册表项，覆盖初始菜单和"显示更多选项"legacy 菜单。
//! 程序退出时清理注册表，确保不运行时无残留。

#![allow(clippy::let_unit_value)]

use super::ffi_imports::*;
use super::ffi_types::*;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::PathBuf;

/// Windows Shell 变更通知常量（用于刷新桌面右键菜单缓存）
/// SHCNE_ASSOCCHANGED: 文件关联已更改，通知 Shell 重新读取关联信息
const SHCNE_ASSOCCHANGED: u32 = 0x08000000;
/// SHCNF_IDLIST: dwItem1 和 dwItem2 为 PIDL 指针（null 时表示全局通知）
const SHCNF_IDLIST: u32 = 0x0000;

/// 用户点击"创建分类框"后设置的原子标志，由应用层轮询并消费
/// （仅在 COM 失败的回退菜单路径中使用）
static PENDING_CREATE_BOX: AtomicBool = AtomicBool::new(false);

/// [DEBUG] 枚举并记录 HMENU 中的所有菜单项
fn log_menu_items(menu: HMENU, label: &str) {
    unsafe {
        let count = GetMenuItemCount(menu);
        log::info!("[MENU-DEBUG] === {} === 菜单项总数: {}", label, count);
        for i in 0..count {
            let mut buf: Vec<u16> = vec![0; 512];
            let _len = GetMenuStringW(
                menu,
                i as u32,
                buf.as_mut_ptr(),
                buf.len() as i32,
                MF_BYPOSITION,
            );
            let cmd_id = GetMenuItemID(menu, i);
            let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
            let text = String::from_utf16_lossy(&buf[..end]);
            if text.is_empty() && cmd_id == 0 {
                log::info!("[MENU-DEBUG]   [{}] SEPARATOR", i);
            } else {
                log::info!("[MENU-DEBUG]   [{}] id={} text=\"{}\"", i, cmd_id, text);
            }
        }
        log::info!("[MENU-DEBUG] === {} === 枚举结束", label);
    }
}
/// [DEBUG] 检查菜单中是否已包含 Ikun 菜单项（用于去重）
fn menu_has_ikun_items(menu: HMENU) -> bool {
    unsafe {
        let count = GetMenuItemCount(menu);
        for i in 0..count {
            let mut buf: Vec<u16> = vec![0; 512];
            GetMenuStringW(
                menu,
                i as u32,
                buf.as_mut_ptr(),
                buf.len() as i32,
                MF_BYPOSITION,
            );
            let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
            let text = String::from_utf16_lossy(&buf[..end]);
            // 检查是否包含 Ikun 菜单项的特征文本
            if text.contains("打开控制面板")
                || text.contains("创建分类框")
                || text.contains("一键AI分类")
                || text.contains("退出应用")
            {
                log::info!("[MENU-DEBUG] 菜单中已包含 Ikun 项: \"{}\"", text);
                return true;
            }
        }
    }
    false
}

/// Result of shell context menu operation
#[derive(Debug, Clone)]
pub enum ShellMenuResult {
    /// User cancelled the menu (menu was NOT shown, e.g. COM failure)
    Cancelled,
    /// Menu was shown successfully but user dismissed it without selecting an item
    Dismissed,
    /// Shell operation completed (verb name if known)
    Completed(Option<String>),
    /// User selected rename - needs custom handling
    NeedsRename(String), // file_path
    /// User selected cut - needs monitoring
    NeedsCutMonitor(String), // file_path
    /// User selected delete - needs vault cleanup
    NeedsDelete(String), // file_path
}

/// Response from shell context menu, includes result and optional staging path for cleanup
pub struct ShellMenuResponse {
    pub result: ShellMenuResult,
    pub staging_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DesktopRegistryMenuItem {
    key_name: &'static str,
    menu_text: &'static str,
    cmd_arg: &'static str,
}

const DESKTOP_REGISTRY_MENU_ITEMS: [DesktopRegistryMenuItem; 4] = [
    DesktopRegistryMenuItem {
        key_name: "IkunOpenPanel",
        menu_text: "打开控制面板(&O)",
        cmd_arg: "--open-panel",
    },
    DesktopRegistryMenuItem {
        key_name: "IkunCreateBox",
        menu_text: "创建分类框(&N)",
        cmd_arg: "--create-box",
    },
    DesktopRegistryMenuItem {
        key_name: "IkunAiClassify",
        menu_text: "一键AI分类(&A)",
        cmd_arg: "--ai-classify",
    },
    DesktopRegistryMenuItem {
        key_name: "IkunQuit",
        menu_text: "退出应用(&Q)",
        cmd_arg: "--quit",
    },
];

fn desktop_registry_menu_items() -> &'static [DesktopRegistryMenuItem] {
    &DESKTOP_REGISTRY_MENU_ITEMS
}

/// 查询是否有待处理的"创建分类框"请求（消费标志，读取后自动清除）
pub fn take_pending_create_box() -> bool {
    PENDING_CREATE_BOX.swap(false, Ordering::SeqCst)
}

// Our custom menu item IDs (below FCIDM_SHVIEWFIRST to avoid conflicts)
/// 显示文件的 Shell 右键菜单（尝试 windows crate 高级绑定 → ShellExecuteW 回退）
/// 纯函数：判断 ShellMenuResult 是否应跳过回退菜单。
/// 仅当菜单未能显示（Cancelled）时才需要回退，
/// 菜单已显示但用户关闭（Dismissed）或其他结果均不需要回退。
fn should_skip_fallback(result: &ShellMenuResult) -> bool {
    !matches!(result, ShellMenuResult::Cancelled)
}

/// 显示文件右键 Shell 上下文菜单（分类框内图标代理入口）
///
/// 将分类框中的图标右键操作代理到 vault 原始文件上：
/// 1. 通过 `resolve_shell_menu_path` 将 logical_path 解析为 vault 路径
/// 2. 直接使用 vault 路径调用 Shell COM API（不导出到桌面，避免产生重复图标）
/// 3. Shell API (SHParseDisplayName + SHBindToParent + GetUIObjectOf) 可处理任意 NTFS 路径
///
/// Public / Non-Public 桌面图标均通过 `resolve_icon_read_path` 统一解析到对应的 vault 路径。
pub fn show_shell_context_menu(hwnd: HWND, file_path: &str) -> ShellMenuResponse {
    log::info!(
        "[MENU-DEBUG] show_shell_context_menu 入口: file_path=\"{}\"",
        file_path
    );
    let menu_path = resolve_shell_menu_path(file_path);

    if menu_path != file_path {
        log::debug!("Shell 菜单路径已解析: {} → {}", file_path, menu_path);
    }

    // 检查是否是 vault 路径，如果是则创建 staging 副本（使用原始文件名）
    let (actual_menu_path, staging_path) = if super::icon_hider::is_vault_path(&menu_path) {
        match super::icon_hider::create_staging_for_shell(file_path) {
            Some(staging) => {
                log::info!("[STAGING] 使用 staging 路径: {}", staging.display());
                (staging.to_string_lossy().to_string(), Some(staging))
            }
            None => {
                log::debug!("[STAGING] 创建 staging 失败，使用 vault 路径");
                (menu_path.clone(), None)
            }
        }
    } else {
        (menu_path.clone(), None)
    };

    log::debug!("Shell 菜单使用路径: {}", actual_menu_path);

    // 尝试使用 windows crate 获取完整 Shell 上下文菜单
    let result = try_windows_crate_context_menu(hwnd, &actual_menu_path);
    if should_skip_fallback(&result) {
        return ShellMenuResponse { result, staging_path };
    }
    // 仅当 try_windows_crate_context_menu 未能显示菜单（Cancelled）时才回退
    let fallback_result = show_shell_fallback_menu(hwnd, &actual_menu_path);
    ShellMenuResponse { result: fallback_result, staging_path }
}

fn resolve_shell_menu_path(file_path: &str) -> String {
    resolve_shell_menu_path_with(
        file_path,
        |path| std::path::Path::new(path).exists(),
        |path| {
            crate::platform::windows::icon_hider::resolve_icon_read_path(path)
                .map(|p| p.to_string_lossy().to_string())
        },
    )
}

fn resolve_shell_menu_path_with(
    file_path: &str,
    exists: impl Fn(&str) -> bool,
    resolve_read_path: impl Fn(&str) -> Option<String>,
) -> String {
    if exists(file_path) {
        return file_path.to_string();
    }
    resolve_read_path(file_path).unwrap_or_else(|| file_path.to_string())
}

/// 使用 windows crate 高级 COM 绑定获取完整 Shell 菜单
///
/// 关键修复：使用 SHBindToParent 获取父文件夹和相对 PIDL，
/// 而非将绝对 PIDL 直接传给桌面文件夹的 GetUIObjectOf。
/// 使用 windows crate 自带的 QueryContextMenu/InvokeCommand 方法，
/// 避免手动 vtable 调用导致的 ABI 兼容性问题。
fn try_windows_crate_context_menu(hwnd: HWND, file_path: &str) -> ShellMenuResult {
    use windows::core::PCWSTR;
    use windows::core::PSTR;
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
            return ShellMenuResult::Cancelled;
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
                return ShellMenuResult::Cancelled;
            }
        };

        if rel_pidl.is_null() {
            log::warn!("windows crate: SHBindToParent 返回空相对 PIDL");
            let _ = ILFree(Some(full_pidl));
            return ShellMenuResult::Cancelled;
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
                match cm
                    .QueryContextMenu(hmenu, 0, FCIDM_SHVIEWFIRST, MENU_CMD_ID_LAST, flags)
                    .ok()
                {
                    Ok(()) => {
                        log::debug!("windows crate: QueryContextMenu 成功");
                        log_menu_items(menu, "try_windows_crate:after-QCM");
                        log::info!("[MENU-DEBUG] TrackPopupMenu 即将调用: pt=({}, {}), hwnd={:?}, 菜单项数={}", pt.x, pt.y, hwnd, GetMenuItemCount(menu));
                        let cmd = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        log::info!("[MENU-DEBUG] TrackPopupMenu 返回: cmd={}", cmd);
                        if cmd > 0 {
                            // 5. Get verb string to determine action
                            let verb_ordinal = (cmd as u32 - FCIDM_SHVIEWFIRST) as usize;
                            let mut verb_buf: Vec<u8> = vec![0; 256];
                            let verb = match cm.GetCommandString(
                                verb_ordinal,
                                windows::Win32::UI::Shell::GCS_VERBA,
                                None,
                                PSTR(verb_buf.as_mut_ptr()),
                                verb_buf.len() as u32,
                            ) {
                                Ok(_) => {
                                    let end = verb_buf.iter().position(|&c| c == 0).unwrap_or(0);
                                    String::from_utf8_lossy(&verb_buf[..end]).to_lowercase()
                                }
                                Err(_) => String::new(),
                            };

                            log::info!("Shell menu verb: '{}'", verb);

                            // Handle special verbs
                            let result = match verb.as_str() {
                                "rename" => {
                                    // Don't call InvokeCommand for rename - we handle it ourselves
                                    return ShellMenuResult::NeedsRename(file_path.to_string());
                                }
                                "cut" => {
                                    // Call InvokeCommand to set up clipboard
                                    let invoke_info =
                                        windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
                                            cbSize: std::mem::size_of::<
                                                windows::Win32::UI::Shell::CMINVOKECOMMANDINFO,
                                            >()
                                                as u32,
                                            fMask: 0,
                                            hwnd: windows::Win32::Foundation::HWND(hwnd),
                                            lpVerb: windows::core::PCSTR(verb_ordinal as *const u8),
                                            lpParameters: windows::core::PCSTR(std::ptr::null()),
                                            lpDirectory: windows::core::PCSTR(std::ptr::null()),
                                            nShow: SW_SHOW,
                                            dwHotKey: 0,
                                            hIcon: windows::Win32::Foundation::HANDLE(
                                                std::ptr::null_mut(),
                                            ),
                                        };
                                    if let Err(e) = cm.InvokeCommand(&invoke_info) {
                                        log::warn!("cut InvokeCommand failed: {:?}", e);
                                    }
                                    return ShellMenuResult::NeedsCutMonitor(file_path.to_string());
                                }
                                "delete" => {
                                    // Call InvokeCommand to move to Recycle Bin
                                    let invoke_info =
                                        windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
                                            cbSize: std::mem::size_of::<
                                                windows::Win32::UI::Shell::CMINVOKECOMMANDINFO,
                                            >()
                                                as u32,
                                            fMask: 0,
                                            hwnd: windows::Win32::Foundation::HWND(hwnd),
                                            lpVerb: windows::core::PCSTR(verb_ordinal as *const u8),
                                            lpParameters: windows::core::PCSTR(std::ptr::null()),
                                            lpDirectory: windows::core::PCSTR(std::ptr::null()),
                                            nShow: SW_SHOW,
                                            dwHotKey: 0,
                                            hIcon: windows::Win32::Foundation::HANDLE(
                                                std::ptr::null_mut(),
                                            ),
                                        };
                                    if let Err(e) = cm.InvokeCommand(&invoke_info) {
                                        log::warn!("delete InvokeCommand failed: {:?}", e);
                                    }
                                    return ShellMenuResult::NeedsDelete(file_path.to_string());
                                }
                                _ => {
                                    // Other verbs: call InvokeCommand normally
                                    let invoke_info =
                                        windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
                                            cbSize: std::mem::size_of::<
                                                windows::Win32::UI::Shell::CMINVOKECOMMANDINFO,
                                            >()
                                                as u32,
                                            fMask: 0,
                                            hwnd: windows::Win32::Foundation::HWND(hwnd),
                                            lpVerb: windows::core::PCSTR(verb_ordinal as *const u8),
                                            lpParameters: windows::core::PCSTR(std::ptr::null()),
                                            lpDirectory: windows::core::PCSTR(std::ptr::null()),
                                            nShow: SW_SHOW,
                                            dwHotKey: 0,
                                            hIcon: windows::Win32::Foundation::HANDLE(
                                                std::ptr::null_mut(),
                                            ),
                                        };
                                    if let Err(e) = cm.InvokeCommand(&invoke_info) {
                                        log::warn!("InvokeCommand failed: {:?}", e);
                                    }
                                    ShellMenuResult::Completed(Some(verb))
                                }
                            };
                            return result;
                        } else {
                            return ShellMenuResult::Dismissed;
                        }
                    }
                    Err(e) => {
                        log::warn!("windows crate: QueryContextMenu 失败: {:?}", e);
                    }
                }
                DestroyMenu(menu);
                drop(cm); // Release COM object
                let _ = ILFree(Some(full_pidl));
                ShellMenuResult::Completed(None)
            }
            Err(e) => {
                log::warn!("windows crate: GetUIObjectOf 失败: {:?}", e);
                let _ = ILFree(Some(full_pidl));
                ShellMenuResult::Cancelled
            }
        }
    }
}

/// ShellExecuteW 回退菜单
fn show_shell_fallback_menu(hwnd: HWND, file_path: &str) -> ShellMenuResult {
    log::info!(
        "[MENU-DEBUG] show_shell_fallback_menu 入口: file_path=\"{}\"",
        file_path
    );
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
        let result = match cmd {
            0 => ShellMenuResult::Cancelled,
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
                ShellMenuResult::Completed(Some("open".to_string()))
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
                ShellMenuResult::Completed(Some("explore".to_string()))
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
                ShellMenuResult::Completed(Some("runas".to_string()))
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
                ShellMenuResult::NeedsDelete(file_path.to_string())
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
                ShellMenuResult::Completed(Some("properties".to_string()))
            }
            _ => ShellMenuResult::Completed(None),
        };
        DestroyMenu(menu);
        result
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
pub fn show_desktop_context_menu(hwnd: HWND, include_ikun_items: bool) {
    log::info!(
        "[MENU-DEBUG] show_desktop_context_menu 入口: include_ikun_items={}",
        include_ikun_items
    );
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
        //    QueryContextMenu 填充原生菜单后，注册表注入的 IkunXxx 项会自动合并。
        let result: Result<IContextMenu, _> = view.GetItemObject(SVGIO_BACKGROUND);
        match result {
            Ok(cm) => {
                log::debug!("桌面菜单: GetItemObject(SVGIO_BACKGROUND) 成功");
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                let menu = CreatePopupMenu();
                let flags = CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME | CMF_EXTENDED_VERBS;
                let hmenu = windows::Win32::UI::WindowsAndMessaging::HMENU(menu);

                match cm
                    .QueryContextMenu(hmenu, 0, FCIDM_SHVIEWFIRST, MENU_CMD_ID_LAST, flags)
                    .ok()
                {
                    Ok(()) => {
                        log::debug!("桌面菜单: QueryContextMenu 成功");
                        log_menu_items(menu, "show_desktop_ctx:after-QCM");

                        // 显式追加 Ikun 菜单项（Shell API 不保证自动合并注册表项到自定义窗口）
                        // 仅覆盖区域右键菜单需要，分类框空白处右键不需要
                        const IKUN_CMD_OPEN_PANEL: u32 = 10;
                        const IKUN_CMD_CREATE_BOX: u32 = 11;
                        const IKUN_CMD_AI_CLASSIFY: u32 = 12;
                        const IKUN_CMD_QUIT: u32 = 13;

                        let pre_count = GetMenuItemCount(menu);
                        log::info!(
                            "[MENU-DEBUG] 准备显式追加 Ikun 菜单项, 追加前项数: {}",
                            pre_count
                        );
                        let already_has_ikun = menu_has_ikun_items(menu);
                        log::info!("[MENU-DEBUG] 菜单是否已含 Ikun 项: {}", already_has_ikun);
                        if include_ikun_items && !already_has_ikun {
                            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

                            let open_panel: Vec<u16> =
                                "打开控制面板(&O)\0".encode_utf16().collect();
                            let create_box: Vec<u16> = "创建分类框(&N)\0".encode_utf16().collect();
                            let ai_classify: Vec<u16> = "一键AI分类(&A)\0".encode_utf16().collect();
                            let quit: Vec<u16> = "退出应用(&Q)\0".encode_utf16().collect();

                            AppendMenuW(
                                menu,
                                MF_STRING,
                                IKUN_CMD_OPEN_PANEL as usize,
                                open_panel.as_ptr(),
                            );
                            AppendMenuW(
                                menu,
                                MF_STRING,
                                IKUN_CMD_CREATE_BOX as usize,
                                create_box.as_ptr(),
                            );
                            AppendMenuW(
                                menu,
                                MF_STRING,
                                IKUN_CMD_AI_CLASSIFY as usize,
                                ai_classify.as_ptr(),
                            );
                            AppendMenuW(menu, MF_STRING, IKUN_CMD_QUIT as usize, quit.as_ptr());

                            // 为追加的菜单项设置图标
                            if let Some(hbmp) = super::app_icon::get_menu_bitmap() {
                                let count = GetMenuItemCount(menu);
                                for i in (count - 4)..count {
                                    SetMenuItemBitmaps(menu, i as u32, MF_BYPOSITION, hbmp, hbmp);
                                }
                            }
                            let post_count = GetMenuItemCount(menu);
                            log::info!("[MENU-DEBUG] 显式追加 Ikun 菜单项后, 总数: {}", post_count);
                            log_menu_items(menu, "show_desktop_ctx:after-ikun-append");
                        }

                        log::info!("[MENU-DEBUG] TrackPopupMenu 即将调用: pt=({}, {}), hwnd={:?}, 菜单项数={}", pt.x, pt.y, hwnd, GetMenuItemCount(menu));
                        let cmd = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        log::info!("[MENU-DEBUG] TrackPopupMenu 返回: cmd={}", cmd);
                        if cmd > 0 {
                            match cmd as u32 {
                                IKUN_CMD_OPEN_PANEL if include_ikun_items => {
                                    crate::platform::windows::sys_tray::set_pending_action(
                                        crate::platform::SystemTrayAction::OpenControlPanel,
                                    );
                                }
                                IKUN_CMD_CREATE_BOX if include_ikun_items => {
                                    crate::platform::windows::sys_tray::set_pending_action(
                                        crate::platform::SystemTrayAction::CreateBox,
                                    );
                                }
                                IKUN_CMD_AI_CLASSIFY if include_ikun_items => {
                                    crate::platform::windows::sys_tray::set_pending_action(
                                        crate::platform::SystemTrayAction::AiClassify,
                                    );
                                }
                                IKUN_CMD_QUIT if include_ikun_items => {
                                    crate::platform::windows::sys_tray::set_pending_action(
                                        crate::platform::SystemTrayAction::Quit,
                                    );
                                }
                                shell_cmd if shell_cmd >= FCIDM_SHVIEWFIRST => {
                                    let invoke_info =
                                        windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
                                            cbSize: std::mem::size_of::<
                                                windows::Win32::UI::Shell::CMINVOKECOMMANDINFO,
                                            >()
                                                as u32,
                                            fMask: 0,
                                            hwnd: windows::Win32::Foundation::HWND(hwnd),
                                            lpVerb: windows::core::PCSTR(
                                                ((shell_cmd - FCIDM_SHVIEWFIRST) as usize)
                                                    as *const u8,
                                            ),
                                            lpParameters: windows::core::PCSTR(std::ptr::null()),
                                            lpDirectory: windows::core::PCSTR(std::ptr::null()),
                                            nShow: SW_SHOW,
                                            dwHotKey: 0,
                                            hIcon: windows::Win32::Foundation::HANDLE(
                                                std::ptr::null_mut(),
                                            ),
                                        };
                                    if let Err(e) = cm.InvokeCommand(&invoke_info) {
                                        log::warn!("桌面菜单: InvokeCommand 失败: {:?}", e);
                                    }
                                }
                                _ => {}
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
    log::info!("[MENU-DEBUG] show_desktop_fallback_menu 入口: 创建自定义回退菜单");
    use crate::platform::SystemTrayAction;

    const CMD_OPEN_PANEL: usize = 10;
    const CMD_CREATE_BOX: usize = 11;
    const CMD_AI_CLASSIFY: usize = 12;
    const CMD_QUIT: usize = 13;

    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let menu = CreatePopupMenu();

        let open_panel: Vec<u16> = "打开控制面板(&O)\0".encode_utf16().collect();
        let create_box: Vec<u16> = "创建分类框(&N)\0".encode_utf16().collect();
        let ai_classify: Vec<u16> = "一键AI分类(&A)\0".encode_utf16().collect();
        let quit: Vec<u16> = "退出应用(&Q)\0".encode_utf16().collect();

        AppendMenuW(menu, MF_STRING, CMD_OPEN_PANEL, open_panel.as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_CREATE_BOX, create_box.as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_AI_CLASSIFY, ai_classify.as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_QUIT, quit.as_ptr());

        // 为每个菜单项添加项目图标
        if let Some(hbmp) = super::app_icon::get_menu_bitmap() {
            let count = GetMenuItemCount(menu);
            for i in 0..count {
                SetMenuItemBitmaps(menu, i as u32, MF_BYPOSITION, hbmp, hbmp);
            }
        }

        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_LEFTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);

        match cmd as usize {
            CMD_OPEN_PANEL => {
                super::sys_tray::set_pending_action(SystemTrayAction::OpenControlPanel)
            }
            CMD_CREATE_BOX => super::sys_tray::set_pending_action(SystemTrayAction::CreateBox),
            CMD_AI_CLASSIFY => super::sys_tray::set_pending_action(SystemTrayAction::AiClassify),
            CMD_QUIT => super::sys_tray::set_pending_action(SystemTrayAction::Quit),
            _ => {}
        }
    }
}

// ===== 注册表管理 =====

/// 注册桌面右键菜单项（通过 Windows 注册表）
///
/// 在 `HKCU\Software\Classes\Directory\Background\shell\IkunCreateBox` 写入注册表项，
/// 使 Windows 原生桌面右键菜单（包括"显示更多选项"legacy 菜单）显示「创建分类框」选项。
/// Shell 会自动将注册表项合并到 `QueryContextMenu` 返回的菜单中。
///

/// 格式化注册表 Icon 值的路径字符串
///
/// Windows 注册表 `HKCU\...\shell\<name>\Icon` 值的格式为 `路径,索引`，
/// **不带引号**（与 `command` 子键的 `(Default)` 值不同，后者需要引号）。
///
/// # 参数
/// - `exe_path` — 可执行文件的绝对路径
///
/// # 返回
/// 格式为 `"<exe_path>,0"` 的字符串（索引 0 表示嵌入图标的第一个资源）
fn format_registry_icon_path(exe_path: &str) -> String {
    format!("{},0", exe_path)
}
/// **必须在程序退出时调用 `cleanup_desktop_registry()` 清理**。
pub fn setup_desktop_registry() {
    log::info!("[MENU-DEBUG] setup_desktop_registry 入口");
    let exe_path = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            log::error!("注册表设置失败: 无法获取可执行文件路径: {}", e);
            return;
        }
    };

    // 图标路径：使用可执行文件本身（内嵌 logo.ico 资源）
    let icon_path = format_registry_icon_path(&exe_path);

    unsafe {
        // [DEBUG] 检查注册表项是否已存在
        for check_item in desktop_registry_menu_items() {
            let check_path = format!(
                "Software\\Classes\\Directory\\Background\\shell\\{}\0",
                check_item.key_name
            );
            let check_wide: Vec<u16> = check_path.encode_utf16().collect();
            let mut check_hkey: isize = 0;
            let ret = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                check_wide.as_ptr(),
                0,
                KEY_READ,
                &mut check_hkey,
            );
            if ret == 0 {
                log::warn!(
                    "[MENU-DEBUG] 注册表项已存在(可能上次未清理): {}",
                    check_item.key_name
                );
                RegCloseKey(check_hkey);
            } else {
                log::info!(
                    "[MENU-DEBUG] 注册表项不存在(正常首次注册): {}",
                    check_item.key_name
                );
            }
        }
        for item in desktop_registry_menu_items() {
            let key_name = item.key_name;
            let menu_text = item.menu_text;
            let cmd_arg = item.cmd_arg;
            let base_path = format!(
                "Software\\Classes\\Directory\\Background\\shell\\{}\0",
                key_name
            );
            let cmd_path = format!(
                "Software\\Classes\\Directory\\Background\\shell\\{}\\command\0",
                key_name
            );

            let key_path: Vec<u16> = base_path.encode_utf16().collect();
            let cmd_key_path: Vec<u16> = cmd_path.encode_utf16().collect();
            let text: Vec<u16> = format!("{}\0", menu_text).encode_utf16().collect();
            let icon: Vec<u16> = format!("{}\0", icon_path).encode_utf16().collect();
            let command: Vec<u16> = format!("\"{}\" {}\0", exe_path, cmd_arg)
                .encode_utf16()
                .collect();

            // 1. 创建菜单项键并设置显示文本
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
                log::error!("注册表: {} 创建失败, 错误码={}", key_name, ret);
                continue;
            }

            // 设置默认值（显示文本）
            RegSetValueExW(
                hkey,
                std::ptr::null(),
                0,
                REG_SZ,
                text.as_ptr() as *const u8,
                (text.len() * 2) as u32,
            );

            // 设置图标（Icon 值指向嵌入图标的 exe）
            let icon_name: Vec<u16> = "Icon\0".encode_utf16().collect();
            RegSetValueExW(
                hkey,
                icon_name.as_ptr(),
                0,
                REG_SZ,
                icon.as_ptr() as *const u8,
                (icon.len() * 2) as u32,
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
                log::error!("注册表: {} command 子键创建失败, 错误码={}", key_name, ret);
                continue;
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

            log::info!(
                "[MENU-DEBUG] 注册表: {} 已注册 (menu_text=\"{}\",  cmd_arg=\"{}\")",
                key_name,
                menu_text,
                cmd_arg
            );
        }

        log::info!(
            "注册表: 桌面右键菜单已注册 ({} 项)",
            desktop_registry_menu_items().len()
        );

        // 通知 Windows Shell 刷新关联缓存，确保桌面右键菜单立即显示新图标
        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
        log::info!("[MENU-DEBUG] 已发送 SHChangeNotify(SHCNE_ASSOCCHANGED) 刷新 Shell 缓存");
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
    log::info!("[MENU-DEBUG] cleanup_desktop_registry 入口");
    let key_names = desktop_registry_menu_items()
        .iter()
        .map(|item| item.key_name);

    unsafe {
        for key_name in key_names {
            let cmd_key: Vec<u16> = format!(
                "Software\\Classes\\Directory\\Background\\shell\\{}\\command\0",
                key_name
            )
            .encode_utf16()
            .collect();
            let key: Vec<u16> = format!(
                "Software\\Classes\\Directory\\Background\\shell\\{}\0",
                key_name
            )
            .encode_utf16()
            .collect();

            let ret1 = RegDeleteKeyW(HKEY_CURRENT_USER, cmd_key.as_ptr());
            let ret2 = RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr());

            if ret1 == 0 || ret2 == 0 {
                log::info!(
                    "[MENU-DEBUG] 已清理注册表项: {} (ret_cmd={}, ret_key={})",
                    key_name,
                    ret1,
                    ret2
                );
            }
        }
        log::info!("[MENU-DEBUG] 注册表清理完成");

        // 通知 Shell 刷新缓存
        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
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

    #[test]
    fn test_desktop_registry_menu_items_cover_all_signal_actions() {
        let items = desktop_registry_menu_items();
        assert_eq!(items.len(), 4);

        let key_names: Vec<&str> = items.iter().map(|item| item.key_name).collect();
        assert_eq!(
            key_names,
            vec![
                "IkunOpenPanel",
                "IkunCreateBox",
                "IkunAiClassify",
                "IkunQuit"
            ]
        );

        let cmd_args: Vec<&str> = items.iter().map(|item| item.cmd_arg).collect();
        assert_eq!(
            cmd_args,
            vec!["--open-panel", "--create-box", "--ai-classify", "--quit"]
        );
    }

    #[test]
    fn test_desktop_registry_menu_text_is_localized() {
        let labels: Vec<&str> = desktop_registry_menu_items()
            .iter()
            .map(|item| item.menu_text)
            .collect();

        assert_eq!(
            labels,
            vec![
                "打开控制面板(&O)",
                "创建分类框(&N)",
                "一键AI分类(&A)",
                "退出应用(&Q)"
            ]
        );
    }

    #[test]
    fn test_resolve_shell_menu_path_keeps_existing_logical_path() {
        let resolved = resolve_shell_menu_path_with(
            r"C:\Users\chenzhihui\Desktop\CC Switch.lnk",
            |_| true,
            |_| {
                Some(
                    r"C:\Users\chenzhihui\.ikun_organizer\managed_icons\user__CC Switch.lnk"
                        .to_string(),
                )
            },
        );

        assert_eq!(resolved, r"C:\Users\chenzhihui\Desktop\CC Switch.lnk");
    }

    #[test]
    fn test_resolve_shell_menu_path_uses_vault_when_logical_path_missing() {
        let resolved = resolve_shell_menu_path_with(
            r"C:\Users\chenzhihui\Desktop\CC Switch.lnk",
            |_| false,
            |_| {
                Some(
                    r"C:\Users\chenzhihui\.ikun_organizer\managed_icons\user__CC Switch.lnk"
                        .to_string(),
                )
            },
        );

        assert_eq!(
            resolved,
            r"C:\Users\chenzhihui\.ikun_organizer\managed_icons\user__CC Switch.lnk"
        );
    }

    #[test]
    fn test_resolve_shell_menu_path_falls_back_to_logical_path() {
        let resolved = resolve_shell_menu_path_with(
            r"C:\Users\chenzhihui\Desktop\Missing.lnk",
            |_| false,
            |_| None,
        );

        assert_eq!(resolved, r"C:\Users\chenzhihui\Desktop\Missing.lnk");
    }

    // ===== Vault path proxy tests (分类框图标右键代理到 vault) =====

    #[test]
    fn test_vault_proxy_user_desktop_icon_in_vault() {
        // 用户桌面图标已分类到框中（logical_path 不在桌面，vault 路径存在）
        // 应直接返回 vault 路径，不导出到桌面
        let resolved = resolve_shell_menu_path_with(
            r"C:\Users\chenzhihui\Desktop\Chrome.lnk",
            |_| false, // logical_path 不在桌面（已隐藏）
            |path| {
                // resolve_icon_read_path 返回 vault 路径
                if path == r"C:\Users\chenzhihui\Desktop\Chrome.lnk" {
                    Some(
                        r"C:\Users\chenzhihui\.ikun_organizer\managed_icons\user__Chrome.lnk"
                            .to_string(),
                    )
                } else {
                    None
                }
            },
        );

        // 应返回 vault 路径（直接使用，不导出）
        assert_eq!(
            resolved,
            r"C:\Users\chenzhihui\.ikun_organizer\managed_icons\user__Chrome.lnk"
        );
    }

    #[test]
    fn test_vault_proxy_public_desktop_icon_in_vault() {
        // 公共桌面图标已分类到框中（logical_path 不在桌面，public vault 路径存在）
        let resolved = resolve_shell_menu_path_with(
            r"C:\Users\Public\Desktop\Notepad.lnk",
            |_| false, // logical_path 不在桌面（已隐藏）
            |path| {
                // resolve_icon_read_path 返回 public vault 路径
                if path == r"C:\Users\Public\Desktop\Notepad.lnk" {
                    Some(
                        r"C:\Users\Public\.ikun_organizer\managed_icons\public__Notepad.lnk"
                            .to_string(),
                    )
                } else {
                    None
                }
            },
        );

        // 应返回 public vault 路径
        assert_eq!(
            resolved,
            r"C:\Users\Public\.ikun_organizer\managed_icons\public__Notepad.lnk"
        );
    }

    #[test]
    fn test_vault_proxy_icon_still_on_desktop() {
        // 图标仍在桌面上（未被隐藏到 vault）
        // logical_path 存在，应直接使用 logical_path
        let resolved = resolve_shell_menu_path_with(
            r"C:\Users\chenzhihui\Desktop\VSCode.lnk",
            |_| true, // logical_path 在桌面
            |_| {
                // vault 路径不存在
                None
            },
        );

        // 应返回原始 logical_path
        assert_eq!(resolved, r"C:\Users\chenzhihui\Desktop\VSCode.lnk");
    }

    #[test]
    fn test_vault_proxy_virtual_shell_object() {
        // 虚拟 Shell 对象（如回收站、此电脑）
        // 这些不是真实文件，路径以 ::{ 开头
        let virtual_path = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"; // 此电脑
        let resolved = resolve_shell_menu_path_with(
            virtual_path,
            |_| false, // 虚拟对象不 "存在" 于文件系统
            |_| None,  // 无 vault 路径
        );

        // 应返回原始虚拟路径
        assert_eq!(resolved, virtual_path);
    }
    // ===== ShellMenuResult fallback guard tests =====

    #[test]
    fn test_should_skip_fallback_returns_false_for_cancelled() {
        // Cancelled = menu failed to show -> should NOT skip fallback (i.e. should fall back)
        assert!(!should_skip_fallback(&ShellMenuResult::Cancelled));
    }

    #[test]
    fn test_should_skip_fallback_returns_true_for_dismissed() {
        // Dismissed = menu was shown but user closed it -> should skip fallback
        assert!(should_skip_fallback(&ShellMenuResult::Dismissed));
    }

    #[test]
    fn test_should_skip_fallback_returns_true_for_completed() {
        // Completed = user selected a menu item -> should skip fallback
        assert!(should_skip_fallback(&ShellMenuResult::Completed(Some(
            "open".to_string()
        ))));
        assert!(should_skip_fallback(&ShellMenuResult::Completed(None)));
    }

    #[test]
    fn test_should_skip_fallback_returns_true_for_needs_rename() {
        assert!(should_skip_fallback(&ShellMenuResult::NeedsRename(
            "C:/test.lnk".to_string()
        )));
    }

    #[test]
    fn test_should_skip_fallback_returns_true_for_needs_cut_monitor() {
        assert!(should_skip_fallback(&ShellMenuResult::NeedsCutMonitor(
            "C:/test.lnk".to_string()
        )));
    }

    #[test]
    fn test_should_skip_fallback_returns_true_for_needs_delete() {
        assert!(should_skip_fallback(&ShellMenuResult::NeedsDelete(
            "C:/test.lnk".to_string()
        )));
    }

    #[test]
    fn test_dismissed_is_distinct_from_cancelled() {
        // Dismissed and Cancelled are different variants
        let dismissed = ShellMenuResult::Dismissed;
        let cancelled = ShellMenuResult::Cancelled;

        // matches! patterns distinguish them
        assert!(matches!(dismissed, ShellMenuResult::Dismissed));
        assert!(!matches!(dismissed, ShellMenuResult::Cancelled));
        assert!(matches!(cancelled, ShellMenuResult::Cancelled));
        assert!(!matches!(cancelled, ShellMenuResult::Dismissed));

        // The fallback guard correctly differentiates them
        assert!(should_skip_fallback(&dismissed));
        assert!(!should_skip_fallback(&cancelled));
    }

    #[test]
    fn test_all_variants_cover_fallback_guard() {
        // Exhaustive check: every non-Cancelled variant skips fallback
        let results = vec![
            ShellMenuResult::Dismissed,
            ShellMenuResult::Completed(Some("explore".to_string())),
            ShellMenuResult::Completed(None),
            ShellMenuResult::NeedsRename("path".to_string()),
            ShellMenuResult::NeedsCutMonitor("path".to_string()),
            ShellMenuResult::NeedsDelete("path".to_string()),
        ];

        for result in &results {
            assert!(
                should_skip_fallback(result),
                "Expected skip for {:?}",
                result
            );
        }

        // Only Cancelled triggers fallback
        assert!(!should_skip_fallback(&ShellMenuResult::Cancelled));
    }

    #[test]
    fn test_format_registry_icon_path_no_quotes() {
        // Windows 注册表 Icon 值不允许带引号
        let path = format_registry_icon_path(r"C:\Program Files\app\ikun.exe");
        assert_eq!(path, r"C:\Program Files\app\ikun.exe,0");
        assert!(!path.contains('"'), "注册表 Icon 值不应包含引号");
    }

    #[test]
    fn test_format_registry_icon_path_simple_path() {
        let path = format_registry_icon_path(r"D:\tools\ikun_organizer.exe");
        assert_eq!(path, r"D:\tools\ikun_organizer.exe,0");
    }

    #[test]
    fn test_format_registry_icon_path_with_spaces() {
        // 路径含空格时也不应加引号
        let path = format_registry_icon_path(r"C:\Users\John Doe\app.exe");
        assert_eq!(path, r"C:\Users\John Doe\app.exe,0");
        assert!(!path.starts_with('"'));
    }

    #[test]
    fn test_shcne_constants_values() {
        // SHCNE_ASSOCCHANGED 和 SHCNF_IDLIST 的值必须与 Windows SDK 定义一致
        assert_eq!(SHCNE_ASSOCCHANGED, 0x08000000_u32);
        assert_eq!(SHCNF_IDLIST, 0x0000_u32);
    }
}
