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

/// 公共桌面仓库根目录
fn public_app_data_root() -> PathBuf {
    get_public_desktop().join(".ikun_organizer")
}

/// 公共桌面仓库路径
fn get_public_vault_dir() -> PathBuf {
    public_app_data_root().join(MANAGED_DIR_NAME)
}

/// 确保公共桌面仓库目录存在
fn ensure_public_vault_dir() -> Result<()> {
    let dir = get_public_vault_dir();
    if dir.exists() {
        return Ok(());
    }
    // 先尝试直接创建（可能不需要管理员权限）
    if std::fs::create_dir_all(&dir).is_ok() {
        log::info!("创建公共仓库目录: {}", dir.display());
        return Ok(());
    }
    // 权限不足，使用 ShellExecuteExW + runas 提升权限（无终端闪烁）
    log::info!(
        "[ELEVATE] 创建公共仓库目录需要管理员权限: {}",
        dir.display()
    );

    use std::ffi::c_void;

    #[repr(C)]
    struct SHELLEXECUTEINFOW {
        cb_size: u32,
        f_mask: u32,
        hwnd: *mut c_void,
        lp_verb: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show: i32,
        h_inst_app: *mut c_void,
        lp_id_list: *mut c_void,
        lp_class: *const u16,
        hkey_class: *mut c_void,
        dw_hot_key: u32,
        h_icon_or_monitor: *mut c_void,
        h_process: *mut c_void,
    }

    extern "system" {
        fn ShellExecuteExW(p_exec_info: *mut SHELLEXECUTEINFOW) -> i32;
        fn WaitForSingleObject(h_handle: *mut c_void, dw_milliseconds: u32) -> u32;
        fn CloseHandle(h_object: *mut c_void) -> i32;
    }

    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x00000040;
    const SEE_MASK_NO_CONSOLE: u32 = 0x00008000;
    const SW_HIDE: i32 = 0;
    const INFINITE: u32 = 0xFFFFFFFF;

    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = "cmd.exe\0".encode_utf16().collect();
    let params_str = format!("/c mkdir \"{}\"", dir.to_string_lossy());
    let params: Vec<u16> = params_str
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut sei = SHELLEXECUTEINFOW {
        cb_size: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        f_mask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_CONSOLE,
        hwnd: std::ptr::null_mut(),
        lp_verb: verb.as_ptr(),
        lp_file: file.as_ptr(),
        lp_parameters: params.as_ptr(),
        lp_directory: std::ptr::null(),
        n_show: SW_HIDE,
        h_inst_app: std::ptr::null_mut(),
        lp_id_list: std::ptr::null_mut(),
        lp_class: std::ptr::null(),
        hkey_class: std::ptr::null_mut(),
        dw_hot_key: 0,
        h_icon_or_monitor: std::ptr::null_mut(),
        h_process: std::ptr::null_mut(),
    };

    let ok = unsafe { ShellExecuteExW(&mut sei) };
    if ok == 0 || sei.h_process.is_null() {
        return Err(anyhow::anyhow!("用户取消了管理员授权"));
    }

    // 等待 cmd 进程结束
    unsafe {
        WaitForSingleObject(sei.h_process, INFINITE);
        CloseHandle(sei.h_process);
    }

    if dir.exists() {
        log::info!("[ELEVATE] 公共仓库目录已创建: {}", dir.display());
        Ok(())
    } else {
        Err(anyhow::anyhow!("目录创建失败"))
    }
}

const SHCNE_UPDATEITEM: u32 = 0x0000_2000;
const SHCNE_CREATE: u32 = 0x0000_0002;
const SHCNE_DELETE: u32 = 0x0000_4000;
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
    if is_public_desktop_path(logical_path) {
        return get_public_desktop().join(file_name);
    }
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

pub(crate) fn notify_shell_create(path: &str) {
    notify_shell_path(path);
}

/// 通知 Shell 文件已从指定路径删除（用于文件从桌面移入仓库时）
pub(crate) fn notify_shell_delete(path: &str) {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = Path::new(path)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        super::ffi_imports::SHChangeNotify(
            SHCNE_DELETE,
            SHCNF_PATHW | SHCNF_FLUSH,
            wide.as_ptr() as *const _,
            std::ptr::null(),
        );
    }
}

