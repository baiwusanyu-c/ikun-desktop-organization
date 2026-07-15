//! 隐藏/恢复原生桌面图标 + 图标仓库（Vault）

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! # 隐藏方式
//!
//! 通过 `ShowWindow(SW_HIDE)` 隐藏整个 SysListView32 窗口，
//! 原子操作，无闪烁。退出时 `ShowWindow(SW_SHOW)` 恢复。
//!
//! # 仓库（Vault）
//!
//! 拖拽操作使用文件仓库管理图标：
//! - 用户桌面 `.lnk` → **移动**到 `~/.ikun_organizer/managed_icons/`
//! - 公共桌面 `.lnk` → **复制**到仓库（原文件保留）
//! - 拖回桌面 → 从仓库导出到用户桌面

use anyhow::Result;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 跨卷移动文件（先尝试 rename，失败后回退到 copy + delete）
///
/// `std::fs::rename` 在源和目标位于不同卷时会失败（Windows 错误码 17: ERROR_NOT_SAME_DEVICE）。
/// 此函数提供可靠回退：先尝试 rename，失败时改用 copy + remove_file。
fn move_file(src: &Path, dst: &Path) -> Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Windows ERROR_NOT_SAME_DEVICE = 17, 跨卷移动时触发
            let is_cross_device = e.raw_os_error() == Some(17);
            if is_cross_device {
                log::info!(
                    "跨卷移动文件 (rename 失败), 回退到 copy + delete: {} -> {}",
                    src.display(),
                    dst.display()
                );
                std::fs::copy(src, dst).map_err(|copy_err| {
                    anyhow::anyhow!(
                        "跨卷复制失败 {} -> {}: {}",
                        src.display(),
                        dst.display(),
                        copy_err
                    )
                })?;
                std::fs::remove_file(src).map_err(|rm_err| {
                    anyhow::anyhow!("复制后删除源文件失败 {}: {}", src.display(), rm_err)
                })?;
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "移动文件失败 {} -> {}: {}",
                    src.display(),
                    dst.display(),
                    e
                ))
            }
        }
    }
}

// =============================================================================
// ShowWindow 隐藏/恢复（e81c7a0 方式）
// =============================================================================

type HWND = *mut c_void;

const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;

extern "system" {
    fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> HWND;
    fn FindWindowExW(
        hWndParent: HWND,
        hWndChildAfter: HWND,
        lpClassName: *const u16,
        lpWindowName: *const u16,
    ) -> HWND;
    fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> i32;
    fn IsWindow(hWnd: HWND) -> i32;
    fn IsWindowVisible(hWnd: HWND) -> i32;
}

static PROG_CLASS: &[u16] = &[80, 114, 111, 103, 109, 97, 110, 0]; // "Progman"
static WORKER_CLASS: &[u16] = &[87, 111, 114, 107, 101, 114, 87, 0]; // "WorkerW"
static DEFVIEW_CLASS: &[u16] = &[
    83, 72, 69, 76, 76, 68, 76, 76, 95, 68, 101, 102, 86, 105, 101, 119, 0,
]; // "SHELLDLL_DefView"
static LISTVIEW_CLASS: &[u16] = &[
    83, 121, 115, 76, 105, 115, 116, 86, 105, 101, 119, 51, 50, 0,
]; // "SysListView32"

/// 查找桌面 SysListView32 句柄（raw FFI，已验证可用）
pub fn find_desktop_listview() -> Result<HWND> {
    unsafe {
        // 尝试 Progman
        let progman = FindWindowW(PROG_CLASS.as_ptr(), std::ptr::null());
        if !progman.is_null() {
            let defview = FindWindowExW(
                progman,
                std::ptr::null_mut(),
                DEFVIEW_CLASS.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    LISTVIEW_CLASS.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Ok(lv);
                }
            }
        }

        // 尝试 WorkerW
        let mut worker = FindWindowW(WORKER_CLASS.as_ptr(), std::ptr::null());
        while !worker.is_null() {
            let defview = FindWindowExW(
                worker,
                std::ptr::null_mut(),
                DEFVIEW_CLASS.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    LISTVIEW_CLASS.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Ok(lv);
                }
            }
            worker = FindWindowExW(
                std::ptr::null_mut(),
                worker,
                WORKER_CLASS.as_ptr(),
                std::ptr::null(),
            );
        }

        anyhow::bail!("无法找到 SysListView32")
    }
}

