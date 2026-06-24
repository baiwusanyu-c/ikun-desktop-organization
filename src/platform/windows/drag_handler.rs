//! 自定义拖拽跟踪处理
//!
//! 替代 OLE DoDragDrop，使用 SetCapture + WM_MOUSEMOVE 实现拖拽：
//! - 创建 WS_EX_LAYERED 覆盖窗口显示拖拽图像（半透明图标跟随光标）
//! - 释放鼠标时检测目标窗口，执行移动/移除操作
//!
//! 不依赖 Shell CF_HDROP，避免域安全策略触发 UAC。

use std::sync::Mutex;

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::get_data;
use super::icon_manager::DesktopIconManager;

// =============================================================================
// 全局桌面图标管理器（懒加载，首次拖拽时初始化）
// =============================================================================

/// 桌面图标管理器单例
///
/// 首次拖拽到桌面时懒加载初始化。
/// 初始化失败不影响拖拽功能（仅降级为不恢复桌面图标位置）。
static DRAG_ICON_MANAGER: Mutex<Option<DesktopIconManager>> = Mutex::new(None);

/// 获取或初始化桌面图标管理器
///
/// 首次调用时创建 DesktopIconManager，失败时返回 None 并记录警告。
fn get_or_init_icon_manager() -> Option<std::sync::MutexGuard<'static, Option<DesktopIconManager>>> {
    let mut guard = DRAG_ICON_MANAGER.lock().ok()?;
    if guard.is_none() {
        match DesktopIconManager::new() {
            Ok(mgr) => {
                log::info!("拖拽用桌面图标管理器已初始化");
                *guard = Some(mgr);
            }
            Err(e) => {
                log::warn!("桌面图标管理器初始化失败（拖拽到桌面将不恢复图标位置）: {}", e);
                return None;
            }
        }
    }
    Some(guard)
}

/// 预先初始化桌面图标管理器（在 SysListView32 隐藏前调用）
///
/// `hide_native_icons()` 会使 SysListView32 无法被 `FindWindowExW` 找到，
/// 因此必须在隐藏前初始化 DesktopIconManager 并缓存 HWND。
pub fn init_icon_manager_early() {
    let mut guard = match DRAG_ICON_MANAGER.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if guard.is_some() {
        return; // 已初始化
    }
    match DesktopIconManager::new() {
        Ok(mgr) => {
            log::info!("桌面图标管理器已预先初始化（SysListView32 隐藏前）");
            *guard = Some(mgr);
        }
        Err(e) => {
            log::warn!("桌面图标管理器预先初始化失败: {}", e);
        }
    }
}

// =============================================================================
// Phase 1: 拖拽阈值检测（从 WM_MOUSEMOVE 调用）
// =============================================================================

/// 在 WM_MOUSEMOVE 中检测拖拽阈值
///
/// 当 `drag_pending` 为 true 且鼠标移动超过 `DRAG_THRESHOLD` 时，
/// 进入自定义拖拽模式：创建覆盖窗口 + SetCapture。
///
/// 返回 true 表示拖拽已激活（调用者应 return 0）。
pub(crate) fn check_drag_threshold(hwnd: HWND, mx: i32, my: i32) -> bool {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return false,
    };

    if !data.drag_pending {
        return false;
    }

    let dx = (mx - data.drag_start_x).abs();
    let dy = (my - data.drag_start_y).abs();
    if dx <= DRAG_THRESHOLD && dy <= DRAG_THRESHOLD {
        return false;
    }

    // 超过阈值 → 激活自定义拖拽
    data.drag_pending = false;
    let item_idx = data.selected_item;
    if item_idx < 0 || (item_idx as usize) >= data.items.len() {
        return false;
    }

    let icon_handle = data.items[item_idx as usize].icon_handle;
    data.drag_active = true;
    data.drag_item_index = item_idx;

    log::info!("自定义拖拽激活: item={}", item_idx);

    // 创建拖拽覆盖窗口（半透明图标 + 名称标签跟随光标，DPI 自适应尺寸）
    let icon_size = data.icon_size;
    let display_name = data.items[item_idx as usize].display_name.clone();
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    unsafe {
        super::drag_image::create_drag_overlay(icon_handle, icon_size, &display_name, dpi);
        super::drag_image::update_drag_overlay();
        SetCapture(hwnd);
    }

    true
}

// =============================================================================
// Phase 2: 拖拽中更新覆盖窗口位置（从 WM_MOUSEMOVE 调用）
// =============================================================================

/// 在 WM_MOUSEMOVE 中更新拖拽覆盖窗口位置
///
/// 当 `drag_active` 为 true 时，将覆盖窗口移动到光标位置。
/// 同时检测光标下的窗口，对目标分类框显示/取消高亮。
///
/// 返回 true 表示正在拖拽中（调用者应 return 0）。
pub(crate) fn update_drag(hwnd: HWND) -> bool {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return false,
    };

    if !data.drag_active {
        return false;
    }

    // 更新覆盖窗口位置
    unsafe { super::drag_image::update_drag_overlay() };

    // 检测光标下的窗口，管理高亮
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    let target_hwnd = unsafe { WindowFromPoint(pt) };

    // 对目标分类框显示高亮
    if !target_hwnd.is_null() && target_hwnd != hwnd {
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_ENTER, 0, 0) };
    }

    true
}

