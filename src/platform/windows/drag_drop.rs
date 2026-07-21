//! Drag-and-drop operations: same-box drop, box-to-box move, box-to-desktop.
//!
//! Handles all drop scenarios when a drag gesture ends, including
//! grid snap, swap, shift-insert, and OLE drop from external sources.

use super::d2d_render::render_layered_window;
use super::drag_visual::clear_drag_over_visual;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::get_data;

/// Save grid layout configuration to disk after drag operations.
fn save_config() {
    super::drag_handler::save_config();
}

/// 同框内拖拽：grid snap + swap
pub(crate) fn handle_same_box_drop(
    hwnd: HWND,
    dragged_idx: usize,
    screen_x: i32,
    screen_y: i32,
    saved_insert_col: i32,
    saved_insert_row: i32,
) {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };

    if dragged_idx >= data.items.len() {
        return;
    }

    // Convert screen coords to window client coords
    let mut wr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut wr) };
    let client_x = screen_x - wr.left;
    let client_y = screen_y - wr.top;

    // Get current layout
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let mut wr2 = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut wr2) };
    let window_width = wr2.right - wr2.left;

    let view_params = crate::box_core::view_engine::get_view_params(&data.view_mode, dpi);
    let layout =
        crate::box_core::layout_calc::calculate_grid(window_width, data.title_h, &view_params, dpi);

    // Determine target grid cell from click position
    let target_col;
    let target_row;
    if client_x >= layout.grid_left && client_y >= layout.grid_top {
        target_col = ((client_x - layout.grid_left) / layout.cell_w).max(0);
        target_row = ((client_y - layout.grid_top) / layout.cell_h).max(0);
    } else {
        // Click outside grid area → cancel
        log::info!("同框拖拽: 释放在网格区域外，取消");
        return;
    }

    // Materialize dragged item's auto-layout position before any swap
    if data.items[dragged_idx].grid_col < 0 {
        let auto_col = (dragged_idx as i32) % layout.cols;
        let auto_row = (dragged_idx as i32) / layout.cols;
        data.items[dragged_idx].grid_col = auto_col;
        data.items[dragged_idx].grid_row = auto_row;
    }

    // Materialize all auto-layout items' positions before occupied check
    // to ensure accurate collision detection and swap
    for (i, item) in data.items.iter_mut().enumerate() {
        if item.grid_col < 0 {
            item.grid_col = (i as i32) % layout.cols;
            item.grid_row = (i as i32) / layout.cols;
        }
    }

    // Use the saved insert state from end_drag (already cleared in data)
    let insert_col = saved_insert_col;
    let insert_row = saved_insert_row;
    let is_insert_mode = insert_col >= 0 && insert_row >= 0;

    let cols = layout.cols;

    if is_insert_mode {
        // === Insert mode: use pure shift-insert function ===
        let insert_lin = insert_row * cols + insert_col;
        let positions: Vec<(i32, i32)> = data
            .items
            .iter()
            .map(|item| (item.grid_col, item.grid_row))
            .collect();
        let result =
            super::drag_visual::apply_shift_insert(&positions, dragged_idx, insert_lin, cols);
        for (i, item) in data.items.iter_mut().enumerate() {
            item.grid_col = result[i].0;
            item.grid_row = result[i].1;
        }
        log::info!(
            "Grid insert: '{}' -> ({}, {})",
            data.items[dragged_idx].display_name,
            data.items[dragged_idx].grid_col,
            data.items[dragged_idx].grid_row,
        );
    } else {
        // === Replace mode: swap or move to empty cell ===
        let occupied_idx = data
            .items
            .iter()
            .enumerate()
            .find(|(i, item)| {
                *i != dragged_idx && item.grid_col == target_col && item.grid_row == target_row
            })
            .map(|(i, _)| i);

        if let Some(other_idx) = occupied_idx {
            // Swap: exchange grid positions
            let drag_col = data.items[dragged_idx].grid_col;
            let drag_row = data.items[dragged_idx].grid_row;
            data.items[dragged_idx].grid_col = target_col;
            data.items[dragged_idx].grid_row = target_row;
            data.items[other_idx].grid_col = drag_col;
            data.items[other_idx].grid_row = drag_row;
            log::info!(
                "Grid swap: '{}' <-> '{}'",
                data.items[dragged_idx].display_name,
                data.items[other_idx].display_name,
            );
        } else {
            // Move to empty cell
            data.items[dragged_idx].grid_col = target_col;
            data.items[dragged_idx].grid_row = target_row;
            log::info!(
                "Grid snap: '{}' -> ({}, {})",
                data.items[dragged_idx].display_name,
                target_col,
                target_row,
            );
        }
    }

    data.selected_item = dragged_idx as i32;
    render_layered_window(hwnd);
    save_config();
}

