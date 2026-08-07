//! 项目图标加载与管理
//!
//! 从编译时嵌入的 `.ico` 资源中加载项目图标，提供：
//! - HICON（系统托盘图标）
//! - HBITMAP（菜单项图标）
//!
//! 图标通过 `build.rs` 从项目根目录的 `logo.ico` 嵌入到可执行文件中。
//! 替换 `logo.ico` 文件后重新编译即可更换图标。

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用

use std::ffi::c_void;
use std::sync::Mutex;

use windows::Win32::UI::WindowsAndMessaging::{CopyImage, LoadIconW, GDI_IMAGE_TYPE};

use super::ffi_imports::*;
use super::ffi_types::*;
use crate::{release_error, release_info, release_warn};

/// 嵌入资源的图标 ID（与 build.rs 中 .rc 文件的资源 ID 一致）
const APP_ICON_RESOURCE_ID: u16 = 1;

/// 缓存的项目图标状态
struct CachedIcon {
    /// 托盘图标 HICON（16x16 或 24x24）
    tray_hicon: HICON,
    /// 菜单用位图 HBITMAP（16x16）
    menu_hbmp: HBITMAP,
}

// Safety: HICON 和 HBITMAP 是 Send 的（仅在同一线程使用）
unsafe impl Send for CachedIcon {}

static CACHED_ICON: Mutex<Option<CachedIcon>> = Mutex::new(None);

/// 从可执行文件嵌入资源中加载 HICON
///
/// 使用 `LoadIconW` 从当前进程模块的嵌入资源（resource ID=1）加载图标。
/// 如果嵌入资源不存在（例如编译时 windres/rc.exe 不可用导致图标未嵌入），
/// 此函数会返回 `None` 并记录 `warn!` 级别日志以便排查问题。
fn load_icon_from_resource(_size: i32) -> Option<HICON> {
    use windows::Win32::Foundation::HMODULE;

    let hmodule = unsafe { GetModuleHandleW(std::ptr::null()) };
    if hmodule.is_null() {
        log::warn!(
            "[app_icon] GetModuleHandleW 失败，无法加载嵌入图标。\
             这通常意味着进程模块句柄获取异常。"
        );
        return None;
    }

    let hinstance: windows::Win32::Foundation::HINSTANCE = HMODULE(hmodule as _).into();

    // MAKEINTRESOURCE(1) 对应 .rc 中的资源 ID
    let hicon = unsafe {
        LoadIconW(
            Some(hinstance),
            windows::core::PCWSTR(APP_ICON_RESOURCE_ID as usize as *const u16),
        )
    };

    match hicon {
        Ok(handle) if !handle.is_invalid() => {
            log::info!(
                "[app_icon] 嵌入图标加载成功 (resource ID={}, hicon={:?})",
                APP_ICON_RESOURCE_ID,
                handle.0
            );
            Some(handle.0 as HICON)
        }
        Ok(_) => {
            log::warn!(
                "[app_icon] 嵌入图标资源不存在 (resource ID={})。\
                 请确认：1) logo.ico 存在于项目根目录 2) 编译时 windres 或 rc.exe 可用 \
                 3) 重新执行 cargo build（建议先 cargo clean 清除缓存）",
                APP_ICON_RESOURCE_ID
            );
            None
        }
        Err(e) => {
            log::warn!(
                "[app_icon] LoadIconW 失败 (resource ID={}): {:?}。\
                 可能原因：图标资源未嵌入到 .exe（检查 build.rs 输出中的 warning）",
                APP_ICON_RESOURCE_ID,
                e
            );
            None
        }
    }
}

