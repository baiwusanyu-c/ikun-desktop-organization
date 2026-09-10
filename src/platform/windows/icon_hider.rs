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
//! - 公共桌面 `.lnk` → **移动**到 `C:\Users\Public\.ikun_organizer\managed_icons\`
//! - 拖回桌面 → 从仓库导出到用户桌面

use anyhow::Result;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 复制文件或整个目录（目录感知复制）
///
/// `std::fs::copy` 仅支持文件，对目录会失败或只创建空目录。
/// 拖拽对象可能是文件夹（桌面/资源管理器→分类框、分类框→资源管理器等），
/// 统一使用本函数确保目录及其内容被完整复制。
pub(crate) fn copy_path_recursive(src: &Path, dst: &Path) -> std::io::Result<u64> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        let mut total = 0u64;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let target = dst.join(entry.file_name());
            if file_type.is_dir() {
                total += copy_path_recursive(&entry.path(), &target)?;
            } else if file_type.is_file() {
                total += std::fs::copy(entry.path(), &target)?;
            }
            // 符号链接等其他类型跳过
        }
        Ok(total)
    } else {
        std::fs::copy(src, dst)
    }
}

/// 删除文件或整个目录（目录感知删除）
///
/// `std::fs::remove_file` 仅支持文件，对目录会失败。
/// 清理仓库/桌面上的残留副本时统一使用本函数。
pub(crate) fn remove_path(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// 跨卷移动文件（先尝试 rename，失败后回退到 copy + delete）
///
/// `std::fs::rename` 在源和目标位于不同卷时会失败（Windows 错误码 17: ERROR_NOT_SAME_DEVICE）。
/// 此函数提供可靠回退：先尝试 rename，失败时改用 copy + remove。
/// 使用 `copy_path_recursive` / `remove_path`，同时支持文件和文件夹。
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
                copy_path_recursive(src, dst).map_err(|copy_err| {
                    anyhow::anyhow!(
                        "跨卷复制失败 {} -> {}: {}",
                        src.display(),
                        dst.display(),
                        copy_err
                    )
                })?;
                remove_path(src).map_err(|rm_err| {
                    anyhow::anyhow!("复制后删除源失败 {}: {}", src.display(), rm_err)
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

/// 移动公共桌面文件：先走普通文件操作，权限不足时交给系统 Shell/UAC。
/// 失败时源文件保持不变，调用方可据此保持 UI 与配置不变。
fn move_public_file(src: &Path, dst: &Path) -> Result<()> {
    use super::ffi_imports::SHFileOperationW;
    use super::ffi_types::{FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FO_MOVE, SHFILEOPSTRUCTW};

    // 1. 先尝试直接移动（有权限时无需 UAC）
    match std::fs::rename(src, dst) {
        Ok(()) => return Ok(()),
        Err(rename_error) => {
            log::info!(
                "[MOVE-PUBLIC] rename 失败 ({}), 尝试 SHFileOperationW",
                rename_error
            );
        }
    }

    // 2. 使用 SHFileOperationW（会弹出 UAC 授权框）
    log::info!(
        "[MOVE-PUBLIC] 使用 SHFileOperationW: {} -> {}",
        src.display(),
        dst.display()
    );

    let from_wide = make_double_null_wide(src);
    let to_wide = make_double_null_wide(dst);

    let fileop = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        w_func: FO_MOVE,
        p_from: from_wide.as_ptr(),
        p_to: to_wide.as_ptr(),
        f_flags: FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR,
        f_any_operations_aborted: 0,
        h_name_mappings: std::ptr::null_mut(),
        lpsz_progress_title: std::ptr::null(),
    };

    let result = unsafe { SHFileOperationW(&fileop) };

    if result != 0 {
        log::warn!("[MOVE-PUBLIC] SHFileOperationW 失败: {}", result);
        return Err(anyhow::anyhow!("SHFileOperationW 失败: {}", result));
    }

    if fileop.f_any_operations_aborted != 0 {
        return Err(anyhow::anyhow!("用户取消了操作"));
    }

    log::info!("[MOVE-PUBLIC] SHFileOperationW 成功");
    Ok(())
}

/// 将路径编码为双 null 结尾的 UTF-16 宽字符串（SHFileOperationW 要求的格式）
fn make_double_null_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .chain(std::iter::once(0))
        .collect()
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
    get_public_desktop()
        .parent()
        .map(|root| root.join(".ikun_organizer"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Public\.ikun_organizer"))
}

/// 公共桌面仓库路径
fn get_public_vault_dir() -> PathBuf {
    public_app_data_root().join(MANAGED_DIR_NAME)
}

/// 旧版本错误创建在 Public\Desktop 下的公共仓库
fn legacy_public_vault_dir() -> PathBuf {
    get_public_desktop()
        .join(".ikun_organizer")
        .join(MANAGED_DIR_NAME)
}

fn public_vault_path_for(logical_path: &str) -> Result<PathBuf> {
    let file_name = Path::new(logical_path)
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("无法获取公共桌面文件名: {}", logical_path))?;
    let current = get_public_vault_dir().join(file_name);
    if current.exists() {
        return Ok(current);
    }
    let legacy = legacy_public_vault_dir().join(file_name);
    if legacy.exists() {
        return Ok(legacy);
    }
    // 兼容更早版本：公共图标曾以 public__ 前缀写入用户仓库。
    let legacy_user = app_data_root()?.join(MANAGED_DIR_NAME).join(format!(
        "{}{}",
        PREFIX_PUBLIC,
        file_name.to_string_lossy()
    ));
    if legacy_user.exists() {
        return Ok(legacy_user);
    }
    Ok(current)
}

/// 确保公共桌面仓库目录存在
fn ensure_public_vault_dir() -> Result<()> {
    let dir = get_public_vault_dir();
    if dir.exists() {
        migrate_legacy_public_vault_dir(&dir)?;
        return Ok(());
    }
    // 先尝试直接创建（可能不需要管理员权限）
    if std::fs::create_dir_all(&dir).is_ok() {
        log::info!("创建公共仓库目录: {}", dir.display());
        migrate_legacy_public_vault_dir(&dir)?;
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
        migrate_legacy_public_vault_dir(&dir)?;
        Ok(())
    } else {
        Err(anyhow::anyhow!("目录创建失败"))
    }
}

/// 将旧版本错误创建在 Public\Desktop 下的文件迁移到公共根目录。
fn migrate_legacy_public_vault_dir(new_dir: &Path) -> Result<()> {
    let legacy_dir = legacy_public_vault_dir();
    if !legacy_dir.exists() || legacy_dir == new_dir {
        return Ok(());
    }

    let entries = std::fs::read_dir(&legacy_dir)
        .map_err(|e| anyhow::anyhow!("读取旧公共仓库失败 {}: {}", legacy_dir.display(), e))?;
    let mut migrated = 0;
    for entry in entries.flatten() {
        let src = entry.path();
        // 文件和文件夹都迁移（文件夹也可被拖入分类框管理）
        let Some(name) = src.file_name() else {
            continue;
        };
        let dest = new_dir.join(name);
        if dest.exists() {
            continue;
        }
        match move_file(&src, &dest) {
            Ok(()) => migrated += 1,
            Err(e) => log::warn!(
                "旧公共仓库迁移失败 {} -> {}: {}",
                src.display(),
                dest.display(),
                e
            ),
        }
    }
    if migrated > 0 {
        log::info!(
            "已迁移 {} 个旧公共仓库文件到 {}",
            migrated,
            new_dir.display()
        );
    }
    Ok(())
}

/// Shell 变更通知常量（数值必须与 Windows SDK `shlobj.h` 一致）。
///
/// 本模块底部的单元测试会把这五个值钉在 `windows` crate 的官方
/// 定义上，避免再出现手写错位的情况。历史上这里有两个致命错值：
///
/// - `SHCNF_PATHW` 曾写成 `0x8000`。`uFlags & SHCNF_TYPE(0xFF)` 因此落到
///   `SHCNF_IDLIST(0)`，Shell 会把传进去的宽字符串指针当成 PIDL 逐段解析；
///   路径里一旦出现中文（首字符码元很大，被当成 `cb` 长度）就会越界，
///   表现为启动还原分类框时在 `windows.storage.dll` / `ucrtbase.dll` 里
///   0xC0000005 崩溃，而纯 ASCII 路径反而能侥幸走完。
/// - `SHCNE_DELETE` 曾写成 `0x4000`，那是 `SHCNE_SERVERDISCONNECT`，
///   桌面图标不会真正刷新。
const SHCNE_UPDATEITEM: u32 = 0x0000_2000;
const SHCNE_CREATE: u32 = 0x0000_0002;
const SHCNE_DELETE: u32 = 0x0000_0004;
const SHCNF_PATHW: u32 = 0x0000_0005;
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
        // 文件和文件夹都迁移（文件夹也可被拖入分类框管理）
        let name = match src.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let dest = managed_dir.join(&name);
        if dest.exists() {
            let _ = remove_path(&src);
            continue;
        }
        if move_file(&src, &dest).is_ok() {
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
    let path = file_path.replace('/', "\\").to_lowercase();
    let root = get_public_desktop()
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase();
    path == root || path.starts_with(&(root + "\\"))
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
    if is_public_desktop_path(logical_path) {
        return public_vault_path_for(logical_path);
    }
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

/// 收入仓库：用户桌面和公共桌面都移入对应仓库，公共桌面仅在权限层使用 Shell 操作。
pub fn ingest_icon(logical_path: &str) -> Result<()> {
    let t0 = std::time::Instant::now();
    log::info!("[TIMING] ingest_icon 开始: logical_path={}", logical_path);
    // 虚拟 Shell 对象（回收站、此电脑等）无需收入仓库，它们始终存在
    if logical_path.starts_with("::{") {
        return Ok(());
    }
    let src = Path::new(logical_path);
    if is_public_desktop_path(logical_path) {
        // 先创建/迁移公共仓库，再解析目标，避免旧目录中的文件触发提前返回。
        ensure_public_vault_dir()?;
    }
    let vault = vault_path_for(logical_path)?;
    log::info!(
        "收入仓库: vault={}, src.exists={}",
        vault.display(),
        src.exists()
    );

    if vault.exists() {
        if (is_user_desktop_path(logical_path) || is_public_desktop_path(logical_path))
            && src.exists()
        {
            // 使用目录感知删除：文件夹（如桌面文件夹被拖入后桌面残留的目录）也能清理
            if let Err(e) = remove_path(src) {
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
        let file_name = Path::new(logical_path)
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("无法获取文件名: {}", logical_path))?;
        let public_vault = get_public_vault_dir().join(file_name);
        match move_public_file(src, &public_vault) {
            Ok(()) => {
                notify_shell_delete(logical_path);
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
    // 使用 move_file（rename + 跨卷回退），同时支持文件和文件夹
    move_file(src, &vault)
        .map_err(|e| anyhow::anyhow!("移动图标到仓库失败 {}: {}", logical_path, e))?;
    log::info!("[TIMING] move 完成 [{:?}]", t_rename.elapsed());
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
        // 使用目录感知删除：仓库中的文件夹副本也能正确清理
        remove_path(&vault)?;
        log::info!("桌面已有该图标，已从仓库删除副本: {}", logical_path);
        return Ok((true, export));
    }

    let t_rename = std::time::Instant::now();
    if is_public_desktop_path(logical_path) {
        move_public_file(&vault, &export).map_err(|e| {
            anyhow::anyhow!(
                "公共图标导出失败 {} → {}: {}",
                vault.display(),
                export.display(),
                e
            )
        })?;
    } else {
        // 使用 move_file（rename + 跨卷回退），同时支持文件和文件夹
        move_file(&vault, &export).map_err(|e| {
            anyhow::anyhow!(
                "从仓库导出失败 {} → {}: {}",
                vault.display(),
                export.display(),
                e
            )
        })?;
    }
    log::info!("[TIMING] export move 完成 [{:?}]", t_rename.elapsed());
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
                // 目录感知删除：文件夹副本也能正确删除
                if let Err(e) = remove_path(&vault) {
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
        // 目录感知删除：文件夹也能彻底删除
        remove_path(desktop_path)
            .map_err(|e| anyhow::anyhow!("delete desktop file failed {}: {}", logical_path, e))?;
        log::info!("deleted desktop file: {}", logical_path);
        deleted_any = true;
    }
    if let Ok(vault) = vault_path_for(logical_path) {
        if vault.exists() {
            remove_path(&vault).map_err(|e| {
                anyhow::anyhow!("delete vault copy failed {}: {}", vault.display(), e)
            })?;
            log::info!("deleted vault copy: {}", vault.display());
            deleted_any = true;
        }
    }
    if deleted_any {
        let shcne_delete: u32 = SHCNE_DELETE;
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
                // 目录感知删除：文件夹副本也能正确删除
                if let Err(e) = remove_path(&vault) {
                    log::warn!("delete vault copy failed {}: {}", vault.display(), e);
                } else {
                    log::info!("deleted vault copy: {}", vault.display());
                }
            }
        }
        Err(e) => log::warn!("get vault path failed: {}", e),
    }
}

// =============================================================================
// Staging: 创建原始文件名的临时副本用于 Shell 操作（避免 user_/public_ 前缀泄漏）
// =============================================================================

/// 获取 staging 目录路径
fn get_staging_dir() -> Result<std::path::PathBuf> {
    let dir = app_data_root()?.join("staging");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("创建 staging 目录失败: {}", e))?;
    }
    Ok(dir)
}

/// 从 vault 文件名还原原始文件名
fn original_filename_from_vault(logical_path: &str) -> Option<String> {
    let vault_name = std::path::Path::new(logical_path)
        .file_name()?
        .to_string_lossy()
        .to_string();
    let original = if let Some(stripped) = vault_name.strip_prefix(PREFIX_USER) {
        stripped.to_string()
    } else if let Some(stripped) = vault_name.strip_prefix(PREFIX_PUBLIC) {
        stripped.to_string()
    } else {
        vault_name
    };
    if original.is_empty() {
        None
    } else {
        Some(original)
    }
}

/// 检查路径是否是 vault 路径（managed_icons 目录下）
/// 检查路径是否是 vault 路径（managed_icons 目录下）
///
/// 用于判断 Shell 菜单是否需要创建 staging 副本。
pub fn is_vault_path(path: &str) -> bool {
    path.contains("managed_icons")
}
/// 为 vault 文件创建 staging 副本（原始文件名），用于 Shell 右键菜单操作
///
/// 返回 staging 文件路径。如果文件不在 vault 中或创建失败，返回 None。
/// 调用者负责在使用后调用 `cleanup_staging` 清理。
pub fn create_staging_for_shell(logical_path: &str) -> Option<std::path::PathBuf> {
    // 检查是否是 vault 管理的文件
    let vault_path = vault_path_for(logical_path).ok()?;
    if !vault_path.exists() {
        return None; // 不在 vault 中，无需 staging
    }

    let staging_dir = get_staging_dir().ok()?;
    let original_name = original_filename_from_vault(logical_path)?;
    let staging_path = staging_dir.join(&original_name);

    // 清理旧的 staging 文件（如果有）——目录感知删除
    if staging_path.exists() {
        let _ = remove_path(&staging_path);
    }

    // 复制 vault 文件到 staging（使用原始文件名）——目录感知复制
    match copy_path_recursive(&vault_path, &staging_path) {
        Ok(_) => {
            log::info!(
                "[STAGING] 创建 staging 副本: {} -> {}",
                vault_path.display(),
                staging_path.display()
            );
            Some(staging_path)
        }
        Err(e) => {
            log::warn!("[STAGING] 创建 staging 副本失败: {}", e);
            None
        }
    }
}

/// 清理 staging 文件
pub fn cleanup_staging(staging_path: &std::path::Path) {
    if staging_path.exists() {
        // 目录感知删除：文件夹的 staging 副本也能清理
        match remove_path(staging_path) {
            Ok(_) => log::debug!("[STAGING] 已清理: {}", staging_path.display()),
            Err(e) => log::warn!("[STAGING] 清理失败 {}: {}", staging_path.display(), e),
        }
    }
}

/// 检查 staging 文件是否存在
pub fn staging_exists(staging_path: &std::path::Path) -> bool {
    staging_path.exists()
}

fn rename_public_icon_file(
    logical_path: &str,
    new_path: &Path,
    new_logical: &str,
) -> Result<String> {
    let old_path = Path::new(logical_path);
    let old_vault = vault_path_for(logical_path)?;
    let new_vault = vault_path_for(new_logical)?;
    let mut desktop_moved = false;

    if old_path.exists() {
        move_public_file(old_path, new_path).map_err(|e| {
            anyhow::anyhow!(
                "公共桌面重命名失败 {} -> {}: {}",
                old_path.display(),
                new_path.display(),
                e
            )
        })?;
        desktop_moved = true;
    }

    if old_vault.exists() {
        if let Err(e) = move_public_file(&old_vault, &new_vault) {
            if desktop_moved {
                let _ = move_public_file(new_path, old_path);
            }
            return Err(anyhow::anyhow!(
                "公共仓库重命名失败 {} -> {}: {}",
                old_vault.display(),
                new_vault.display(),
                e
            ));
        }
    }

    if desktop_moved {
        use std::os::windows::ffi::OsStrExt;
        let old_wide: Vec<u16> = old_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            super::ffi_imports::SHChangeNotify(
                SHCNE_DELETE,
                SHCNF_PATHW | SHCNF_FLUSH,
                old_wide.as_ptr() as *const _,
                std::ptr::null(),
            );
        }
        notify_shell_path(new_logical);
    }
    Ok(new_logical.to_string())
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
    if is_public_desktop_path(logical_path) {
        return rename_public_icon_file(logical_path, &new_path, &new_logical);
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
        let shcne_delete: u32 = SHCNE_DELETE;
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

/// 从仓库文件名解析出桌面图标显示名称（纯逻辑函数，可独立测试）
///
/// 去掉 user__/public__ 前缀和 .lnk 扩展名，返回小写名称。
/// 如果解析结果为空则返回 None。
fn parse_vault_display_name(vault_name: &str) -> Option<String> {
    let file_name = vault_name
        .strip_prefix(PREFIX_USER)
        .or_else(|| vault_name.strip_prefix(PREFIX_PUBLIC))
        .unwrap_or(vault_name);

    let display_name = file_name
        .strip_suffix(".lnk")
        .or_else(|| file_name.strip_suffix(".LNK"))
        .unwrap_or(file_name);

    if display_name.is_empty() {
        None
    } else {
        Some(display_name.to_lowercase())
    }
}

/// 获取仓库中所有图标的显示名称集合（用于退出时只恢复仓库图标的位置）
///
/// 扫描用户仓库（managed_icons）和公共仓库目录，从仓库文件名推导出
/// 图标在 SysListView32 中的显示名称：去掉 user__/public__ 前缀
/// 和 .lnk 扩展名。返回的集合使用小写，用于大小写不敏感匹配。
pub fn get_vault_display_names() -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();

    let mut vault_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(managed_dir) = get_managed_dir() {
        if managed_dir.exists() {
            vault_dirs.push(managed_dir);
        }
    }
    let public_dir = get_public_vault_dir();
    if public_dir.exists() {
        vault_dirs.push(public_dir);
    }
    let legacy_dir = legacy_public_vault_dir();
    if legacy_dir.exists() {
        vault_dirs.push(legacy_dir);
    }

    for dir in &vault_dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("读取仓库目录失败 {}: {}", dir.display(), e);
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            // 文件和文件夹都是合法的仓库条目（文件夹也可被拖入分类框管理）
            let vault_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            if let Some(display_name) = parse_vault_display_name(&vault_name) {
                names.insert(display_name);
            }
        }
    }

    log::info!("仓库图标显示名称: {} 个", names.len());
    names
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
    let public_dir = if get_public_vault_dir().exists() {
        get_public_vault_dir()
    } else {
        legacy_public_vault_dir()
    };
    if public_dir.exists() {
        let (public_restored, public_skipped) = restore_public_icons_from_dir(&public_dir);
        log::info!(
            "public vault restored: restored={}, skipped={}",
            public_restored,
            public_skipped
        );
    }
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
        // 文件和文件夹都是合法的仓库条目（文件夹也可被拖入分类框管理），
        // 分类框删除后未再被管理的条目（含文件夹）需要还原到桌面

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
            if let Err(e) = remove_path(&path) {
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

    restore_unmanaged_public_vault_icons(&managed_paths);
}

fn restore_unmanaged_public_vault_icons(managed_paths: &std::collections::HashSet<String>) {
    let public_dir = get_public_vault_dir();
    let entries = match std::fs::read_dir(&public_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let vault_path = entry.path();
        // 文件和文件夹都是合法的仓库条目（文件夹也可被拖入分类框管理）
        let Some(name) = vault_path.file_name() else {
            continue;
        };
        let logical_path = get_public_desktop().join(name);
        let logical = logical_path
            .to_string_lossy()
            .replace('/', "\\")
            .to_lowercase();
        let is_managed = managed_paths.contains(&logical)
            || managed_paths.iter().any(|path| {
                let file_name = Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                !file_name.is_empty() && logical.ends_with(file_name)
            });
        if is_managed {
            continue;
        }

        if logical_path.exists() {
            let _ = remove_path(&vault_path);
            continue;
        }
        if let Err(e) = export_icon(&logical_path.to_string_lossy()) {
            log::warn!(
                "公共图标未再被分类框管理，恢复失败 {}: {}",
                logical_path.display(),
                e
            );
        }
    }
}

/// 从指定目录还原图标（可测试的核心逻辑）。
///
/// 遍历给定目录中的所有文件，根据仓库文件名前缀还原原始桌面路径，
/// 将文件移回桌面。返回 `(restored, skipped)` 计数。
fn restore_public_icons_from_dir(public_dir: &Path) -> (u32, u32) {
    let entries = match std::fs::read_dir(public_dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("读取公共图标仓库失败 {}: {}", public_dir.display(), e);
            return (0, 0);
        }
    };
    let mut restored = 0;
    let mut skipped = 0;
    for entry in entries.flatten() {
        let vault_path = entry.path();
        // 文件和文件夹都是合法的仓库条目（文件夹也可被拖入分类框管理）
        let Some(name) = vault_path.file_name() else {
            continue;
        };
        let desktop_path = get_public_desktop().join(name);
        if desktop_path.exists() {
            if remove_path(&vault_path).is_ok() {
                skipped += 1;
            }
            continue;
        }
        match move_public_file(&vault_path, &desktop_path) {
            Ok(()) => {
                notify_shell_path(&desktop_path.to_string_lossy());
                restored += 1;
            }
            Err(e) => log::warn!(
                "公共图标还原失败 {} -> {}: {}",
                vault_path.display(),
                desktop_path.display(),
                e
            ),
        }
    }
    (restored, skipped)
}

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

        // 文件和文件夹都是合法的仓库条目（文件夹也可被拖入分类框管理），
        // 程序退出时均需还原到桌面
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
            if let Err(e) = remove_path(&path) {
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

    let mut icons = Vec::new();
    let mut vault_dirs = vec![(managed_dir, false)];
    let public_dir = get_public_vault_dir();
    if public_dir.exists() {
        vault_dirs.push((public_dir, true));
    }

    for (managed_dir, is_public) in vault_dirs {
        let entries = match std::fs::read_dir(&managed_dir) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("读取仓库目录失败 {}: {}", managed_dir.display(), e);
                continue;
            }
        };

        for entry in entries.flatten() {
            let vault_file_path = entry.path();

            // 文件和文件夹都是合法的仓库条目（文件夹也可被拖入分类框管理）
            let vault_name = match vault_file_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            // 还原原始桌面路径
            let logical_path = match if is_public {
                Some(get_public_desktop().join(&vault_name))
            } else {
                logical_path_from_vault_name(&vault_name)
            } {
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
#[cfg(test)]
mod tests {
    use super::*;
    /// Shell 通知常量必须与 `windows` crate（即 Windows SDK）一致。
    ///
    /// 这不是形式主义：`SHCNF_PATHW` 写错会让 Shell 把宽字符串指针
    /// 当 PIDL 解析，含中文的桌面路径直接把进程打成 0xC0000005。
    #[test]
    fn shell_notify_constants_match_windows_sdk() {
        use windows::Win32::UI::Shell::{
            SHCNE_CREATE as SDK_CREATE, SHCNE_DELETE as SDK_DELETE,
            SHCNE_UPDATEITEM as SDK_UPDATEITEM, SHCNF_FLUSH as SDK_FLUSH, SHCNF_PATHW as SDK_PATHW,
            SHCNF_TYPE,
        };

        assert_eq!(SHCNE_CREATE, SDK_CREATE.0, "SHCNE_CREATE 必须与 SDK 一致");
        assert_eq!(SHCNE_DELETE, SDK_DELETE.0, "SHCNE_DELETE 必须与 SDK 一致");
        assert_eq!(
            SHCNE_UPDATEITEM, SDK_UPDATEITEM.0,
            "SHCNE_UPDATEITEM 必须与 SDK 一致"
        );
        assert_eq!(SHCNF_PATHW, SDK_PATHW.0, "SHCNF_PATHW 必须与 SDK 一致");
        assert_eq!(SHCNF_FLUSH, SDK_FLUSH.0, "SHCNF_FLUSH 必须与 SDK 一致");
        // 组合后的类型位必须落在 PATHW 上，否则 Shell 会按 PIDL 解析参数
        assert_eq!(
            (SHCNF_PATHW | SHCNF_FLUSH) & SHCNF_TYPE.0,
            SDK_PATHW.0,
            "uFlags 的类型位必须是 SHCNF_PATHW，不能退化成 SHCNF_IDLIST"
        );
    }

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
    fn test_public_desktop_path_is_case_insensitive() {
        assert!(is_public_desktop_path(
            r"C:\USERS\PUBLIC\DESKTOP\Chrome.lnk"
        ));
    }

    #[test]
    fn test_public_vault_root_is_outside_desktop() {
        assert_eq!(
            public_app_data_root(),
            PathBuf::from(r"C:\Users\Public\.ikun_organizer")
        );
        assert_eq!(
            get_public_vault_dir(),
            PathBuf::from(r"C:\Users\Public\.ikun_organizer\managed_icons")
        );
    }

    #[test]
    fn test_public_vault_path_does_not_use_user_vault() {
        let path = vault_path_for(r"C:\Users\Public\Desktop\Chrome.lnk").unwrap();
        assert_eq!(
            path,
            PathBuf::from(r"C:\Users\Public\.ikun_organizer\managed_icons\Chrome.lnk")
        );
    }

    #[test]
    fn test_user_vault_path_remains_in_user_home() {
        let logical = dirs::home_dir().unwrap().join("Desktop").join("Chrome.lnk");
        let path = vault_path_for(&logical.to_string_lossy()).unwrap();
        assert_eq!(
            path,
            dirs::home_dir()
                .unwrap()
                .join(".ikun_organizer")
                .join("managed_icons")
                .join("user__Chrome.lnk")
        );
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
    fn test_restore_icons_restores_subdirectories() {
        // 文件夹是合法的仓库条目（文件夹也可被拖入分类框管理），
        // 还原时需要移回桌面而不是被跳过
        let folder_name = "ikun_test_restore_folder_0831";
        let temp_dir = std::env::temp_dir().join("ikun_test_restore_subdirs");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        std::fs::create_dir_all(temp_dir.join(format!("user__{}", folder_name)))
            .expect("create subdir");

        let (restored, skipped) = restore_icons_from_dir(&temp_dir);
        assert_eq!(restored, 1, "仓库中的文件夹应被还原到桌面");
        assert_eq!(skipped, 0);

        // 清理：删除还原到桌面的测试文件夹
        let desktop = dirs::home_dir().expect("home dir").join("Desktop");
        let restored_dir = desktop.join(folder_name);
        if restored_dir.is_dir() {
            let _ = std::fs::remove_dir_all(&restored_dir);
        }
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

    // ===== parse_vault_display_name ???? =====

    #[test]
    fn test_parse_vault_user_prefix_lnk() {
        let result = parse_vault_display_name("user__Chrome.lnk");
        assert_eq!(result, Some("chrome".to_string()));
    }

    #[test]
    fn test_parse_vault_public_prefix_lnk() {
        let result = parse_vault_display_name("public__Firefox.lnk");
        assert_eq!(result, Some("firefox".to_string()));
    }

    #[test]
    fn test_parse_vault_no_prefix() {
        let result = parse_vault_display_name("Notepad.lnk");
        assert_eq!(result, Some("notepad".to_string()));
    }

    #[test]
    fn test_parse_vault_no_lnk_extension() {
        let result = parse_vault_display_name("user__MyFile.txt");
        assert_eq!(result, Some("myfile.txt".to_string()));
    }

    #[test]
    fn test_parse_vault_uppercase_lnk() {
        let result = parse_vault_display_name("user__App.LNK");
        assert_eq!(result, Some("app".to_string()));
    }

    #[test]
    fn test_parse_vault_empty_after_strip() {
        // "user__" 前缀后没有名称，返回 None。
        let result = parse_vault_display_name("user__");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_vault_empty_string() {
        let result = parse_vault_display_name("");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_vault_chinese_name() {
        let result = parse_vault_display_name("user__此电脑.lnk");
        assert_eq!(result, Some("此电脑".to_string()));
    }

    #[test]
    fn test_parse_vault_case_insensitive_result() {
        let r1 = parse_vault_display_name("user__CHROME.lnk");
        let r2 = parse_vault_display_name("user__chrome.lnk");
        assert_eq!(r1, r2);
        assert_eq!(r1, Some("chrome".to_string()));
    }

    // ===== make_double_null_wide 单元测试 =====

    #[test]
    fn test_make_double_null_wide_ascii_path() {
        let path = std::path::Path::new(r"C:\Users\Public\Desktop\test.lnk");
        let wide = make_double_null_wide(path);
        // 最后两个元素必须是 0（双 null 结尾）
        assert!(wide.len() >= 2);
        assert_eq!(wide[wide.len() - 1], 0);
        assert_eq!(wide[wide.len() - 2], 0);
        // 倒数第三个元素不是 0（最后一个有效字符）
        assert_ne!(wide[wide.len() - 3], 0);
    }

    #[test]
    fn test_make_double_null_wide_chinese_path() {
        let path = std::path::Path::new(r"C:\Users\Public\Desktop\微信.lnk");
        let wide = make_double_null_wide(path);
        // 双 null 结尾
        assert!(wide.len() >= 2);
        assert_eq!(wide[wide.len() - 1], 0);
        assert_eq!(wide[wide.len() - 2], 0);
        // 包含中文字符
        let s = String::from_utf16_lossy(&wide[..wide.len() - 2]);
        assert!(s.contains("微信"));
    }

    #[test]
    fn test_make_double_null_wide_empty_path() {
        let path = std::path::Path::new("");
        let wide = make_double_null_wide(path);
        // 空路径应只有两个 null
        assert_eq!(wide, vec![0, 0]);
    }

    #[test]
    fn test_make_double_null_wide_length() {
        // "C:\test.lnk" = 10 字符 + 2 null = 12
        let path = std::path::Path::new(r"C:\test.lnk");
        let wide = make_double_null_wide(path);
        assert_eq!(wide.len(), 13);
    }

    // ===== FFI 常量单元测试 =====

    #[test]
    fn test_shfileop_constants() {
        use super::super::ffi_types::{
            FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FO_COPY, FO_DELETE, FO_MOVE,
        };
        assert_eq!(FO_MOVE, 0x0001);
        assert_eq!(FO_COPY, 0x0002);
        assert_eq!(FO_DELETE, 0x0003);
        assert_eq!(FOF_NOCONFIRMATION, 0x0010);
        assert_eq!(FOF_NOCONFIRMMKDIR, 0x0200);
    }

    // ===== Staging 机制单元测试 =====

    #[test]
    fn test_is_vault_path_with_managed_icons() {
        assert!(is_vault_path(
            r"C:\Users\test\.ikun_organizer\managed_icons\user__Chrome.lnk"
        ));
        assert!(is_vault_path(
            r"C:\Users\Public\.ikun_organizer\managed_icons\public__Edge.lnk"
        ));
    }

    #[test]
    fn test_is_vault_path_non_vault() {
        assert!(!is_vault_path(r"C:\Users\test\Desktop\Chrome.lnk"));
        assert!(!is_vault_path(""));
    }

    #[test]
    fn test_original_filename_from_vault_user_prefix() {
        let result = original_filename_from_vault(
            r"C:\Users\test\.ikun_organizer\managed_icons\user__Chrome.lnk",
        );
        assert_eq!(result, Some("Chrome.lnk".to_string()));
    }

    #[test]
    fn test_original_filename_from_vault_public_prefix() {
        let result = original_filename_from_vault(
            r"C:\Users\Public\.ikun_organizer\managed_icons\public__Edge.lnk",
        );
        assert_eq!(result, Some("Edge.lnk".to_string()));
    }

    #[test]
    fn test_original_filename_from_vault_no_prefix() {
        let result = original_filename_from_vault(
            r"C:\Users\test\.ikun_organizer\managed_icons\SomeFile.lnk",
        );
        assert_eq!(result, Some("SomeFile.lnk".to_string()));
    }

    #[test]
    fn test_original_filename_from_vault_empty() {
        assert_eq!(original_filename_from_vault(""), None);
    }

    #[test]
    fn test_cleanup_staging_nonexistent() {
        cleanup_staging(std::path::Path::new(r"C:\nonexistent\staging\test.lnk"));
    }

    #[test]
    fn test_cleanup_staging_real_file() {
        let dir = std::env::temp_dir().join("ikun_staging_test_cleanup");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("test.lnk");
        std::fs::write(&file, "test").unwrap();
        assert!(file.exists());
        cleanup_staging(&file);
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_staging_exists() {
        let dir = std::env::temp_dir().join("ikun_staging_test_exists");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("test.lnk");
        std::fs::write(&file, "test").unwrap();
        assert!(staging_exists(&file));
        let _ = std::fs::remove_file(&file);
        assert!(!staging_exists(&file));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // =========================================================================
    // copy_path_recursive / remove_path（目录感知复制/删除）
    // =========================================================================

    #[test]
    fn test_copy_path_recursive_file() {
        let dir = std::env::temp_dir().join("ikun_copy_rec_file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.txt");
        let dst = dir.join("b.txt");
        std::fs::write(&src, "hello").unwrap();
        copy_path_recursive(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "hello");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_copy_path_recursive_directory() {
        let dir = std::env::temp_dir().join("ikun_copy_rec_dir");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src_folder");
        let dst = dir.join("dst_folder");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::write(src.join("root.txt"), "root").unwrap();
        std::fs::write(src.join("nested").join("deep.txt"), "deep").unwrap();

        copy_path_recursive(&src, &dst).unwrap();

        assert!(dst.is_dir());
        assert_eq!(
            std::fs::read_to_string(dst.join("root.txt")).unwrap(),
            "root"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("nested").join("deep.txt")).unwrap(),
            "deep"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_path_file_and_directory() {
        let dir = std::env::temp_dir().join("ikun_remove_path_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 删除普通文件
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        remove_path(&file).unwrap();
        assert!(!file.exists());

        // 删除非空目录
        let sub = dir.join("subdir");
        std::fs::create_dir_all(sub.join("inner")).unwrap();
        std::fs::write(sub.join("inner").join("f.txt"), "y").unwrap();
        remove_path(&sub).unwrap();
        assert!(!sub.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