pub(crate) fn notify_shell_path(path: &str) {
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
    if logical_path.starts_with("::{") {
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
    let t0 = std::time::Instant::now();
    log::info!("[TIMING] ingest_icon 开始: logical_path={}", logical_path);
    // 虚拟 Shell 对象（回收站、此电脑等）无需收入仓库，它们始终存在
    if logical_path.starts_with("::{") {
        return Ok(());
    }
    let vault = vault_path_for(logical_path)?;
    let src = Path::new(logical_path);
    log::info!(
        "收入仓库: vault={}, src.exists={}",
        vault.display(),
        src.exists()
    );

    if vault.exists() {
        if is_user_desktop_path(logical_path) && src.exists() {
            if let Err(e) = std::fs::remove_file(src) {
                log::warn!("清理桌面残留副本失败 {}: {}", logical_path, e);
            } else {
                notify_shell_delete(logical_path);
                log::info!("已清理桌面残留副本: {}", logical_path);
            }
        }
        return Ok(());
    }

    if !src.exists() {
        log::debug!("ingest: 源文件不存在，跳过: {}", logical_path);
        return Ok(());
    }

    // 公共桌面图标：使用 IFileOperation COM 接口移动
    // Windows 会自动弹出原生权限对话框（"你需要权限来执行此操作" → 继续 → UAC 授权）
    if is_public_desktop_path(logical_path) {
        log::info!(
            "[INGEST] 公共桌面图标，使用 IFileOperation: {}",
            logical_path
        );
        ensure_public_vault_dir()?;
        let file_name = Path::new(logical_path)
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("无法获取文件名: {}", logical_path))?;
        let public_vault = get_public_vault_dir().join(file_name);
        match move_with_ifile_operation(logical_path, &public_vault) {
            Ok(()) => {
                log::info!(
                    "[INGEST] 公共桌面图标已移动: {} → {}",
                    logical_path,
                    public_vault.display()
                );
                return Ok(());
            }
            Err(e) => {
                log::warn!("[INGEST] IFileOperation 失败: {} - {}", logical_path, e);
                return Err(e);
            }
        }
    }

    let t_rename = std::time::Instant::now();
    std::fs::rename(src, &vault)
        .map_err(|e| anyhow::anyhow!("移动图标到仓库失败 {}: {}", logical_path, e))?;
    log::info!("[TIMING] rename 完成 [{:?}]", t_rename.elapsed());
    let t_notify = std::time::Instant::now();
    notify_shell_delete(logical_path);
    log::info!(
        "[TIMING] notify_shell_delete 完成 [{:?}]",
        t_notify.elapsed()
    );
    log::info!(
        "[TIMING] ingest_icon 总计 [{:?}]: {} → {}",
        t0.elapsed(),
        logical_path,
        vault.display()
    );
    Ok(())
}