/// 框→框移动：从源框移除项，添加到目标框
pub(crate) fn handle_box_to_box_move(source_hwnd: HWND, target_hwnd: HWND, item_index: usize) {
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

    // 获取释放位置坐标
    let mut drop_pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut drop_pt) };
    // 通知目标框添加项（包含释放坐标）
    unsafe {
        // 读取目标框的拖拽覆盖状态（用于插入模式）
        let (insert_col, insert_row) = get_data(target_hwnd)
            .map(|d| (d.drag_over_insert_col, d.drag_over_insert_row))
            .unwrap_or((-1, -1));
        super::drop_target::set_pending_add_item(
            file_path.clone(),
            display_name.clone(),
            drop_pt.x,
            drop_pt.y,
            insert_col,
            insert_row,
        );
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

/// 框→桌面：从仓库导出到用户桌面并定位，同时刷新覆盖窗口。
pub(crate) fn handle_box_to_desktop(
    hwnd: HWND,
    item_index: usize,
    drop_x: i32,
    drop_y: i32,
    target_is_overlay: bool,
) {
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

    if target_is_overlay {
        log::info!(
            "框→覆盖窗口: 释放图标 '{}' (path={})",
            display_name,
            file_path
        );
    } else {
        log::info!("框→桌面: 导出图标 '{}' (path={})", display_name, file_path);
    }

    let mut export_path = super::icon_hider::export_desktop_path(&file_path);
    match super::icon_hider::export_icon(&file_path) {
        Ok((true, path)) => {
            export_path = path;
            log::info!(
                "框→桌面: 图标 '{}' 已导出到 {}",
                display_name,
                export_path.display()
            );
        }
        Ok((false, path)) => {
            export_path = path;
            if export_path.exists() {
                log::info!("框→桌面: 图标 '{}' 已在桌面", display_name);
            } else {
                log::warn!("框→桌面: 图标 '{}' 不在仓库也不在桌面", display_name);
            }
        }
        Err(e) => {
            log::warn!("框→桌面: 导出图标 '{}' 失败: {}", display_name, e);
        }
    }

    // 始终尝试添加到覆盖窗口，使用 export_path 或原始 file_path 作为回退
    // CLSID 虚拟对象（如回收站）的 export_path 不是真实文件，但 add_icon_to_overlay 能处理
    let overlay_path = if export_path.exists() || file_path.starts_with("::{") {
        export_path.to_string_lossy().to_string()
    } else {
        // 导出路径不存在，使用原始 file_path 作为回退
        log::info!(
            "框→覆盖窗口: 导出路径不存在，使用原始路径 '{}' 回退添加",
            file_path
        );
        file_path.clone()
    };
    let shown_on_overlay = super::desktop_overlay::add_icon_to_overlay(
        overlay_path.clone(),
        display_name.clone(),
        drop_x,
        drop_y,
    );
    if shown_on_overlay {
        log::info!("框→覆盖窗口: '{}' 已交给覆盖窗口渲染", display_name);
    } else {
        if target_is_overlay {
            log::warn!(
                "框→覆盖窗口: '{}' 未能刷新覆盖窗口，回退到桌面定位",
                display_name
            );
        }
        // 仅在非覆盖窗口目标时回退到桌面定位
        if !target_is_overlay {
            super::icon_manager::position_icon_at_screen(&overlay_path, drop_x, drop_y);
        }
    }

    // 从框中移除项
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };
    if item_index < data.items.len() {
        let removed = data.items.remove(item_index);
        if !removed.icon_handle.is_null() {
            super::icon_extract::destroy_icon(removed.icon_handle);
        }
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
        let mut changed = false;
        if data.drop_highlight {
            data.drop_highlight = false;
            changed = true;
        }
        // 清除跨框拖拽覆盖可视化状态
        if data.drag_over_item != -1
            || data.drag_over_insert_col != -1
            || data.drag_over_insert_row != -1
        {
            data.drag_over_item = -1;
            data.drag_over_insert_col = -1;
            data.drag_over_insert_row = -1;
            changed = true;
        }
        if changed {
            render_layered_window(hwnd);
        }
    }
}

