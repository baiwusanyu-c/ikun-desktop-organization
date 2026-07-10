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

/// 启动时初始化仓库目录（迁移旧数据）
pub fn restore_all_hidden_icons() {
    if let Err(e) = get_managed_dir() {
        log::warn!("初始化图标仓库失败: {}", e);
    }
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
}