/// IconHider 全局状态
struct IconHider {
    listview_hwnd: HWND,
    hidden: bool,
}

// Safety: HWND 是 Send 的（只在同一线程使用）
unsafe impl Send for IconHider {}

static ICON_HIDER: Mutex<Option<IconHider>> = Mutex::new(None);

/// 隐藏所有原生桌面图标
///
/// 通过 `ShowWindow(SW_HIDE)` 隐藏整个 SysListView32 窗口，
/// 原子操作，不会产生闪烁。
pub fn hide_native_icons() -> Result<()> {
    let lv_hwnd = find_desktop_listview()?;

    unsafe {
        let before_valid = IsWindow(lv_hwnd);
        log::info!(
            "准备隐藏桌面 SysListView32 窗口: hwnd={:?}, valid={}",
            lv_hwnd,
            before_valid
        );
        let result = ShowWindow(lv_hwnd, SW_HIDE);
        log::info!(
            "已隐藏桌面 SysListView32 窗口: hwnd={:?}, ShowWindowResult={}",
            lv_hwnd,
            result
        );
    }

    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;
    *guard = Some(IconHider {
        listview_hwnd: lv_hwnd,
        hidden: true,
    });

    Ok(())
}

/// 检查原生桌面图标窗口是否仍处于隐藏状态。
///
/// Explorer 重启后旧的 SysListView32 句柄会失效，此时返回 false，
/// 调用方应重新执行 `hide_native_icons()` 绑定新的桌面列表窗口。
pub fn native_icons_hidden() -> bool {
    let guard = match ICON_HIDER.lock() {
        Ok(guard) => guard,
        Err(e) => {
            log::warn!("检查桌面图标隐藏状态失败，锁获取失败: {}", e);
            return false;
        }
    };

    let Some(hider) = guard.as_ref() else {
        return false;
    };
    if !hider.hidden {
        return false;
    }

    let valid = unsafe { IsWindow(hider.listview_hwnd) } != 0;
    if !valid {
        return false;
    }
    unsafe { IsWindowVisible(hider.listview_hwnd) == 0 }
}

/// 恢复原生桌面图标
///
/// 通过 `ShowWindow(SW_SHOW)` 恢复 SysListView32 窗口。
pub fn restore_native_icons() -> Result<()> {
    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;

    if let Some(hider) = guard.take() {
        if !hider.hidden {
            return Ok(());
        }

        let valid = unsafe { IsWindow(hider.listview_hwnd) };
        if valid != 0 {
            unsafe {
                let result = ShowWindow(hider.listview_hwnd, SW_SHOW);
                log::info!(
                    "已恢复桌面 SysListView32 窗口: hwnd={:?}, ShowWindowResult={}",
                    hider.listview_hwnd,
                    result
                );
            }
        } else {
            log::warn!("SysListView32 窗口已失效，跳过恢复");
        }
    }

    Ok(())
}

// =============================================================================
// 图标仓库（Vault）— 拖拽操作使用
// =============================================================================

/// 仓库目录名
const MANAGED_DIR_NAME: &str = "managed_icons";

/// 旧版隐藏目录（启动时自动迁移到 managed_icons）
const LEGACY_HIDDEN_DIR_NAME: &str = "hidden_icons";

const PREFIX_USER: &str = "user__";
#[allow(dead_code)]
const PREFIX_PUBLIC: &str = "public__";

