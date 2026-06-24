//! 隐藏/恢复桌面图标
//!
//! 通过逐个操作 SysListView32 中的图标位置来隐藏/显示，
//! **不移动整个 SysListView32 窗口**。
//!
//! # 架构决策
//!
//! **不移动 SysListView32 窗口**：将整个窗口移到屏幕外会导致
//! `show_icon_at` 设置的图标位置不可见（拖拽到桌面时图标消失）。
//! 保持 SysListView32 在原位，仅将管理的图标移到屏幕外坐标，
//! 这样拖拽释放时图标可以立即显示在桌面可见位置。
//!
//! **旧方案（已废弃）**：`SetWindowPos` 将 SysListView32 移到屏幕外。
//! 导致拖拽到桌面空白区域时图标不可见（仅程序退出后才恢复）。

use anyhow::Result;
use std::ffi::c_void;
use std::sync::Mutex;

// ===== Raw FFI =====
type HWND = *mut c_void;

extern "system" {
    fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> HWND;
    fn FindWindowExW(
        hWndParent: HWND,
        hWndChildAfter: HWND,
        lpClassName: *const u16,
        lpWindowName: *const u16,
    ) -> HWND;
    fn IsWindow(hWnd: HWND) -> i32;
}

static PROG_CLASS: &[u16] = &[80, 114, 111, 103, 109, 97, 110, 0]; // "Progman"
static WORKER_CLASS: &[u16] = &[87, 111, 114, 107, 101, 114, 87, 0]; // "WorkerW"
static DEFVIEW_CLASS: &[u16] = &[
    83, 72, 69, 76, 76, 68, 76, 76, 95, 68, 101, 102, 86, 105, 101, 119, 0,
]; // "SHELLDLL_DefView"
static LISTVIEW_CLASS: &[u16] = &[
    83, 121, 115, 76, 105, 115, 116, 86, 105, 101, 119, 51, 50, 0,
]; // "SysListView32"