/// 从仓库导出到用户桌面
pub fn export_icon(logical_path: &str) -> Result<(bool, PathBuf)> {
    let t0 = std::time::Instant::now();
    log::info!("[TIMING] export_icon 开始: {}", logical_path);
    // 虚拟 Shell 对象无法导出到桌面，返回原始路径
    if logical_path.starts_with("::{") {
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

    let t_rename = std::time::Instant::now();
    std::fs::rename(&vault, &export).map_err(|e| {
        anyhow::anyhow!(
            "从仓库导出失败 {} → {}: {}",
            vault.display(),
            export.display(),
            e
        )
    })?;
    log::info!("[TIMING] export rename 完成 [{:?}]", t_rename.elapsed());
    let t_notify = std::time::Instant::now();
    notify_shell_create(&export.to_string_lossy());
    log::info!(
        "[TIMING] export notify_shell_create 完成 [{:?}]",
        t_notify.elapsed()
    );
    log::info!(
        "[TIMING] export_icon 总计 [{:?}]: {} → {}",
        t0.elapsed(),
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

/// Permanently delete icon file (desktop + vault copy)
pub fn delete_icon_permanently(logical_path: &str) -> Result<()> {
    log::info!("delete icon permanently: {}", logical_path);
    if logical_path.starts_with("::{") {
        return Err(anyhow::anyhow!("virtual shell object cannot be deleted"));
    }
    let mut deleted_any = false;
    let desktop_path = Path::new(logical_path);
    if desktop_path.exists() {
        std::fs::remove_file(desktop_path)
            .map_err(|e| anyhow::anyhow!("delete desktop file failed {}: {}", logical_path, e))?;
        log::info!("deleted desktop file: {}", logical_path);
        deleted_any = true;
    }
    if let Ok(vault) = vault_path_for(logical_path) {
        if vault.exists() {
            std::fs::remove_file(&vault).map_err(|e| {
                anyhow::anyhow!("delete vault copy failed {}: {}", vault.display(), e)
            })?;
            log::info!("deleted vault copy: {}", vault.display());
            deleted_any = true;
        }
    }
    if deleted_any {
        let shcne_delete: u32 = 0x0000_4000;
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = Path::new(logical_path)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            super::ffi_imports::SHChangeNotify(
                shcne_delete,
                SHCNF_PATHW | SHCNF_FLUSH,
                wide.as_ptr() as *const _,
                std::ptr::null(),
            );
        }
    }
    Ok(())
}

/// Delete vault copy only (used when shell moves file to Recycle Bin)
pub fn delete_vault_copy_for(logical_path: &str) {
    match vault_path_for(logical_path) {
        Ok(vault) => {
            if vault.exists() {
                if let Err(e) = std::fs::remove_file(&vault) {
                    log::warn!("delete vault copy failed {}: {}", vault.display(), e);
                } else {
                    log::info!("deleted vault copy: {}", vault.display());
                }
            }
        }
        Err(e) => log::warn!("get vault path failed: {}", e),
    }
}

/// Rename icon file (desktop + vault copy), returns new logical path
pub fn rename_icon_file(logical_path: &str, new_name: &str) -> Result<String> {
    log::info!("rename icon: {} -> {}", logical_path, new_name);
    if logical_path.starts_with("::{") {
        return Err(anyhow::anyhow!("virtual shell object cannot be renamed"));
    }
    let old_path = Path::new(logical_path);
    let parent = old_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot get parent dir: {}", logical_path))?;
    let new_path = parent.join(new_name);
    let new_logical = new_path.to_string_lossy().to_string();
    if new_path == old_path {
        return Ok(new_logical);
    }
    // 1. Rename desktop file
    if old_path.exists() {
        std::fs::rename(old_path, &new_path).map_err(|e| {
            anyhow::anyhow!(
                "rename desktop file failed {} -> {}: {}",
                old_path.display(),
                new_path.display(),
                e
            )
        })?;
        log::info!(
            "renamed desktop file: {} -> {}",
            old_path.display(),
            new_path.display()
        );
        let shcne_delete: u32 = 0x0000_4000;
        use std::os::windows::ffi::OsStrExt;
        let old_wide: Vec<u16> = old_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            super::ffi_imports::SHChangeNotify(
                shcne_delete,
                SHCNF_PATHW | SHCNF_FLUSH,
                old_wide.as_ptr() as *const _,
                std::ptr::null(),
            );
        }
        notify_shell_path(&new_logical);
    }
    // 2. Rename vault copy
    if let Ok(old_vault) = vault_path_for(logical_path) {
        if old_vault.exists() {
            if let Ok(new_vault) = vault_path_for(&new_logical) {
                match std::fs::rename(&old_vault, &new_vault) {
                    Ok(()) => log::info!(
                        "renamed vault copy: {} -> {}",
                        old_vault.display(),
                        new_vault.display()
                    ),
                    Err(e) => log::warn!("rename vault copy failed: {}", e),
                }
            }
        }
    }
    Ok(new_logical)
}

/// Put icon file path into clipboard with cut (move) semantics
pub fn cut_icon_to_clipboard(logical_path: &str) -> Result<()> {
    log::info!("cut icon to clipboard: {}", logical_path);

    // Export vault file to desktop first (if it's in vault)
    let clipboard_path = match export_icon(logical_path) {
        Ok((_, desktop_path)) => {
            log::info!(
                "exported vault file to desktop for cut: {}",
                desktop_path.display()
            );
            desktop_path.to_string_lossy().to_string()
        }
        Err(e) => {
            log::warn!("export failed, using logical path: {}", e);
            logical_path.to_string()
        }
    };

    let path_wide: Vec<u16> = clipboard_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let dropfiles_size = std::mem::size_of::<super::ffi_types::DROPFILES>();
    let total_bytes = dropfiles_size + (path_wide.len() + 1) * 2;
    unsafe {
        if super::ffi_imports::OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(anyhow::anyhow!("cannot open clipboard"));
        }
        super::ffi_imports::EmptyClipboard();
        let hmem = super::ffi_imports::GlobalAlloc(super::ffi_types::GHND, total_bytes);
        if hmem.is_null() {
            super::ffi_imports::CloseClipboard();
            return Err(anyhow::anyhow!("GlobalAlloc failed"));
        }
        let ptr = super::ffi_imports::GlobalLock(hmem) as *mut u8;
        if ptr.is_null() {
            super::ffi_imports::GlobalFree(hmem);
            super::ffi_imports::CloseClipboard();
            return Err(anyhow::anyhow!("GlobalLock failed"));
        }
        let drop_files = super::ffi_types::DROPFILES {
            p_files: dropfiles_size as u32,
            pt_x: 0,
            pt_y: 0,
            f_nc: 0,
            f_wide: 1,
        };
        std::ptr::copy_nonoverlapping(&drop_files as *const _ as *const u8, ptr, dropfiles_size);
        let path_ptr = ptr.add(dropfiles_size) as *mut u16;
        std::ptr::copy_nonoverlapping(path_wide.as_ptr(), path_ptr, path_wide.len());
        super::ffi_imports::GlobalUnlock(hmem);
        super::ffi_imports::SetClipboardData(super::ffi_types::CF_HDROP, hmem);
        // Set Preferred DropEffect = MOVE
        let fmt_wide: Vec<u16> = "Preferred DropEffect ".encode_utf16().collect();
        let fmt_id = super::ffi_imports::RegisterClipboardFormatW(fmt_wide.as_ptr());
        if fmt_id != 0 {
            let effect_mem =
                super::ffi_imports::GlobalAlloc(super::ffi_types::GHND, std::mem::size_of::<u32>());
            if !effect_mem.is_null() {
                let effect_ptr = super::ffi_imports::GlobalLock(effect_mem) as *mut u32;
                if !effect_ptr.is_null() {
                    *effect_ptr = super::ffi_types::DROPEFFECT_MOVE;
                    super::ffi_imports::GlobalUnlock(effect_mem);
                }
                super::ffi_imports::SetClipboardData(fmt_id, effect_mem);
            }
        }
        super::ffi_imports::CloseClipboard();
    }
    log::info!("put icon path in clipboard (move): {}", clipboard_path);
    Ok(())
}