const SHCNE_UPDATEITEM: u32 = 0x0000_2000;
const SHCNE_CREATE: u32 = 0x0000_0002;
const SHCNF_PATHW: u32 = 0x0000_8000;
const SHCNF_FLUSH: u32 = 0x0000_1000;

fn app_data_root() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("无法获取用户主目录"))?;
    Ok(home.join(".ikun_organizer"))
}

fn get_user_desktop() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Desktop"))
}

fn get_public_desktop() -> PathBuf {
    PathBuf::from(r"C:\Users\Public\Desktop")
}

fn get_managed_dir() -> Result<PathBuf> {
    let dir = app_data_root()?.join(MANAGED_DIR_NAME);
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("创建仓库目录失败 {}: {}", dir.display(), e))?;
    }
    migrate_legacy_hidden_dir(&dir)?;
    Ok(dir)
}

/// 将旧版 hidden_icons/ 内容迁移到 managed_icons/
fn migrate_legacy_hidden_dir(managed_dir: &Path) -> Result<()> {
    let legacy = app_data_root()?.join(LEGACY_HIDDEN_DIR_NAME);
    if !legacy.exists() {
        return Ok(());
    }
    let entries = match std::fs::read_dir(&legacy) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    let mut migrated = 0;
    for entry in entries.flatten() {
        let src = entry.path();
        if !src.is_file() {
            continue;
        }
        let name = match src.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let dest = managed_dir.join(&name);
        if dest.exists() {
            let _ = std::fs::remove_file(&src);
            continue;
        }
        if std::fs::rename(&src, &dest).is_ok() {
            migrated += 1;
        }
    }
    if migrated > 0 {
        log::info!(
            "已从旧版 hidden_icons 迁移 {} 个图标到 managed_icons",
            migrated
        );
    }
    Ok(())
}

pub fn is_public_desktop_path(file_path: &str) -> bool {
    Path::new(file_path).starts_with(get_public_desktop())
}

#[allow(dead_code)]
pub fn is_user_desktop_path(file_path: &str) -> bool {
    get_user_desktop()
        .map(|d| Path::new(file_path).starts_with(&d))
        .unwrap_or(false)
}

fn vault_filename(logical_path: &str) -> Option<String> {
    let file_name = Path::new(logical_path)
        .file_name()?
        .to_string_lossy()
        .to_string();
    if is_user_desktop_path(logical_path) {
        return Some(format!("{}{}", PREFIX_USER, file_name));
    }
    if is_public_desktop_path(logical_path) {
        return Some(format!("{}{}", PREFIX_PUBLIC, file_name));
    }
    Some(format!("{}{}", PREFIX_USER, file_name))
}

fn vault_path_for(logical_path: &str) -> Result<PathBuf> {
    let name = vault_filename(logical_path)
        .ok_or_else(|| anyhow::anyhow!("无法生成仓库文件名: {}", logical_path))?;
    Ok(get_managed_dir()?.join(name))
}

/// 导出到用户桌面的目标路径
pub fn export_desktop_path(logical_path: &str) -> PathBuf {
    if is_user_desktop_path(logical_path) {
        return PathBuf::from(logical_path);
    }
    let file_name = Path::new(logical_path)
        .file_name()
        .map(|n| n.to_owned())
        .unwrap_or_else(|| std::ffi::OsStr::new("shortcut.lnk").to_owned());
    get_user_desktop()
        .map(|d| d.join(file_name))
        .unwrap_or_else(|| PathBuf::from(logical_path))
}

fn logical_path_from_vault_name(vault_name: &str) -> Option<PathBuf> {
    if let Some(file_name) = vault_name.strip_prefix(PREFIX_USER) {
        get_user_desktop().map(|d| d.join(file_name))
    } else if let Some(file_name) = vault_name.strip_prefix(PREFIX_PUBLIC) {
        Some(get_public_desktop().join(file_name))
    } else {
        get_user_desktop().map(|d| d.join(vault_name))
    }
}