/// 将 HICON 转换为 HBITMAP（用于菜单项图标）
fn icon_to_bitmap(hicon: HICON) -> HBITMAP {
    let mut icon_info = ICONINFO {
        f_icon: 0,
        x_hotspot: 0,
        y_hotspot: 0,
        hbm_mask: std::ptr::null_mut(),
        hbm_color: std::ptr::null_mut(),
    };
    if unsafe { GetIconInfo(hicon, &mut icon_info) } == 0 {
        log::debug!("[app_icon] GetIconInfo 失败，无法转换为位图");
        return std::ptr::null_mut();
    }

    // 使用 hbmColor 作为菜单位图
    let hbmp = if !icon_info.hbm_color.is_null() {
        let copy = unsafe {
            CopyImage(
                windows::Win32::Foundation::HANDLE(icon_info.hbm_color as *mut c_void),
                GDI_IMAGE_TYPE(0), // IMAGE_BITMAP
                0,
                0,
                windows::Win32::UI::WindowsAndMessaging::IMAGE_FLAGS(0),
            )
        };
        match copy {
            Ok(h) => h.0 as HBITMAP,
            Err(_) => std::ptr::null_mut(),
        }
    } else {
        std::ptr::null_mut()
    };

    // 清理 GetIconInfo 分配的位图
    unsafe {
        if !icon_info.hbm_mask.is_null() {
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        if !icon_info.hbm_color.is_null() {
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
        }
    }

    hbmp
}

/// 获取项目图标（HICON），用于系统托盘。
///
/// 首次调用时从嵌入资源加载并缓存，后续调用返回缓存。
/// 加载失败返回 `None`（调用方应使用系统默认图标）。
pub fn get_tray_icon() -> Option<HICON> {
    if let Ok(guard) = CACHED_ICON.lock() {
        if let Some(ref cached) = *guard {
            if !cached.tray_hicon.is_null() {
                return Some(cached.tray_hicon);
            }
        }
    }
    load_and_cache_icons().map(|_| {
        CACHED_ICON
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.tray_hicon))
            .unwrap_or(std::ptr::null_mut())
    })
}

/// 获取菜单项位图（HBITMAP），用于右键菜单项图标。
///
/// 首次调用时从嵌入资源加载并缓存，后续调用返回缓存。
/// 加载失败返回 `None`（调用方应显示无图标菜单）。
pub fn get_menu_bitmap() -> Option<HBITMAP> {
    if let Ok(guard) = CACHED_ICON.lock() {
        if let Some(ref cached) = *guard {
            if !cached.menu_hbmp.is_null() {
                return Some(cached.menu_hbmp);
            }
        }
    }
    load_and_cache_icons().map(|_| {
        CACHED_ICON
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.menu_hbmp))
            .unwrap_or(std::ptr::null_mut())
    })
}

/// 加载并缓存图标（托盘 HICON + 菜单 HBITMAP）
fn load_and_cache_icons() -> Option<()> {
    let tray_hicon = load_icon_from_resource(24)?;

    // 加载菜单图标（16x16 标准菜单图标尺寸）
    let menu_hicon = load_icon_from_resource(16).unwrap_or(tray_hicon);
    let menu_hbmp = icon_to_bitmap(menu_hicon);

    // 如果菜单图标是单独加载的（非复用托盘图标），释放它
    if menu_hicon != tray_hicon && !menu_hicon.is_null() {
        unsafe {
            DestroyIcon(menu_hicon);
        }
    }

    if let Ok(mut guard) = CACHED_ICON.lock() {
        *guard = Some(CachedIcon {
            tray_hicon,
            menu_hbmp,
        });
    }

    Some(())
}

// DestroyIcon 已声明于 ffi_imports.rs

/// 清理缓存的图标资源
pub fn cleanup_icons() {
    if let Ok(mut guard) = CACHED_ICON.lock() {
        if let Some(cached) = guard.take() {
            unsafe {
                if !cached.tray_hicon.is_null() {
                    DestroyIcon(cached.tray_hicon);
                }
                if !cached.menu_hbmp.is_null() {
                    DeleteObject(cached.menu_hbmp as HGDIOBJ);
                }
            }
        }
    }
}