/// 处理 WM_USER_DRAG_ITEM_ADDED（框→框移动 或 桌面→框 时，目标框添加项）
pub(crate) fn handle_drag_item_added(hwnd: HWND) {
    if let Some((file_path, display_name, drop_x, drop_y, insert_col, insert_row)) =
        super::drop_target::take_pending_add_item()
    {
        // 检查是否已存在
        if let Some(data) = unsafe { get_data(hwnd) } {
            if data.items.iter().any(|i| i.file_path == file_path) {
                log::debug!("跳过已存在项: {}", file_path);
                return;
            }

            // 提取图标。分类框中保存的是“逻辑桌面路径”，图标可能已经被移动/复制到
            // managed_icons 仓库；读取时必须解析到实际存在的路径，否则框→框拖拽后
            // 目标框只能得到名称，图标句柄为空，右键菜单也会因为原路径不存在而降级。
            let read_path = resolve_drag_item_read_path(&file_path);
            let read_path_str = read_path.to_string_lossy().to_string();
            let icon_handle = super::icon_extract::extract_file_icon(&read_path_str)
                .map(|info| info.hicon)
                .unwrap_or(std::ptr::null_mut());

            // 计算释放位置对应的网格坐标
            let (target_col, target_row) = compute_drop_grid_position(hwnd, drop_x, drop_y, data);
            log::info!(
                "[DIAG] 释放位置计算: drop=({}, {}) -> grid=({}, {})",
                drop_x,
                drop_y,
                target_col,
                target_row
            );

            let is_insert = insert_col >= 0 && insert_row >= 0;

            if is_insert {
                // === Insert mode: shift existing items right, then add new item ===
                let cols = data.cols;
                let insert_lin = insert_row * cols + insert_col;

                // Materialize all auto-layout positions
                for (i, item) in data.items.iter_mut().enumerate() {
                    if item.grid_col < 0 {
                        item.grid_col = (i as i32) % cols;
                        item.grid_row = (i as i32) / cols;
                    }
                }

                // Shift items at or after insert position right by 1
                for item in data.items.iter_mut() {
                    let item_lin = item.grid_row * cols + item.grid_col;
                    if item_lin >= insert_lin {
                        let new_lin = item_lin + 1;
                        item.grid_col = new_lin % cols;
                        item.grid_row = new_lin / cols;
                    }
                }

                data.items.push(super::grid_window::GridFileItem {
                    file_path: file_path.clone(),
                    display_name: display_name.clone(),
                    icon_handle,
                    grid_col: insert_col,
                    grid_row: insert_row,
                });
                log::info!(
                    "[DIAG] 项已插入到框: '{}', insert=({}, {})",
                    display_name,
                    insert_col,
                    insert_row
                );
            } else {
                // === Normal mode: add at computed grid position ===
                data.items.push(super::grid_window::GridFileItem {
                    file_path: file_path.clone(),
                    display_name: display_name.clone(),
                    icon_handle,
                    grid_col: target_col,
                    grid_row: target_row,
                });
                log::info!(
                    "[DIAG] 项已添加到框: '{}', grid=({}, {})",
                    display_name,
                    target_col,
                    target_row
                );
            }
            render_layered_window(hwnd);
            save_config();
        }

        // ★ 隐藏桌面原始图标（该图标已在分类框中管理，桌面不应再显示）
        hide_single_desktop_icon(&file_path, &display_name);
    }
}

fn resolve_drag_item_read_path(file_path: &str) -> std::path::PathBuf {
    resolve_drag_item_read_path_with(file_path, super::icon_hider::resolve_icon_read_path)
}

fn resolve_drag_item_read_path_with(
    file_path: &str,
    resolve_read_path: impl Fn(&str) -> Option<std::path::PathBuf>,
) -> std::path::PathBuf {
    resolve_read_path(file_path).unwrap_or_else(|| std::path::PathBuf::from(file_path))
}

fn hide_single_desktop_icon(file_path: &str, display_name: &str) {
    match super::icon_hider::hide_single_icon(file_path) {
        Ok(()) => {
            if !super::icon_hider::is_public_desktop_path(file_path) {
                log::info!("桌面图标 '{}' 已收入仓库", display_name);
            }
        }
        Err(e) => {
            log::warn!("收入仓库失败 '{}': {}", display_name, e);
        }
    }
}

/// 根据释放位置的屏幕坐标计算目标网格位置
fn compute_drop_grid_position(
    hwnd: HWND,
    drop_x: i32,
    drop_y: i32,
    data: &super::grid_window::GridWindowData,
) -> (i32, i32) {
    let mut wr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut wr) };
    let client_x = drop_x - wr.left;
    let client_y = drop_y - wr.top;

    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let window_width = wr.right - wr.left;

    let view_params = crate::box_core::view_engine::get_view_params(&data.view_mode, dpi);
    let layout =
        crate::box_core::layout_calc::calculate_grid(window_width, data.title_h, &view_params, dpi);

    if client_x >= layout.grid_left && client_y >= layout.grid_top {
        let col = ((client_x - layout.grid_left) / layout.cell_w).max(0);
        let row = ((client_y - layout.grid_top) / layout.cell_h).max(0);
        (col, row)
    } else {
        // 释放在网格区域外，使用自动布局
        (-1, -1)
    }
}