fn notify_shell_path(path: &str) {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = Path::new(path)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        super::ffi_imports::SHChangeNotify(
            SHCNE_CREATE | SHCNE_UPDATEITEM,
            SHCNF_PATHW | SHCNF_FLUSH,
            wide.as_ptr() as *const _,
            std::ptr::null(),
        );
    }
}

// =============================================================================
// Vault 公开 API（拖拽操作使用）
// =============================================================================

/// 图标是否在仓库或桌面上可读
pub fn icon_exists_for_box(logical_path: &str) -> bool {
    // 虚拟 Shell 对象（回收站、此电脑等）始终视为存在
    if logical_path.starts_with("::{") || logical_path.starts_with("shell::virtual::") {
        return true;
    }
    if Path::new(logical_path).exists() {
        return true;
    }
    vault_path_for(logical_path)
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// 读取图标文件路径：优先仓库，其次桌面
pub fn resolve_icon_read_path(logical_path: &str) -> Option<PathBuf> {
    if let Ok(vault) = vault_path_for(logical_path) {
        if vault.exists() {
            return Some(vault);
        }
    }
    let p = Path::new(logical_path);
    if p.exists() {
        Some(p.to_path_buf())
    } else {
        None
    }
}

/// 收入仓库：用户桌面移动，公共桌面只读复制
pub fn ingest_icon(logical_path: &str) -> Result<()> {
    // 虚拟 Shell 对象（回收站、此电脑等）无需收入仓库，它们始终存在
    if logical_path.starts_with("::{") || logical_path.starts_with("shell::virtual::") {
        return Ok(());
    }
    let vault = vault_path_for(logical_path)?;
    let src = Path::new(logical_path);

    if vault.exists() {
        if is_user_desktop_path(logical_path) && src.exists() {
            if let Err(e) = std::fs::remove_file(src) {
                log::warn!("清理桌面残留副本失败 {}: {}", logical_path, e);
            } else {
                notify_shell_path(logical_path);
                log::info!("已清理桌面残留副本: {}", logical_path);
            }
        }
        return Ok(());
    }

    if !src.exists() {
        log::debug!("ingest: 源文件不存在，跳过: {}", logical_path);
        return Ok(());
    }

    if is_public_desktop_path(logical_path) {
        std::fs::copy(src, &vault)
            .map_err(|e| anyhow::anyhow!("复制公共桌面图标到仓库失败 {}: {}", logical_path, e))?;
        log::info!(
            "公共桌面图标已复制到仓库: {} → {}",
            logical_path,
            vault.display()
        );
        return Ok(());
    }

    std::fs::rename(src, &vault)
        .map_err(|e| anyhow::anyhow!("移动图标到仓库失败 {}: {}", logical_path, e))?;
    notify_shell_path(logical_path);
    log::info!("图标已收入仓库: {} → {}", logical_path, vault.display());
    Ok(())
}

/// 从仓库导出到用户桌面
pub fn export_icon(logical_path: &str) -> Result<(bool, PathBuf)> {
    // 虚拟 Shell 对象无法导出到桌面，返回原始路径
    if logical_path.starts_with("::{") || logical_path.starts_with("shell::virtual::") {
        return Ok((false, PathBuf::from(logical_path)));
    }
    let vault = vault_path_for(logical_path)?;
    if !vault.exists() {
        let export = export_desktop_path(logical_path);
        if export.exists() {
            return Ok((true, export));
        }
        return Ok((false, export));
    }

    let export = export_desktop_path(logical_path);
    if export.exists() {
        std::fs::remove_file(&vault)?;
        log::info!("桌面已有该图标，已从仓库删除副本: {}", logical_path);
        return Ok((true, export));
    }

    std::fs::rename(&vault, &export).map_err(|e| {
        anyhow::anyhow!(
            "从仓库导出失败 {} → {}: {}",
            vault.display(),
            export.display(),
            e
        )
    })?;
    notify_shell_path(&export.to_string_lossy());
    log::info!(
        "图标已从仓库导出到桌面: {} → {}",
        logical_path,
        export.display()
    );
    Ok((true, export))
}

/// 隐藏单个图标（拖入分类框时调用）
pub fn hide_single_icon(logical_path: &str) -> Result<()> {
    ingest_icon(logical_path)
}

/// 删除仓库副本（公共桌面图标拖回桌面时使用）
pub fn delete_vault_copy(logical_path: &str) {
    match vault_path_for(logical_path) {
        Ok(vault) => {
            if vault.exists() {
                if let Err(e) = std::fs::remove_file(&vault) {
                    log::warn!("删除仓库副本失败 {}: {}", vault.display(), e);
                } else {
                    log::info!("已删除仓库副本: {}", vault.display());
                }
            }
        }
        Err(e) => log::warn!("获取仓库路径失败: {}", e),
    }
}

/// 程序退出时将所有仓库中的图标还原到桌面。
///
/// 遍历 `managed_icons/` 目录下的所有文件，根据文件名前缀
/// (`user__` / `public__`) 还原原始桌面路径，将 .lnk 文件移回桌面，
/// 并通知 Shell 刷新。已在桌面上存在的文件不会被覆盖。
pub fn restore_all_hidden_icons() {
    let managed_dir = match get_managed_dir() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("获取图标仓库目录失败: {}", e);
            return;
        }
    };
    restore_icons_from_dir(&managed_dir);
}