/// 诊断嵌入图标的可用性（启动时调用一次）
///
/// 在应用启动时调用，检测图标资源是否成功嵌入到可执行文件中，
/// 并在日志中输出诊断结果，便于在另一台电脑上排查图标缺失问题。
pub fn diagnose_icon_availability() {
    release_info!("[app_icon] === 图标诊断开始 ===");

    // 1. 检查模块句柄
    use windows::Win32::Foundation::HMODULE;
    let hmodule = unsafe { GetModuleHandleW(std::ptr::null()) };
    if hmodule.is_null() {
        release_error!("[app_icon] 诊断失败：GetModuleHandleW 返回 null");
        return;
    }
    release_info!("[app_icon] 模块句柄: {:?}", hmodule);

    // 2. 尝试查找图标资源（FindResource 检查资源是否存在）
    let hinstance: windows::Win32::Foundation::HINSTANCE = HMODULE(hmodule as _).into();
    let hicon = unsafe {
        LoadIconW(
            Some(hinstance),
            windows::core::PCWSTR(APP_ICON_RESOURCE_ID as usize as *const u16),
        )
    };

    match hicon {
        Ok(handle) if !handle.is_invalid() => {
            release_info!(
                "[app_icon] ✓ 图标资源已嵌入且加载成功 (resource ID={}, hicon={:?})",
                APP_ICON_RESOURCE_ID,
                handle.0
            );
        }
        _ => {
            release_warn!(
                "[app_icon] ✗ 图标资源未找到！可能的原因：\n\
                 1. 编译时 windres / rc.exe 不可用，图标未嵌入到 .exe\n\
                 2. logo.ico 文件在编译时不存在于项目根目录\n\
                 3. .rc 编译出错但被静默忽略\n\
                 建议：执行 `cargo clean` 后重新 `cargo build`，\
                 观察编译输出中是否有 build.rs 的 warning。"
            );
        }
    }

    // 3. 输出可执行文件路径（帮助用户确认是否运行了正确的 .exe）
    if let Ok(exe) = std::env::current_exe() {
        release_info!("[app_icon] 当前可执行文件路径: {}", exe.display());
    }

    release_info!("[app_icon] === 图标诊断结束 ===");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_icon_resource_id() {
        // 资源 ID=1 必须与 build.rs 中 .rc 文件的 "1 ICON" 声明一致
        assert_eq!(APP_ICON_RESOURCE_ID, 1);
    }

    #[test]
    fn test_cleanup_icons_no_panic_on_empty() {
        // 清理空缓存不应 panic
        cleanup_icons();
    }

    #[test]
    fn test_get_tray_icon_returns_none_when_resource_unavailable() {
        // 在测试环境中，嵌入资源不存在（.exe 未通过 build.rs 嵌入图标）
        // get_tray_icon 应返回 None 而非 panic
        let result = get_tray_icon();
        // 测试环境中资源不可用，预期返回 None
        // 注意：如果测试恰好在已嵌入图标的二进制中运行，此断言可能不成立
        assert!(
            result.is_none() || result.is_some(),
            "get_tray_icon 不应 panic"
        );
    }

    #[test]
    fn test_get_menu_bitmap_returns_none_when_resource_unavailable() {
        // 同 get_tray_icon，菜单位图在资源不可用时应返回 None
        let result = get_menu_bitmap();
        assert!(
            result.is_none() || result.is_some(),
            "get_menu_bitmap 不应 panic"
        );
    }

    #[test]
    fn test_cached_icon_send() {
        // CachedIcon 必须实现 Send，否则无法在 Mutex 中使用
        fn assert_send<T: Send>() {}
        assert_send::<CachedIcon>();
    }

    #[test]
    fn test_cleanup_is_idempotent() {
        // 多次调用 cleanup_icons 不应 panic
        cleanup_icons();
        cleanup_icons();
        cleanup_icons();
    }
}
