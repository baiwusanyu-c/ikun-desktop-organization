//! Custom drag tracking coordinator.
//!
//! Manages the drag lifecycle: threshold detection, drag tracking via timer,
//! and dispatching drop operations. Delegates to sub-modules:
//! - [`drag_drop`]: drop operations (same-box, box-to-box, box-to-desktop, OLE)
//! - [`drag_visual`]: drag-over visual feedback (insert separator + replace highlight)
//!
//! Uses Timer-based polling instead of SetCapture (unstable on WS_EX_LAYERED windows).

use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::get_data;

// Re-exports for external callers (wnd_proc, overlay_wnd_proc, etc.)
pub(crate) use super::drag_drop::{
    handle_drag_enter, handle_drag_item_added, handle_drag_leave, handle_ole_drop,
};
pub(crate) use super::drag_visual::handle_cross_box_drag_over;

// =============================================================================
// Phase 1: Drag threshold detection (from WM_MOUSEMOVE)
// =============================================================================
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
    let cell_w = data.cell_w;
    let display_name = data.items[item_idx as usize].display_name.clone();
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    unsafe {
        super::drag_image::create_drag_overlay(icon_handle, icon_size, cell_w, &display_name, dpi);
        super::drag_image::update_drag_overlay();
        // 使用 Timer 轮询检测鼠标释放（SetCapture 在分层窗口上不稳定）
        SetTimer(hwnd, TIMER_ID_GRID_DRAG, 8, std::ptr::null_mut());
    }

    true
}

// =============================================================================
// Phase 2: Drag position update (from WM_MOUSEMOVE)
// =============================================================================
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
    // 使用 find_grid_window_at_point 替代 WindowFromPoint，
    // 因为分类框在 HWND_BOTTOM，WindowFromPoint 会被覆盖窗口拦截
    let target_hwnd = super::zorder::find_grid_window_at_point(pt.x, pt.y);

    // 对目标分类框显示高亮
    if !target_hwnd.is_null() && target_hwnd != hwnd {
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_ENTER, 0, 0) };
    }

    true
}

// =============================================================================
// Phase 3: Drag release (from WM_LBUTTONUP / timer)
// =============================================================================
/// 返回 true 表示拖拽已完成（调用者应 return 0）。
pub(crate) fn end_drag(hwnd: HWND) -> bool {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return false,
    };

    log::info!("end_drag called: drag_active={}", data.drag_active);

    if !data.drag_active {
        return false;
    }

    let item_idx = data.drag_item_index;
    data.drag_active = false;
    data.drag_item_index = -1;
    // 保存拖拽覆盖可视化状态（在清除前），供 handle_same_box_drop 使用
    let saved_insert_col = data.drag_over_insert_col;
    let saved_insert_row = data.drag_over_insert_row;
    // 清除拖拽覆盖可视化状态
    data.drag_over_item = -1;
    data.drag_over_insert_col = -1;
    data.drag_over_insert_row = -1;

    // 停止定时器 + 销毁覆盖窗口
    unsafe {
        KillTimer(hwnd, TIMER_ID_GRID_DRAG);
        super::drag_image::destroy_drag_overlay();
    }

    if item_idx < 0 || (item_idx as usize) >= data.items.len() {
        log::warn!("拖拽释放: 项索引无效");
        return true;
    }

    // 检测释放目标
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    // 使用 find_grid_window_at_point 替代 WindowFromPoint，
    // 因为分类框在 HWND_BOTTOM，WindowFromPoint 会被覆盖窗口拦截
    let target_hwnd = super::zorder::find_grid_window_at_point(pt.x, pt.y);

    if target_hwnd == hwnd {
        // 同框内拖拽 → grid snap + swap
        super::drag_drop::handle_same_box_drop(
            hwnd,
            item_idx as usize,
            pt.x,
            pt.y,
            saved_insert_col,
            saved_insert_row,
        );
        return true;
    }

    if !target_hwnd.is_null() {
        // === 框→框移动 ===
        super::drag_drop::handle_box_to_box_move(hwnd, target_hwnd, item_idx as usize);
    } else {
        // === 框→桌面/覆盖窗口（从框中移除项）===
        // target_hwnd 为 null 表示拖到覆盖区域或桌面
        super::drag_drop::handle_box_to_desktop(hwnd, item_idx as usize, pt.x, pt.y, true);
    }

    // 清除所有框的高亮
    unsafe { PostMessageW(hwnd, WM_USER_DRAG_LEAVE, 0, 0) };
    if !target_hwnd.is_null() {
        unsafe { PostMessageW(target_hwnd, WM_USER_DRAG_LEAVE, 0, 0) };
    }

    true
}