// =============================================================================
// Phase 3: 拖拽释放（从 WM_LBUTTONUP 调用）
// =============================================================================

/// 在 WM_LBUTTONUP 中完成拖拽操作
///
/// 检测释放目标：
/// - 另一个分类框 → 移动项（PostMessage 通知目标框添加 + 源框移除）
/// - 桌面/其他窗口 → 从源框移除项（.lnk 文件保留在磁盘）
/// - 同一个分类框 → 取消（无操作）
///
/// 返回 true 表示拖拽已完成（调用者应 return 0）。
pub(crate) fn end_drag(hwnd: HWND) -> bool {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return false,
    };

    if !data.drag_active {
        return false;
    }

    let item_idx = data.drag_item_index;
    data.drag_active = false;
    data.drag_item_index = -1;

    // 释放鼠标捕获 + 销毁覆盖窗口
    unsafe {
        ReleaseCapture();
        super::drag_image::destroy_drag_overlay();
    }

    if item_idx < 0 || (item_idx as usize) >= data.items.len() {
        log::warn!("拖拽释放: 项索引无效");
        return true;
    }

    // 检测释放目标
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    let target_hwnd = unsafe { WindowFromPoint(pt) };

    if target_hwnd == hwnd {
        // 拖回同一个框 → 取消
        log::info!("拖拽取消: 拖回同一个分类框");
        return true;
    }

    // 检查目标是否是另一个分类框
    let target_is_grid = if !target_hwnd.is_null() {
        let mut class_name = [0u16; 64];
        let len = unsafe {
            GetClassNameW(target_hwnd, class_name.as_mut_ptr(), class_name.len() as i32)
        };
        if len > 0 {
            let name = String::from_utf16_lossy(&class_name[..len as usize]);
            name == "DesktopOrgGrid"
        } else {
            false
        }
    } else {
        false
    };

    if target_is_grid {
        // === 框→框移动 ===
        handle_box_to_box_move(hwnd, target_hwnd, item_idx as usize);
    } else {
        // === 框→桌面（从框中移除项）===
        handle_box_to_desktop(hwnd, item_idx as usize);
    }

    // 清除所有框的高亮
    unsafe { PostMessageW(hwnd, WM_USER_DRAG_LEAVE, 0, 0) };
    if !target_hwnd.is_null() {
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_LEAVE, 0, 0) };
    }

    true
}

/// 框→框移动：从源框移除项，添加到目标框
fn handle_box_to_box_move(source_hwnd: HWND, target_hwnd: HWND, item_index: usize) {
    let data = match unsafe { get_data(source_hwnd) } {
        Some(d) => d,
        None => return,
    };

    if item_index >= data.items.len() {
        return;
    }

    let item = &data.items[item_index];
    let file_path = item.file_path.clone();
    let display_name = item.display_name.clone();

    // 通知目标框添加项（使用 WM_USER_DRAG_ITEM_ADDED）
    // 通过全局暂存传递数据
    unsafe {
        super::drop_target::set_pending_add_item(file_path.clone(), display_name.clone());
        PostMessageW(target_hwnd, WM_USER_DRAG_ITEM_ADDED, 0, 0);
    }

    // 从源框移除项
    let data = match unsafe { get_data(source_hwnd) } {
        Some(d) => d,
        None => return,
    };
    if item_index < data.items.len() {
        let removed = data.items.remove(item_index);
        log::info!("框→框移动: '{}' 已从源框移除", removed.display_name);
        if data.selected_item >= data.items.len() as i32 {
            data.selected_item = -1;
        }
        render_layered_window(source_hwnd);
        save_config();
    }
}

