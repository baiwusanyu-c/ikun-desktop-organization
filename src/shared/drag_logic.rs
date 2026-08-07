//! 拖拽纯逻辑
//!
//! 从 platform/windows/drag_visual.rs 提取的平台无关拖拽状态计算函数。
//! macOS 重新实现时可直接复用。

/// 根据光标在单元格中的相对位置，计算拖拽覆盖状态。
///
/// 参数:
/// - ratio_x: 光标在单元格中的水平比例 (0.0~1.0)
/// - col, row: 当前单元格坐标
/// - cols: 总列数
/// - item_at_cell: 该单元格上是否有项（Some(index) 或 None）
/// - insert_threshold: 插入阈值（通常 0.30）
///
/// 返回 (drag_over_item, drag_over_insert_col, drag_over_insert_row):
/// - 插入模式: (-1, col, row)
/// - 替换模式: (item_index, -1, -1)
/// - 无操作: (-1, -1, -1)
pub fn compute_drag_over_state(
    ratio_x: f32,
    col: i32,
    row: i32,
    cols: i32,
    item_at_cell: Option<i32>,
    insert_threshold: f32,
) -> (i32, i32, i32) {
    if ratio_x < insert_threshold {
        (-1, col, row)
    } else if ratio_x > 1.0 - insert_threshold {
        if col + 1 < cols {
            (-1, col + 1, row)
        } else {
            (item_at_cell.unwrap_or(-1), -1, -1)
        }
    } else {
        (item_at_cell.unwrap_or(-1), -1, -1)
    }
}

/// Shift-drag 插入：将被拖拽项插入到指定位置，其余项按线性顺序平移。
pub fn apply_shift_insert(
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
        assert_eq!(ic, 2);
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
        assert_eq!(oi, 3);
        let (oi, _, _) = compute_drag_over_state(0.70, 1, 0, 4, Some(3), 0.30);
        assert_eq!(oi, 3);
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

    fn shift_pos(col: i32, row: i32) -> (i32, i32) {
        (col, row)
    }

    #[test]
    fn test_shift_drag_right_between_b_and_c() {
        let p = vec![
            shift_pos(0, 0),
            shift_pos(1, 0),
            shift_pos(2, 0),
            shift_pos(3, 0),
        ];
        let r = apply_shift_insert(&p, 0, 2, 4);
        assert_eq!(r[0], shift_pos(1, 0));
        assert_eq!(r[1], shift_pos(0, 0));
        assert_eq!(r[2], shift_pos(2, 0));
        assert_eq!(r[3], shift_pos(3, 0));
    }

    #[test]
    fn test_shift_drag_left_between_a_and_b() {
        let p = vec![
            shift_pos(0, 0),
            shift_pos(1, 0),
            shift_pos(2, 0),
            shift_pos(3, 0),
        ];
        let r = apply_shift_insert(&p, 3, 1, 4);
        assert_eq!(r[0], shift_pos(0, 0));
        assert_eq!(r[1], shift_pos(2, 0));
        assert_eq!(r[2], shift_pos(3, 0));
        assert_eq!(r[3], shift_pos(1, 0));
    }

    #[test]
    fn test_shift_cross_row() {
        let p = vec![
            shift_pos(0, 0),
            shift_pos(1, 0),
            shift_pos(2, 0),
            shift_pos(0, 1),
            shift_pos(1, 1),
        ];
        let r = apply_shift_insert(&p, 0, 4, 3);
        assert_eq!(r[0], shift_pos(0, 1));
        assert_eq!(r[1], shift_pos(0, 0));
        assert_eq!(r[2], shift_pos(1, 0));
        assert_eq!(r[3], shift_pos(2, 0));
        assert_eq!(r[4], shift_pos(1, 1));
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
        let p = vec![shift_pos(0, 0), shift_pos(1, 0)];
        let r = apply_shift_insert(&p, 0, 2, 2);
        assert_eq!(r[0], shift_pos(1, 0));
        assert_eq!(r[1], shift_pos(0, 0));
    }
}
