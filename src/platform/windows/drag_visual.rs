//! Drag-over visual feedback: insert separator + replace highlight.
//!
//! Provides pure functions and HWND-aware handlers for computing and
//! rendering drag-over visual states during grid drag operations.

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::{get_data, get_grid_content_rect};

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

/// 处理跨框拖拽覆盖可视化更新（从 WM_USER_DRAG_OVER_UPDATE 调用）
///
/// 与同框拖拽类似，但不需要排除被拖拽项（因为被拖拽项在另一个框中）。
/// 光标在单元格边缘 → 插入模式（分隔线），光标在中心 → 替换模式（高亮）。
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

    let (new_over_item, new_insert_col, new_insert_row) = if ratio_x < INSERT_THRESHOLD {
        (-1, col, row)
    } else if ratio_x > 1.0 - INSERT_THRESHOLD {
        if col + 1 < data.cols {
            (-1, col + 1, row)
        } else {
            (item_at_cell.unwrap_or(-1), -1, -1)
        }
    } else {
        (item_at_cell.unwrap_or(-1), -1, -1)
    };

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

    // 考虑滚动偏移
    let virtual_y = client_y + data.scroll_offset;

    // 光标是否在网格区域内？
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

    // 计算光标所在的网格单元格
    let col = ((client_x - layout.grid_left) / layout.cell_w).max(0);
    let row = ((virtual_y - layout.grid_top) / layout.cell_h).max(0);

    // 计算光标在单元格内的相对位置（0.0 ~ 1.0）
    let cell_offset_x = client_x - layout.grid_left - col * layout.cell_w;
    let ratio_x = cell_offset_x as f32 / layout.cell_w as f32;

    // 查找该单元格上是否有项
    let item_at_cell = data
        .items
        .iter()
        .enumerate()
        .find(|(i, item)| {
            *i != dragged_idx as usize && item.grid_col == col && item.grid_row == row
        })
        .map(|(i, _)| i as i32);

    // 插入区域阈值：单元格左/右 30% 为插入区域
    const INSERT_THRESHOLD: f32 = 0.30;

    let (new_over_item, new_insert_col, new_insert_row) = if ratio_x < INSERT_THRESHOLD {
        // 左侧插入：分隔线显示在单元格左边缘
        (-1, col, row)
    } else if ratio_x > 1.0 - INSERT_THRESHOLD {
        // 右侧插入：分隔线显示在单元格右边缘（下一列左边缘）
        // 但要确保不超出列范围
        if col + 1 < data.cols {
            (-1, col + 1, row)
        } else {
            // 最后一列右侧：显示替换高亮
            (item_at_cell.unwrap_or(-1), -1, -1)
        }
    } else {
        // 中心区域：替换模式
        (item_at_cell.unwrap_or(-1), -1, -1)
    };

    // 检查状态是否变化，仅在变化时重绘
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

/// - Replace mode: `(item_index, -1, -1)` - show highlight background
fn compute_drag_over_state(
    ratio_x: f32,
    col: i32,
    row: i32,
    max_cols: i32,
    item_at_cell: Option<i32>,
    insert_threshold: f32,
) -> (i32, i32, i32) {
    if ratio_x < insert_threshold {
        (-1, col, row)
    } else if ratio_x > 1.0 - insert_threshold {
        if col + 1 < max_cols {
            (-1, col + 1, row)
        } else {
            (item_at_cell.unwrap_or(-1), -1, -1)
        }
    } else {
        (item_at_cell.unwrap_or(-1), -1, -1)
    }
}