/// 从指定目录还原图标（可测试的核心逻辑）。
///
/// 遍历给定目录中的所有文件，根据仓库文件名前缀还原原始桌面路径，
/// 将文件移回桌面。返回 `(restored, skipped)` 计数。
fn restore_icons_from_dir(managed_dir: &Path) -> (u32, u32) {
    let entries = match std::fs::read_dir(managed_dir) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("读取图标仓库目录失败: {}", e);
            return (0, 0);
        }
    };

    let mut restored = 0u32;
    let mut skipped = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();

        // 跳过子目录（如 user__backup/）
        if !path.is_file() {
            continue;
        }

        let vault_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // 根据仓库文件名还原原始桌面路径
        let logical_path = match logical_path_from_vault_name(&vault_name) {
            Some(p) => p,
            None => {
                log::warn!("无法解析仓库文件名: {}", vault_name);
                continue;
            }
        };

        let logical_str = logical_path.to_string_lossy().to_string();

        // 如果桌面已有该文件，仅清理仓库副本
        if logical_path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("清理仓库副本失败 {}: {}", vault_name, e);
            } else {
                log::info!("桌面已有该图标，已清理仓库副本: {}", vault_name);
                skipped += 1;
            }
            continue;
        }

        // 将文件从仓库移回桌面
        match move_file(&path, &logical_path) {
            Ok(()) => {
                notify_shell_path(&logical_str);
                log::info!(
                    "图标已从仓库还原到桌面: {} → {}",
                    vault_name,
                    logical_path.display()
                );
                restored += 1;
            }
            Err(e) => {
                log::warn!(
                    "从仓库还原图标失败 {} → {}: {}",
                    vault_name,
                    logical_path.display(),
                    e
                );
            }
        }
    }

    if restored > 0 || skipped > 0 {
        log::info!(
            "图标仓库清理完成: 还原 {} 个，跳过 {} 个（桌面已存在）",
            restored,
            skipped
        );
    }

    (restored, skipped)
}


