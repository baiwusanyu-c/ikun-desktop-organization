//! Drag-over visual feedback: insert separator + replace highlight.
//!
//! Provides HWND-aware handlers for computing and
//! rendering drag-over visual states during grid drag operations.
//! Pure logic functions have been extracted to \crate::box_core::drag_logic\.

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::{get_data, get_grid_content_rect};
pub(crate) use crate::box_core::drag_logic::{apply_shift_insert, compute_drag_over_state};

/// 清除拖拽覆盖可视化状态
pub(crate) fn clear_drag_over_visual(data: &mut super::grid_window::GridWindowData) {
    if data.drag_over_item != -1
        || data.drag_over_insert_col != -1
        || data.drag_over_insert_row != -1
    {
        data.drag_over_item = -1;
        data.drag_over_insert_col = -1;
        data.drag_over_insert_row = -1;
    }
}

/// 处理跨框拖拽覆盖可视化更新（以 WM_USER_DRAG_OVER_UPDATE 调用）
///
/// 与同框拖拽类似，但不需要排除被拖拽项（因为被拖拽项在另一个框中）。
/// 光标在单元格边缘 → 插入模式（分隔线），光标在中间 → 替换模式（高亮）。
pub(crate) fn handle_cross_box_drag_over(hwnd: HWND, screen_x: i32, screen_y: i32) {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };

    let Some(wr) = get_grid_content_rect(hwnd) else {
        return;
    };
    let client_x = screen_x - wr.left;
    let client_y = screen_y - wr.top;

    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let window_width = wr.right - wr.left;

    let view_params = crate::box_core::view_engine::get_view_params(&data.view_mode, dpi);
    let layout =
        crate::box_core::layout_calc::calculate_grid(window_width, data.title_h, &view_params, dpi);

    let virtual_y = client_y + data.scroll_offset;

    if client_x < layout.grid_left || virtual_y < layout.grid_top {
        let changed = data.drag_over_item != -1
            || data.drag_over_insert_col != -1
            || data.drag_over_insert_row != -1;
        data.drag_over_item = -1;
        data.drag_over_insert_col = -1;
        data.drag_over_insert_row = -1;
        if changed {
            render_layered_window(hwnd);
        }
        return;
    }

    let col = ((client_x - layout.grid_left) / layout.cell_w).max(0);
    let row = ((virtual_y - layout.grid_top) / layout.cell_h).max(0);

    let cell_offset_x = client_x - layout.grid_left - col * layout.cell_w;
    let ratio_x = cell_offset_x as f32 / layout.cell_w as f32;

    // 查找该单元格上是否有项（跨框拖拽不需要排除任何项）
    let item_at_cell = data
        .items
        .iter()
        .enumerate()
        .find(|(_, item)| item.grid_col == col && item.grid_row == row)
        .map(|(i, _)| i as i32);

    const INSERT_THRESHOLD: f32 = 0.30;

    let (new_over_item, new_insert_col, new_insert_row) =
        compute_drag_over_state(ratio_x, col, row, data.cols, item_at_cell, INSERT_THRESHOLD);

    let changed = data.drag_over_item != new_over_item
        || data.drag_over_insert_col != new_insert_col
        || data.drag_over_insert_row != new_insert_row;

    data.drag_over_item = new_over_item;
    data.drag_over_insert_col = new_insert_col;
    data.drag_over_insert_row = new_insert_row;

    if changed {
        render_layered_window(hwnd);
    }
}

/// 更新拖拽覆盖可视化状态（同框内拖拽时调用）
///
/// 根据光标在网格单元格中的相对位置，决定显示插入指示器还是替换高亮：
/// - 光标在单元格左/右 30% 区域 → 插入模式（显示垂直分隔线）
/// - 光标在单元格中间 40% 区域 → 替换模式（显示背景高亮）
pub(crate) fn update_drag_over_visual(hwnd: HWND, screen_x: i32, screen_y: i32) {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };

    let dragged_idx = data.drag_item_index;
    if dragged_idx < 0 {
        return;
    }

    // 计算窗口坐标
    let Some(wr) = get_grid_content_rect(hwnd) else {
        return;
    };
    let client_x = screen_x - wr.left;
    let client_y = screen_y - wr.top;

    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let window_width = wr.right - wr.left;

    let view_params = crate::box_core::view_engine::get_view_params(&data.view_mode, dpi);
    let layout =
        crate::box_core::layout_calc::calculate_grid(window_width, data.title_h, &view_params, dpi);

    let virtual_y = client_y + data.scroll_offset;

    if client_x < layout.grid_left || virtual_y < layout.grid_top {
        let changed = data.drag_over_item != -1
            || data.drag_over_insert_col != -1
            || data.drag_over_insert_row != -1;
        data.drag_over_item = -1;
        data.drag_over_insert_col = -1;
        data.drag_over_insert_row = -1;
        if changed {
            render_layered_window(hwnd);
        }
        return;
    }

    let col = ((client_x - layout.grid_left) / layout.cell_w).max(0);
    let row = ((virtual_y - layout.grid_top) / layout.cell_h).max(0);

    let cell_offset_x = client_x - layout.grid_left - col * layout.cell_w;
    let ratio_x = cell_offset_x as f32 / layout.cell_w as f32;

    // 查找该单元格上的项（排除被拖拽项自身）
    let item_at_cell = data
        .items
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != dragged_idx as usize)
        .find(|(_, item)| item.grid_col == col && item.grid_row == row)
        .map(|(i, _)| i as i32);

    const INSERT_THRESHOLD: f32 = 0.30;

    let (new_over_item, new_insert_col, new_insert_row) =
        compute_drag_over_state(ratio_x, col, row, data.cols, item_at_cell, INSERT_THRESHOLD);

    let changed = data.drag_over_item != new_over_item
        || data.drag_over_insert_col != new_insert_col
        || data.drag_over_insert_row != new_insert_row;

    data.drag_over_item = new_over_item;
    data.drag_over_insert_col = new_insert_col;
    data.drag_over_insert_row = new_insert_row;

    if changed {
        render_layered_window(hwnd);
    }
}

/// 计算 shift-drag 插入后的新位置布局（委托到 box_core::drag_logic）
pub(crate) fn compute_shift_insert_positions(
    positions: &[(i32, i32)],
    dragged_idx: usize,
    insert_lin: i32,
    cols: i32,
) -> Vec<(i32, i32)> {
    apply_shift_insert(positions, dragged_idx, insert_lin, cols)
}