// =============================================================================
// Drag timer polling (replaces SetCapture on WS_EX_LAYERED)
// =============================================================================
/// 网格拖拽轮询定时器处理函数（每 16ms 调用一次）
pub(crate) fn handle_grid_drag_timer(hwnd: HWND) {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };
    if !data.drag_active {
        log::warn!("handle_grid_drag_timer: drag_active=false, 清理残影");
        unsafe {
            KillTimer(hwnd, TIMER_ID_GRID_DRAG);
            super::drag_image::destroy_drag_overlay();
        }
        return;
    }

    // 更新拖拽残影位置
    unsafe { super::drag_image::update_drag_overlay() };

    // 检测光标下的窗口，管理高亮
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    let target_hwnd = super::zorder::find_grid_window_at_point(pt.x, pt.y);

    // 拖拽可视化反馈：检测同框内和跨框的拖拽覆盖状态
    if target_hwnd == hwnd {
        super::drag_visual::update_drag_over_visual(hwnd, pt.x, pt.y);
    } else {
        // 光标不在同框内，清除本框的拖拽覆盖状态
        super::drag_visual::clear_drag_over_visual(data);
    }

    // 跨框可视化反馈：发送光标位置给目标框以更新可视化
    if !target_hwnd.is_null() && target_hwnd != hwnd {
        let lparam = ((pt.y as isize) << 16) | ((pt.x as isize) & 0xFFFF);
        unsafe {
            PostMessageW(target_hwnd, WM_USER_DRAG_OVER_UPDATE, 0, lparam as LPARAM);
        }
    }
    // 检测外部窗口（Explorer 等）：当光标不在任何分类框上时，启动 OLE 拖拽
    if target_hwnd.is_null() {
        let ext_hwnd = super::zorder::find_external_window_at_point(pt.x, pt.y);
        if !ext_hwnd.is_null() {
            start_grid_ole_drag(hwnd, ext_hwnd);
            return;
        }
    }

    // 检测鼠标左键是否仍然按下
    let lbutton_down = unsafe { GetAsyncKeyState(VK_LBUTTON) } & 0x8000u16 as i16 != 0;
    if !lbutton_down {
        log::info!("grid drag timer: 鼠标已释放，结束拖拽");
        end_drag(hwnd);
    }
}

// =============================================================================
// OLE drag to external windows (Explorer etc.)
// =============================================================================
/// 检测到光标在外部窗口（Explorer）上时，启动 OLE 拖拽。
///
/// 行为与覆盖区域拖拽到 Explorer 一致：
/// - 将文件复制到目标位置（CF_HDROP 格式）
/// - 分类框内图标保持原位（复制操作，不删除）
fn start_grid_ole_drag(hwnd: HWND, _external_hwnd: HWND) {
    log::info!("start_grid_ole_drag: 启动分类框→Explorer OLE 拖拽");
    unsafe { KillTimer(hwnd, TIMER_ID_GRID_DRAG); }

    // 获取拖拽项信息并重置拖拽状态
    let (item_idx, file_path, display_name, icon_handle) = {
        let data = match unsafe { get_data(hwnd) } {
            Some(d) => d,
            None => return,
        };

        let idx = data.drag_item_index;
        data.drag_pending = false;
        data.drag_active = false;
        data.drag_item_index = -1;
        data.drag_over_item = -1;
        data.drag_over_insert_col = -1;
        data.drag_over_insert_row = -1;

        if idx < 0 || (idx as usize) >= data.items.len() {
            unsafe { super::drag_image::destroy_drag_overlay(); }
            super::d2d_render::render_layered_window(hwnd);
            return;
        }

        let item = &data.items[idx as usize];
        (idx, item.file_path.clone(), item.display_name.clone(), item.icon_handle)
    };

    // 销毁拖拽残影
    unsafe { super::drag_image::destroy_drag_overlay(); }

    log::info!(
        "分类框→Explorer OLE: {} (path={})",
        display_name,
        file_path
    );

    // 构造 OLE 拖拽数据（CF_HDROP + IkunBoxDragData 自定义格式）
    let box_drag_data = IkunBoxDragData {
        source_hwnd: hwnd as usize,
        source_config_id: String::new(),
        items: vec![IkunDragItemData {
            index: item_idx as usize,
            file_path: file_path.clone(),
            display_name: display_name.clone(),
        }],
    };

    // 隐藏覆盖窗口和分类框，让 OLE hit-testing 能到达 Explorer
    // （覆盖窗口是全屏分层窗口，会拦截 hit-test；分类框也需隐藏以确保 OLE 正确检测）
    let overlay_hwnd = super::desktop_overlay::get_overlay_window_hwnd();
    if let Some(ohwnd) = overlay_hwnd {
        unsafe { ShowWindow(ohwnd, SW_HIDE); }
    }
    unsafe { ShowWindow(hwnd, SW_HIDE); }

    let drop_succeeded = super::ole_dnd::start_box_drag(
        vec![file_path],
        box_drag_data,
        icon_handle,
    );

    // 恢复窗口可见性
    unsafe { ShowWindow(hwnd, SW_SHOWNA); }
    if let Some(ohwnd) = overlay_hwnd {
        unsafe { ShowWindow(ohwnd, SW_SHOWNA); }
    }

    if drop_succeeded {
        log::info!("分类框→Explorer OLE 拖拽成功，图标保持原位");
    } else {
        log::info!("分类框→Explorer OLE 拖拽取消，图标保持原位");
    }
    // 图标始终保持原位（复制操作，不是移动）
    super::d2d_render::render_layered_window(hwnd);
}

// =============================================================================
// Config save helper (used by drag_drop sub-module)
// =============================================================================
/// 拖拽操作后保存配置
pub(crate) fn save_config() {
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