/// 查找桌面 SysListView32 句柄
fn find_desktop_listview() -> Result<HWND> {
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

/// 图标隐藏器全局状态
struct IconHider {
    hidden: bool,
    /// 已隐藏的图标文件路径列表（用于恢复）
    hidden_file_paths: Vec<String>,
}

// Safety: HWND 是 Send 的（只在同一线程使用）
unsafe impl Send for IconHider {}

static ICON_HIDER: Mutex<Option<IconHider>> = Mutex::new(None);

/// 初始化图标隐藏系统（缓存 SysListView32 HWND + 预初始化图标管理器）
///
/// 不再移动 SysListView32 窗口，仅缓存句柄和初始化 DesktopIconManager，
/// 为后续 `hide_managed_icons` 做准备。
///
/// 保持 SysListView32 在原位，使拖拽到桌面时图标可立即显示。
pub fn hide_native_icons() -> Result<()> {
    let lv_hwnd = find_desktop_listview()?;

    // ★ 缓存 SysListView32 HWND（供 DesktopIconManager 使用）
    super::ffi_types::SHARED_LISTVIEW_HWND.store(
        lv_hwnd as isize,
        std::sync::atomic::Ordering::SeqCst,
    );
    log::info!("SysListView32 HWND 已缓存: {:?}（窗口保持在原位）", lv_hwnd);

    // 预先初始化 DesktopIconManager
    super::drag_handler::init_icon_manager_early();

    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;
    *guard = Some(IconHider {
        hidden: true,
        hidden_file_paths: Vec::new(),
    });

    Ok(())
}

/// 隐藏已管理的桌面图标（逐个移动到屏幕外）
///
/// 将配置中属于分类框的图标从桌面 SysListView32 中逐个移到屏幕外，
/// 保持 SysListView32 窗口在原位，使非管理图标和后续拖拽释放的图标可见。
///
/// # 参数
///
/// - `file_paths`: 分类框中管理的文件路径列表（从配置中获取）
pub fn hide_managed_icons(file_paths: &[String]) -> Result<()> {
    if file_paths.is_empty() {
        log::info!("无管理的图标需要隐藏");
        return Ok(());
    }

    let mut hider_guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;

    // 获取 DesktopIconManager（可变引用，用于 rebuild_index）
    let mut icon_mgr_guard = match super::drag_handler::get_or_init_icon_manager() {
        Some(g) => g,
        None => {
            log::warn!("DesktopIconManager 不可用，跳过逐个隐藏");
            return Ok(());
        }
    };

    let mgr = match icon_mgr_guard.as_mut() {
        Some(m) => m,
        None => {
            log::warn!("DesktopIconManager 为 None，跳过逐个隐藏");
            return Ok(());
        }
    };

    // 刷新索引
    if let Err(e) = mgr.rebuild_index() {
        log::warn!("hide_managed_icons: rebuild_index 失败: {}", e);
    }

    let mut hidden_paths = Vec::new();
    let mut hidden_count = 0;

    for file_path in file_paths {
        let file_stem = std::path::Path::new(file_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let display_name = std::path::Path::new(file_path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        // 尝试多个名称匹配（file_stem 和 display_name）
        let search_names = [file_stem.clone(), display_name.clone()];
        for name in &search_names {
            if name.is_empty() {
                continue;
            }
            if let Some(index) = mgr.find_icon_by_name(name) {
                // 保存位置并隐藏
                mgr.save_icon_position(name);
                if let Err(e) = mgr.hide_icon(index) {
                    log::warn!("隐藏图标 '{}' 失败: {}", name, e);
                } else {
                    hidden_count += 1;
                    hidden_paths.push(file_path.clone());
                    log::debug!("已隐藏图标: '{}' (index={})", name, index);
                }
                break;
            }
        }
    }

    log::info!(
        "已逐个隐藏 {} 个管理的桌面图标（共 {} 个文件路径）",
        hidden_count,
        file_paths.len()
    );

    if let Some(ref mut hider) = *hider_guard {
        hider.hidden_file_paths = hidden_paths;
    }

    Ok(())
}

/// 恢复所有已管理的桌面图标到原始位置
///
/// 将之前通过 `hide_managed_icons` 隐藏的图标移回原来的桌面坐标。
/// SysListView32 窗口始终在原位，无需移动。
pub fn restore_native_icons() -> Result<()> {
    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;

    if let Some(hider) = guard.take() {
        if !hider.hidden {
            return Ok(());
        }

        if hider.hidden_file_paths.is_empty() {
            log::info!("无隐藏的图标需要恢复");
            return Ok(());
        }

        // 获取 DesktopIconManager 恢复图标位置
        let icon_mgr_guard = match super::drag_handler::get_or_init_icon_manager() {
            Some(g) => g,
            None => {
                log::warn!("DesktopIconManager 不可用，无法恢复图标位置");
                return Ok(());
            }
        };

        let mgr = match icon_mgr_guard.as_ref() {
            Some(m) => m,
            None => {
                log::warn!("DesktopIconManager 为 None，无法恢复图标位置");
                return Ok(());
            }
        };

        // 恢复每个隐藏的图标到其原始位置
        let mut restored_count = 0;
        for file_path in &hider.hidden_file_paths {
            let file_stem = std::path::Path::new(file_path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            let display_name = std::path::Path::new(file_path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            let search_names = [file_stem.as_str(), display_name.as_str()];
            for name in &search_names {
                if name.is_empty() {
                    continue;
                }
                if let Some(index) = mgr.find_icon_by_name(name) {
                    // 使用保存的原始位置恢复
                    let (orig_x, orig_y) = mgr
                        .get_saved_position(name)
                        .unwrap_or((100, 100));
                    if let Err(e) = mgr.show_icon_at(index, orig_x, orig_y) {
                        log::warn!("恢复图标 '{}' 失败: {}", name, e);
                    } else {
                        restored_count += 1;
                        log::debug!("已恢复图标: '{}' → ({}, {})", name, orig_x, orig_y);
                    }
                    break;
                }
            }
        }

        log::info!("已恢复 {} 个桌面图标到原始位置", restored_count);
    }

    Ok(())
}