/// 程序退出时将所有仓库中的图标还原到桌面。
///
/// 遍历 `managed_icons/` 目录下的所有文件，根据文件名前缀
/// (`user__` / `public__`) 还原原始桌面路径，将 .lnk 文件移回桌面，
/// 并通知 Shell 刷新。已在桌面上存在的文件不会被覆盖。
pub fn restore_all_hidden_icons() {
    log::info!("开始还原所有隐藏的图标");
    let managed_dir = match get_managed_dir() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("获取图标仓库目录失败: {}", e);
            return;
        }
    };
    log::info!("还原图标: managed_dir={}", managed_dir.display());
    let (restored, skipped) = restore_icons_from_dir(&managed_dir);
    log::info!("还原图标完成: 还原={}, 跳过={}", restored, skipped);
}

/// 删除分类框后，将仓库中不再被任何剩余分类框管理的图标还原到桌面。
///
/// 遍历仓库文件，根据仓库文件名前缀还原原始桌面路径，
/// 然后检查该路径是否仍被剩余分类框管理（通过配置文件的 icon_names 字段）。
/// 未被管理的图标移回桌面，仍被管理的保留在仓库中。
pub fn restore_unmanaged_vault_icons() {
    let managed_dir = match get_managed_dir() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("获取图标仓库目录失败: {}", e);
            return;
        }
    };

    // 从配置中收集所有剩余分类框管理的图标路径
    let managed_paths = match crate::config::AppConfig::load() {
        Ok(config) => {
            let mut paths = std::collections::HashSet::new();
            for box_data in &config.layout.boxes {
                for path in &box_data.icon_names {
                    let normalized = path.replace('/', "\\").to_lowercase();
                    paths.insert(normalized.clone());
                    if let Some(read_path) = crate::platform::windows::resolve_icon_read_path(path)
                    {
                        paths.insert(
                            read_path
                                .to_string_lossy()
                                .replace('/', "\\")
                                .to_lowercase(),
                        );
                    }
                }
            }
            paths
        }
        Err(e) => {
            log::warn!("加载配置失败，跳过还原: {}", e);
            return;
        }
    };

    let entries = match std::fs::read_dir(&managed_dir) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("读取图标仓库目录失败: {}", e);
            return;
        }
    };

    let mut restored = 0u32;
    let mut kept = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let vault_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let logical_path = match logical_path_from_vault_name(&vault_name) {
            Some(p) => p,
            None => continue,
        };

        let logical_str = logical_path.to_string_lossy().to_string();
        let normalized = logical_str.replace('/', "\\").to_lowercase();

        // 检查是否仍被某个分类框管理
        let is_managed = managed_paths.contains(&normalized)
            || managed_paths.iter().any(|m| {
                let file_name = Path::new(m)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                !file_name.is_empty() && normalized.ends_with(file_name)
            });

        if is_managed {
            kept += 1;
            continue;
        }

        // 不再被管理，还原到桌面
        if logical_path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("清理仓库副本失败 {}: {}", vault_name, e);
            } else {
                log::info!("桌面已有该图标，已清理仓库副本: {}", vault_name);
            }
        } else {
            match move_file(&path, &logical_path) {
                Ok(()) => {
                    notify_shell_path(&logical_str);
                    log::info!(
                        "分类框删除后还原图标: {} → {}",
                        vault_name,
                        logical_path.display()
                    );
                    restored += 1;
                }
                Err(e) => {
                    log::warn!(
                        "分类框删除后还原图标失败 {} → {}: {}",
                        vault_name,
                        logical_path.display(),
                        e
                    );
                }
            }
        }
    }

    if restored > 0 || kept > 0 {
        log::info!(
            "分类框删除后仓库清理: 还原 {} 个，保留 {} 个（仍被管理）",
            restored,
            kept
        );
    }
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