/// 枚举仓库中的所有图标，返回 DesktopIcon 列表
///
/// 遍历 managed_icons/ 目录，根据仓库文件名前缀还原原始桌面路径，
/// 为每个仓库中的图标创建 DesktopIcon 对象。
/// 这些图标在桌面上不可见（已被移入仓库），但在分类框中应可见。
pub fn list_vault_icons() -> Vec<crate::platform::types::DesktopIcon> {
    let managed_dir = match get_managed_dir() {
        Ok(d) => d,
        Err(e) => {
            log::debug!("获取仓库目录失败（仓库可能不存在）: {}", e);
            return Vec::new();
        }
    };

    if !managed_dir.exists() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(&managed_dir) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("读取仓库目录失败: {}", e);
            return Vec::new();
        }
    };

    let mut icons = Vec::new();

    for entry in entries.flatten() {
        let vault_file_path = entry.path();

        // 跳过子目录
        if !vault_file_path.is_file() {
            continue;
        }

        let vault_name = match vault_file_path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // 还原原始桌面路径
        let logical_path = match logical_path_from_vault_name(&vault_name) {
            Some(p) => p,
            None => {
                log::debug!("无法解析仓库文件名: {}", vault_name);
                continue;
            }
        };

        let is_shortcut = vault_file_path
            .extension()
            .map(|ext| ext == "lnk" || ext == "url")
            .unwrap_or(false);

        // 显示名称（去掉扩展名）
        let display_name = if is_shortcut {
            logical_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| vault_name.clone())
        } else {
            logical_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| vault_name.clone())
        };

        // 解析快捷方式目标路径（从仓库中的 .lnk 文件）
        let target_path = if is_shortcut {
            let vault_str = vault_file_path.to_string_lossy();
            super::icon_extract::resolve_shortcut_target(&vault_str).map(PathBuf::from)
        } else {
            Some(logical_path.clone())
        };

        icons.push(crate::platform::types::DesktopIcon {
            id: uuid::Uuid::new_v4(),
            display_name,
            path: logical_path,
            target_path,
            icon_path: None,
            position: (0, 0),
            is_shortcut,
        });
    }

    if !icons.is_empty() {
        log::info!("从仓库加载 {} 个图标", icons.len());
    }

    icons
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_desktop_path_detection() {
        let desktop = dirs::home_dir()
            .expect("home dir")
            .join("Desktop")
            .join("app.lnk");
        let user = desktop.to_string_lossy();
        assert!(is_user_desktop_path(&user));
        assert!(!is_public_desktop_path(&user));
    }

    #[test]
    fn test_public_desktop_export_to_user() {
        let public = r"C:\Users\Public\Desktop\Chrome.lnk";
        assert!(is_public_desktop_path(public));
        let export = export_desktop_path(public);
        assert!(is_user_desktop_path(&export.to_string_lossy()));
        assert!(export.file_name().unwrap() == "Chrome.lnk");
    }

    #[test]
    fn test_vault_filename_user_desktop() {
        let desktop = dirs::home_dir()
            .expect("home dir")
            .join("Desktop")
            .join("RustRover 2026.1.4.lnk");
        let path = desktop.to_string_lossy();
        let vault = vault_filename(&path);
        assert_eq!(vault, Some("user__RustRover 2026.1.4.lnk".to_string()));
    }

    #[test]
    fn test_vault_filename_public_desktop() {
        let public = r"C:\Users\Public\Desktop\Chrome.lnk";
        let vault = vault_filename(public);
        assert_eq!(vault, Some("public__Chrome.lnk".to_string()));
    }

    #[test]
    fn test_vault_filename_unknown_path() {
        let vault = vault_filename(r"C:\Other\app.lnk");
        assert_eq!(vault, Some("user__app.lnk".to_string()));
    }

    #[test]
    fn test_logical_path_from_vault_name_user() {
        let desktop = dirs::home_dir().expect("home dir").join("Desktop");
        let result = logical_path_from_vault_name("user__RustRover.lnk");
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(path.starts_with(&desktop));
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "RustRover.lnk");
    }

    #[test]
    fn test_logical_path_from_vault_name_public() {
        let result = logical_path_from_vault_name("public__Chrome.lnk");
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(path.starts_with(r"C:\Users\Public\Desktop"));
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "Chrome.lnk");
    }

    #[test]
    fn test_logical_path_from_vault_name_no_prefix() {
        let desktop = dirs::home_dir().expect("home dir").join("Desktop");
        let result = logical_path_from_vault_name("unknown.lnk");
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(path.starts_with(&desktop));
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "unknown.lnk");
    }

    #[test]
    fn test_vault_roundtrip_user() {
        let desktop = dirs::home_dir()
            .expect("home dir")
            .join("Desktop")
            .join("Test.lnk");
        let original = desktop.to_string_lossy().to_string();
        let vault = vault_filename(&original).expect("vault name");
        let restored = logical_path_from_vault_name(&vault).expect("restored path");
        assert_eq!(restored.to_string_lossy(), original);
    }

    #[test]
    fn test_vault_roundtrip_public() {
        let original = r"C:\Users\Public\Desktop\App.lnk";
        let vault = vault_filename(original).expect("vault name");
        assert_eq!(vault, "public__App.lnk");
        let restored = logical_path_from_vault_name(&vault).expect("restored path");
        assert_eq!(restored.to_string_lossy(), original);
    }

    #[test]
    fn test_restore_icons_from_empty_dir() {
        let temp_dir = std::env::temp_dir().join("ikun_test_empty_vault");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        let (restored, skipped) = restore_icons_from_dir(&temp_dir);
        assert_eq!(restored, 0);
        assert_eq!(skipped, 0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_restore_icons_skips_subdirectories() {
        let temp_dir = std::env::temp_dir().join("ikun_test_skip_subdirs");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        std::fs::create_dir_all(temp_dir.join("user__backup")).expect("create subdir");

        let (restored, skipped) = restore_icons_from_dir(&temp_dir);
        assert_eq!(restored, 0);
        assert_eq!(skipped, 0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_restore_icons_cleans_vault_when_desktop_exists() {
        // Create a vault file for a file that already exists on the user's desktop
        let desktop = dirs::home_dir().expect("home dir").join("Desktop");
        let existing_file = desktop.join("ikun_test_existing.lnk");

        // Skip test if we can't create files on the desktop
        if std::fs::write(&existing_file, b"test").is_err() {
            return;
        }

        let temp_dir = std::env::temp_dir().join("ikun_test_clean_vault");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        let vault_file = temp_dir.join("user__ikun_test_existing.lnk");
        std::fs::write(&vault_file, b"vault copy").expect("write vault file");

        let (restored, skipped) = restore_icons_from_dir(&temp_dir);
        assert_eq!(restored, 0);
        assert_eq!(skipped, 1);
        assert!(!vault_file.exists(), "vault file should be cleaned up");

        // Cleanup
        let _ = std::fs::remove_file(&existing_file);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_restore_icons_handles_rename_failure_gracefully() {
        // Create a vault file for a path that doesn't exist and can't be created
        // (e.g., a path on a non-existent volume)
        let temp_dir = std::env::temp_dir().join("ikun_test_rename_fail");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        // This vault name will resolve to a user desktop path, but since the
        // file doesn't exist there, rename will be attempted. If the temp_dir
        // is on a different volume than the desktop, rename will fail.
        // The function should handle this gracefully (log warning, continue).
        let vault_file = temp_dir.join("user__nonexistent_app.lnk");
        std::fs::write(&vault_file, b"vault copy").expect("write vault file");

        // This should not panic, even if rename fails
        let (restored, _skipped) = restore_icons_from_dir(&temp_dir);
        // restored may be 0 (rename failed) or 1 (rename succeeded)
        // The important thing is no panic
        assert!(restored <= 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
        // 清理可能在桌面上创建的 nonexistent_app.lnk
        let desktop = dirs::home_dir().expect("home dir").join("Desktop");
        let _ = std::fs::remove_file(desktop.join("nonexistent_app.lnk"));
    }
}