/// 框→桌面：从框中移除项，并将图标恢复到桌面释放位置
///
/// 使用 DesktopIconManager 将 SysListView32 中的图标从屏幕外
/// 移动到鼠标释放位置，使图标重新显示在桌面上。
fn handle_box_to_desktop(hwnd: HWND, item_index: usize) {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };

    if item_index >= data.items.len() {
        return;
    }

    let removed = &data.items[item_index];
    let display_name = removed.display_name.clone();
    let file_path = removed.file_path.clone();

    // 获取鼠标释放位置（屏幕坐标）
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };

    log::info!(
        "框→桌面: 开始恢复图标 '{}' (path={}) 到位置 ({}, {})",
        display_name, file_path, pt.x, pt.y
    );

    // 尝试恢复桌面图标到释放位置
    let mut restored = false;
    if let Some(mut guard) = get_or_init_icon_manager() {
        if let Some(ref mut mgr) = *guard {
            // 每次拖拽到桌面时刷新索引（explorer 可能重启、图标可能变化）
            if let Err(e) = mgr.rebuild_index() {
                log::warn!("框→桌面: rebuild_index 失败: {}", e);
            }

            log::info!(
                "框→桌面: 桌面图标管理器索引已刷新，共 {} 个图标",
                mgr.get_icon_count()
            );

            // 搜索名称列表：display_name + file_stem（覆盖不同命名方式）
            let file_stem = std::path::Path::new(&file_path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            let search_names = vec![display_name.clone(), file_stem.clone()];

            for name in &search_names {
                if name.is_empty() {
                    continue;
                }
                log::debug!("框→桌面: 搜索桌面图标名称 '{}'", name);
                if let Some(index) = mgr.find_icon_by_name(name) {
                    log::info!("框→桌面: 找到图标 '{}' → index={}", name, index);
                    match mgr.show_icon_at(index, pt.x, pt.y) {
                        Ok(()) => {
                            log::info!(
                                "框→桌面: 图标 '{}' 已恢复到桌面位置 ({}, {})",
                                name, pt.x, pt.y
                            );
                            restored = true;
                            break;
                        }
                        Err(e) => {
                            log::warn!("框→桌面: show_icon_at 失败 (index={}): {}", index, e);
                        }
                    }
                } else {
                    log::debug!("框→桌面: 未找到图标 '{}'", name);
                }
            }

            if !restored {
                // 输出索引中的所有名称用于调试
                let cached = mgr.cached_names();
                log::warn!(
                    "框→桌面: 无法匹配桌面图标 (display_name='{}', file_stem='{}'), \
                     索引中共 {} 个图标: {:?}",
                    display_name, file_stem,
                    cached.len(),
                    if cached.len() <= 20 { cached.clone() } else { cached[..20].to_vec() }
                );
            }
        }
    }

    if !restored {
        log::info!(
            "框→桌面: 降级处理 — 仅从框中移除 '{}'（.lnk 文件保留在 {}）",
            display_name, file_path
        );
    }

    // 从框中移除项
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };
    if item_index < data.items.len() {
        data.items.remove(item_index);
        if data.selected_item >= data.items.len() as i32 {
            data.selected_item = -1;
        }
        render_layered_window(hwnd);
        save_config();
    }
}

// =============================================================================
// 高亮和拖入处理（从 WM_USER_DRAG_* 消息调用）
// =============================================================================

/// 处理 WM_USER_DRAG_ENTER（拖拽悬停进入，显示高亮）
pub(crate) fn handle_drag_enter(hwnd: HWND) {
    if let Some(data) = unsafe { get_data(hwnd) } {
        if !data.locked && !data.drop_highlight {
            data.drop_highlight = true;
            render_layered_window(hwnd);
            log::debug!("拖拽悬停进入: 高亮");
        }
    }
}

/// 处理 WM_USER_DRAG_LEAVE（拖拽悬停离开，清除高亮）
pub(crate) fn handle_drag_leave(hwnd: HWND) {
    if let Some(data) = unsafe { get_data(hwnd) } {
        if data.drop_highlight {
            data.drop_highlight = false;
            render_layered_window(hwnd);
            log::debug!("拖拽悬停离开: 取消高亮");
        }
    }
}

/// 处理 WM_USER_DRAG_ITEM_ADDED（框→框移动时，目标框添加项）
pub(crate) fn handle_drag_item_added(hwnd: HWND) {
    if let Some((file_path, display_name)) = unsafe { super::drop_target::take_pending_add_item() }
    {
        // 检查是否已存在
        if let Some(data) = unsafe { get_data(hwnd) } {
            if data.items.iter().any(|i| i.file_path == file_path) {
                log::debug!("跳过已存在项: {}", file_path);
                return;
            }

            // 提取图标
            let icon_info = unsafe { super::icon_extract::extract_file_icon(&file_path) };
            let (icon_handle, actual_name) = match icon_info {
                Some(info) => (info.hicon, info.display_name),
                None => {
                    let name = std::path::Path::new(&file_path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| file_path.clone());
                    (std::ptr::null_mut(), name)
                }
            };

            data.items.push(super::grid_window::GridFileItem {
                file_path: file_path.clone(),
                display_name: actual_name.clone(),
                icon_handle,
            });
            log::info!("框→框移动: '{}' 已添加到目标框", actual_name);
            render_layered_window(hwnd);
            save_config();
        }
    }
}

/// 拖拽操作后保存配置
fn save_config() {
    let layouts = super::grid_window::collect_grid_layout();
    if let Ok(mut config) = crate::config::AppConfig::load() {
        config.layout.boxes = layouts;
        if let Err(e) = config.save() {
            log::warn!("拖拽后保存配置失败: {}", e);
        } else {
            log::info!("拖拽后配置已保存");
        }
    }
}