/// 使用 IFileOperation COM 接口移动文件
///
/// Windows 原生文件操作接口，权限不足时自动弹出标准对话框：
/// "你需要权限来执行此操作" → 用户点"继续" → UAC 授权 → 文件移动
fn move_with_ifile_operation(src_path: &str, dst_path: &Path) -> anyhow::Result<()> {
    use std::ffi::c_void;

    extern "system" {
        fn CoCreateInstance(
            clsid: *const u8,
            p_unk: *mut c_void,
            cls_ctx: u32,
            iid: *const u8,
            ppv: *mut *mut c_void,
        ) -> i32;
        fn SHCreateItemFromParsingName(
            psz_path: *const u16,
            pbc: *mut c_void,
            riid: *const u8,
            ppv: *mut *mut c_void,
        ) -> i32;
    }

    // IFileOperation vtable - 按 Windows SDK shobjidl_core.h 正确顺序
    // 0: QueryInterface, 1: AddRef, 2: Release (IUnknown)
    // 3: Advise, 4: Unadvise, 5: SetOperationFlags
    // 6: SetProgressMessage, 7: SetProgressDialogTitle
    // 8: SetProgressDialogMessage, 9: SetProgressDialogAnimation
    // 10: SetOwnerWindow, 11: ApplyPropertiesToItem
    // 12: ApplyPropertiesToItems, 13: SetProperties
    // 14: MoveItem, 15: MoveItems
    // 16: CopyItem, 17: CopyItems
    // 18: DeleteItem, 19: DeleteItems
    // 20: RenameItem, 21: RenameItems
    // 22: NewItem, 23: NewItems
    // 24: PerformOperations, 25: GetAnyOperationsAborted
    const IDX_MOVE_ITEM: usize = 14;
    const IDX_PERFORM_OPS: usize = 24;
    const IDX_GET_ABORTED: usize = 25;

    type MoveItemFn =
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *const u16, u32) -> i32;
    type PerformOpsFn = unsafe extern "system" fn(*mut c_void) -> i32;
    type GetAbortedFn = unsafe extern "system" fn(*mut c_void, *mut i32) -> i32;
    type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

    // CLSID_FileOperation = {3ad05575-8857-4850-98f2-148fa4e14053}
    const CLSID: [u8; 16] = [
        0x75, 0x55, 0xd0, 0x3a, 0x57, 0x88, 0x50, 0x48, 0x98, 0xf2, 0x14, 0x8f, 0xa4, 0xe1, 0x40,
        0x53,
    ];
    // IID_IFileOperation = {947aab5f-0a5c-4c13-b4d6-4bf7836fc9f8}
    const IID_IOP: [u8; 16] = [
        0x5f, 0xab, 0x7a, 0x94, 0x5c, 0x0a, 0x13, 0x4c, 0xb4, 0xd6, 0x4b, 0xf7, 0x83, 0x6f, 0xc9,
        0xf8,
    ];
    // IID_IShellItem = {43826d1e-e718-42ee-bc55-a1e261c37bfe}
    const IID_SI: [u8; 16] = [
        0x1e, 0x6d, 0x82, 0x43, 0x18, 0xe7, 0xee, 0x42, 0xbc, 0x55, 0xa1, 0xe2, 0x61, 0xc3, 0x7b,
        0xfe,
    ];

    const CLSCTX: u32 = 1;
    const S_OK: i32 = 0;

    unsafe {
        let mut file_op: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            CLSID.as_ptr(),
            std::ptr::null_mut(),
            CLSCTX,
            IID_IOP.as_ptr(),
            &mut file_op,
        );
        if hr != S_OK || file_op.is_null() {
            return Err(anyhow::anyhow!(
                "CoCreateInstance(IFileOperation) failed: {:#x}",
                hr
            ));
        }

        let vtbl = *(file_op as *const *const *const c_void);
        let release_fn: ReleaseFn = std::mem::transmute(*vtbl.add(2));
        let move_item_fn: MoveItemFn = std::mem::transmute(*vtbl.add(IDX_MOVE_ITEM));
        let perform_fn: PerformOpsFn = std::mem::transmute(*vtbl.add(IDX_PERFORM_OPS));
        let get_aborted_fn: GetAbortedFn = std::mem::transmute(*vtbl.add(IDX_GET_ABORTED));

        // 创建源 IShellItem
        let src_w: Vec<u16> = src_path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut src_item: *mut c_void = std::ptr::null_mut();
        let hr = SHCreateItemFromParsingName(
            src_w.as_ptr(),
            std::ptr::null_mut(),
            IID_SI.as_ptr(),
            &mut src_item,
        );
        if hr != S_OK {
            release_fn(file_op);
            return Err(anyhow::anyhow!("创建源 IShellItem 失败: {:#x}", hr));
        }

        // 创建目标目录 IShellItem（ensure_public_vault_dir 已确保目录存在）
        let dst_dir = dst_path.parent().unwrap_or(Path::new("."));
        let dst_w: Vec<u16> = dst_dir
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut dst_item: *mut c_void = std::ptr::null_mut();
        let hr = SHCreateItemFromParsingName(
            dst_w.as_ptr(),
            std::ptr::null_mut(),
            IID_SI.as_ptr(),
            &mut dst_item,
        );
        if hr != S_OK {
            release_fn(src_item);
            release_fn(file_op);
            return Err(anyhow::anyhow!(
                "创建目标目录 IShellItem 失败: {:#x} ({})",
                hr,
                dst_dir.display()
            ));
        }

        // 目标文件名
        let new_name: Vec<u16> = dst_path
            .file_name()
            .map(|n| {
                n.to_string_lossy()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect()
            })
            .unwrap_or_default();

        // MoveItem: 将移动操作加入队列
        let hr = move_item_fn(
            file_op,
            src_item,
            dst_item,
            if new_name.is_empty() {
                std::ptr::null()
            } else {
                new_name.as_ptr()
            },
            0,
        );
        if hr != S_OK {
            release_fn(dst_item);
            release_fn(src_item);
            release_fn(file_op);
            return Err(anyhow::anyhow!("MoveItem 入队失败: {:#x}", hr));
        }

        // PerformOperations: 执行所有排队的操作
        // Windows 会在这里弹出原生权限对话框（无终端窗口）
        log::info!("[IFileOp] PerformOperations 开始");
        let hr = perform_fn(file_op);
        log::info!("[IFileOp] PerformOperations 返回: {:#x}", hr);

        let mut aborted: i32 = 0;
        get_aborted_fn(file_op, &mut aborted);

        release_fn(dst_item);
        release_fn(src_item);
        release_fn(file_op);

        if hr != S_OK {
            return Err(anyhow::anyhow!("PerformOperations 失败: {:#x}", hr));
        }
        if aborted != 0 {
            return Err(anyhow::anyhow!("用户取消了操作"));
        }

        Ok(())
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
    fn test_public_desktop_export_to_public() {
        let public = r"C:\Users\Public\Desktop\Chrome.lnk";
        assert!(is_public_desktop_path(public));
        let export = export_desktop_path(public);
        assert!(is_public_desktop_path(&export.to_string_lossy()));
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