/// Apply shift-insert repositioning on grid item positions.
///
/// Moves the dragged item to the target linear position and shifts
/// other items in the affected range by 1.
pub(crate) fn apply_shift_insert(
    positions: &[(i32, i32)],
    dragged_idx: usize,
    insert_lin: i32,
    cols: i32,
) -> Vec<(i32, i32)> {
    let drag_lin = positions[dragged_idx].1 * cols + positions[dragged_idx].0;
    let mut result: Vec<(i32, i32)> = positions.to_vec();

    if drag_lin == insert_lin {
        return result;
    }

    if drag_lin < insert_lin {
        for (i, pos) in result.iter_mut().enumerate() {
            if i == dragged_idx {
                continue;
            }
            let item_lin = pos.1 * cols + pos.0;
            if item_lin > drag_lin && item_lin < insert_lin {
                let new_lin = item_lin - 1;
                *pos = (new_lin % cols, new_lin / cols);
            }
        }
        let target_lin = insert_lin - 1;
        result[dragged_idx] = (target_lin % cols, target_lin / cols);
    } else {
        for (i, pos) in result.iter_mut().enumerate() {
            if i == dragged_idx {
                continue;
            }
            let item_lin = pos.1 * cols + pos.0;
            if item_lin >= insert_lin && item_lin < drag_lin {
                let new_lin = item_lin + 1;
                *pos = (new_lin % cols, new_lin / cols);
            }
        }
        result[dragged_idx] = (insert_lin % cols, insert_lin / cols);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drag_over_left_zone_insert() {
        let (oi, ic, ir) = compute_drag_over_state(0.1, 2, 0, 4, Some(5), 0.30);
        assert_eq!(oi, -1);
        assert_eq!(ic, 2);
        assert_eq!(ir, 0);
    }

    #[test]
    fn test_drag_over_right_zone_insert() {
        let (oi, ic, ir) = compute_drag_over_state(0.85, 1, 0, 4, Some(3), 0.30);
        assert_eq!(oi, -1);
        assert_eq!(ic, 2); // col+1
        assert_eq!(ir, 0);
    }

    #[test]
    fn test_drag_over_right_zone_last_col_replace() {
        let (oi, ic, ir) = compute_drag_over_state(0.85, 3, 0, 4, Some(7), 0.30);
        assert_eq!(oi, 7);
        assert_eq!(ic, -1);
        assert_eq!(ir, -1);
    }

    #[test]
    fn test_drag_over_center_replace_with_item() {
        let (oi, ic, ir) = compute_drag_over_state(0.5, 1, 0, 4, Some(3), 0.30);
        assert_eq!(oi, 3);
        assert_eq!(ic, -1);
        assert_eq!(ir, -1);
    }

    #[test]
    fn test_drag_over_center_empty_cell() {
        let (oi, ic, ir) = compute_drag_over_state(0.5, 2, 1, 4, None, 0.30);
        assert_eq!(oi, -1);
        assert_eq!(ic, -1);
        assert_eq!(ir, -1);
    }

    #[test]
    fn test_drag_over_exact_threshold_boundary() {
        let (oi, _, _) = compute_drag_over_state(0.30, 1, 0, 4, Some(3), 0.30);
        assert_eq!(oi, 3, "exact 0.30 -> center (not < 0.30)");
        let (oi, _, _) = compute_drag_over_state(0.70, 1, 0, 4, Some(3), 0.30);
        assert_eq!(oi, 3, "exact 0.70 -> center (not > 0.70)");
    }

    #[test]
    fn test_drag_over_just_below_left_threshold() {
        let (oi, ic, _) = compute_drag_over_state(0.299, 1, 0, 4, Some(3), 0.30);
        assert_eq!(oi, -1);
        assert_eq!(ic, 1);
    }

    #[test]
    fn test_drag_over_just_above_right_threshold() {
        let (oi, ic, _) = compute_drag_over_state(0.701, 1, 0, 4, Some(3), 0.30);
        assert_eq!(oi, -1);
        assert_eq!(ic, 2);
    }

    // ===== apply_shift_insert tests =====

    fn shift_pos(col: i32, row: i32) -> (i32, i32) {
        (col, row)
    }

    #[test]
    fn test_shift_drag_right_between_b_and_c() {
        // A(0,0) B(1,0) C(2,0) D(3,0), drag A to insert_lin=2
        // Expected: B A C D
        let p = vec![
            shift_pos(0, 0),
            shift_pos(1, 0),
            shift_pos(2, 0),
            shift_pos(3, 0),
        ];
        let r = apply_shift_insert(&p, 0, 2, 4);
        assert_eq!(r[0], shift_pos(1, 0)); // A -> lin 1
        assert_eq!(r[1], shift_pos(0, 0)); // B -> lin 0
        assert_eq!(r[2], shift_pos(2, 0)); // C stays
        assert_eq!(r[3], shift_pos(3, 0)); // D stays
    }

    #[test]
    fn test_shift_drag_left_between_a_and_b() {
        // A(0,0) B(1,0) C(2,0) D(3,0), drag D to insert_lin=1
        // Expected: A D B C
        let p = vec![
            shift_pos(0, 0),
            shift_pos(1, 0),
            shift_pos(2, 0),
            shift_pos(3, 0),
        ];
        let r = apply_shift_insert(&p, 3, 1, 4);
        assert_eq!(r[0], shift_pos(0, 0)); // A stays
        assert_eq!(r[1], shift_pos(2, 0)); // B -> lin 2
        assert_eq!(r[2], shift_pos(3, 0)); // C -> lin 3
        assert_eq!(r[3], shift_pos(1, 0)); // D -> lin 1
    }

    #[test]
    fn test_shift_cross_row() {
        // 3 cols: A(0,0) B(1,0) C(2,0) / D(0,1) E(1,1)
        // Drag A to insert_lin=4
        let p = vec![
            shift_pos(0, 0),
            shift_pos(1, 0),
            shift_pos(2, 0),
            shift_pos(0, 1),
            shift_pos(1, 1),
        ];
        let r = apply_shift_insert(&p, 0, 4, 3);
        assert_eq!(r[0], shift_pos(0, 1)); // A -> lin 3
        assert_eq!(r[1], shift_pos(0, 0)); // B -> lin 0
        assert_eq!(r[2], shift_pos(1, 0)); // C -> lin 1
        assert_eq!(r[3], shift_pos(2, 0)); // D -> lin 2
        assert_eq!(r[4], shift_pos(1, 1)); // E stays
    }

    #[test]
    fn test_shift_same_position_noop() {
        let p = vec![shift_pos(0, 0), shift_pos(1, 0), shift_pos(2, 0)];
        let r = apply_shift_insert(&p, 1, 1, 3);
        assert_eq!(r, p);
    }

    #[test]
    fn test_shift_single_item() {
        let p = vec![shift_pos(2, 1)];
        let r = apply_shift_insert(&p, 0, 0, 3);
        assert_eq!(r[0], shift_pos(0, 0));
    }

    #[test]
    fn test_shift_two_items_right() {
        // A(0,0) B(1,0), drag A to insert_lin=2
        let p = vec![shift_pos(0, 0), shift_pos(1, 0)];
        let r = apply_shift_insert(&p, 0, 2, 2);
        assert_eq!(r[0], shift_pos(1, 0)); // A -> lin 1
        assert_eq!(r[1], shift_pos(0, 0)); // B -> lin 0
    }
}