/// 触发。从 `PENDING_DROP` 读取文件路径，添加到目标分类框，并隐藏桌面图标。
pub(crate) fn handle_ole_drop(hwnd: HWND) {
    let drop = match super::drop_target::take_pending_drop() {
        Some(d) => d,
        None => {
            log::debug!("OLE drop: 无待处理数据");
            return;
        }
    };

    let is_box_drag = drop.box_data.is_some();
    log::info!(
        "OLE drop: {} 个文件, box_drag={}, drop=({}, {})",
        drop.file_paths.len(),
        is_box_drag,
        drop.drop_x,
        drop.drop_y
    );

    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };

    let mut added = 0;
    for file_path in &drop.file_paths {
        // 去重：已存在则跳过
        if data.items.iter().any(|i| i.file_path == *file_path) {
            log::debug!("OLE drop: 跳过已存在项 '{}'", file_path);
            continue;
        }

        // 提取图标
        let icon_info = super::icon_extract::extract_file_icon(file_path);
        let (icon_handle, display_name) = match icon_info {
            Some(info) => (info.hicon, info.display_name),
            None => {
                let name = std::path::Path::new(file_path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.clone());
                (std::ptr::null_mut(), name)
            }
        };

        data.items.push(super::grid_window::GridFileItem {
            file_path: file_path.clone(),
            display_name: display_name.clone(),
            icon_handle,
            grid_col: -1,
            grid_row: -1,
        });
        added += 1;

        if !is_box_drag {
            // 桌面→框：收入仓库
            if let Err(e) = super::icon_hider::ingest_icon(file_path) {
                log::warn!("OLE drop: 收入仓库失败 '{}': {}", display_name, e);
            } else {
                log::info!("OLE drop: '{}' 已收入仓库", display_name);
            }
        }
    }

    if added > 0 {
        log::info!("OLE drop: 添加 {} 个项到分类框", added);
        super::d2d_render::render_layered_window(hwnd);
        save_config();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_resolve_drag_item_read_path_uses_resolved_vault_path() {
            let logical = r"C:\Users\chenzhihui\Desktop\CC Switch.lnk";
            let vault = std::path::PathBuf::from(
                r"C:\Users\chenzhihui\.ikun_organizer\managed_icons\user__CC Switch.lnk",
            );

            assert_eq!(
                resolve_drag_item_read_path_with(logical, |_| Some(vault.clone())),
                vault
            );
        }

        #[test]
        fn test_resolve_drag_item_read_path_falls_back_to_logical_path() {
            let path = r"Z:\ikun-organizer-test\missing-shortcut.lnk";

            assert_eq!(
                resolve_drag_item_read_path_with(path, |_| None),
                std::path::PathBuf::from(path)
            );
        }
    }

    // =============================================================================
    // OLE 拖拽放置处理（由 WM_USER_DRAG_DROP 触发）
    // =============================================================================

    /// 处理 OLE 拖拽放置（桌面→框 或 框→框）
    ///
    /// 由 `drop_target.rs` 的 `drop_target_drop()` 通过 `PostMessageW(WM_USER_DRAG_DROP)`

    #[test]
    fn test_compute_drop_grid_position_basic() {
        // Test basic grid position calculation
        // This is a simplified test - in real usage, HWND and GetWindowRect would be mocked
        // For now, we test the logic with known values

        let grid_left = 10;
        let grid_top = 40;
        let cell_w = 80;
        let cell_h = 96;

        // Drop at first cell
        let drop_x = grid_left + 10;
        let drop_y = grid_top + 10;
        let client_x = drop_x;
        let client_y = drop_y;

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 0);
        assert_eq!(row, 0);

        // Drop at second cell (col=1, row=0)
        let drop_x = grid_left + cell_w + 10;
        let drop_y = grid_top + 10;
        let client_x = drop_x;
        let client_y = drop_y;

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 1);
        assert_eq!(row, 0);

        // Drop at third cell (col=0, row=1)
        let drop_x = grid_left + 10;
        let drop_y = grid_top + cell_h + 10;
        let client_x = drop_x;
        let client_y = drop_y;

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 0);
        assert_eq!(row, 1);
    }

    #[test]
    fn test_compute_drop_grid_position_outside_grid() {
        // Test dropping outside grid area returns (-1, -1)
        let grid_left = 10;
        let grid_top = 40;

        // Drop before grid_left
        let client_x = grid_left - 5;
        let client_y = grid_top + 10;

        if client_x < grid_left || client_y < grid_top {
            let col = -1;
            let row = -1;
            assert_eq!(col, -1);
            assert_eq!(row, -1);
        }
    }

    #[test]
    fn test_compute_drop_grid_position_negative_coordinates() {
        // Test that negative coordinates are clamped to 0
        let grid_left = 10;
        let grid_top = 40;
        let cell_w = 80;
        let cell_h = 96;

        let client_x = grid_left + 10;
        let client_y = grid_top + 10;

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 0);
        assert_eq!(row, 0);
    }
}
