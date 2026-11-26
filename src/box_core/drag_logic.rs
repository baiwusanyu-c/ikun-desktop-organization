//! 拖拽纯逻辑
//!
//! 从 platform/windows/drag_visual.rs 提取的平台无关拖拽状态计算函数。
//! 分类框与桌面覆盖层共用的纯逻辑，无平台依赖。

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

// =============================================================================
// 拖拽启动判据（DPI 阈值 + 起拖位置）
//
// 背景（2026-11-26 用户报「在分类框中双击图标打开，有时候没反应」）：
// 旧实现用硬编码 `DRAG_THRESHOLD = 5` 物理像素判阈值，既不随 DPI 放大，
// 又允许「光标仍在框内」就发起阻塞式 OLE 拖拽。分类框常驻 HWND_BOTTOM，
// 只要资源管理器窗口几何上盖在它上面，单击图标 + 手抖 6px 就会进入
// DoDragDrop 模态循环并把本框置为 WS_EX_TRANSPARENT —— 双击的第二次按下
// 被彻底吞掉（表现为「没反应」），还会误发起一次文件复制。
// =============================================================================

/// 拖拽启动阈值下限（物理像素），与历史行为保持一致。
pub const MIN_DRAG_THRESHOLD_PX: i32 = 5;
/// 拖拽启动阈值上限（物理像素），高 DPI 下不得放大到「拖不动」。
pub const MAX_DRAG_THRESHOLD_PX: i32 = 16;

/// 计算拖拽启动阈值（物理像素）。
///
/// 系统拖拽阈值 `SM_CXDRAG/SM_CYDRAG` 以主屏（96 DPI）逻辑像素为准，
/// 在高 DPI 分类框上必须按 DPI 等比放大，否则 200% 缩放下手抖 5 物理像素
/// 就等于 100% 下的 10 像素，极易误判成拖拽。
///
/// - `system_drag_px <= 0`（取不到系统值）→ 回退 [`MIN_DRAG_THRESHOLD_PX`]；
/// - 结果夹在 [`MIN_DRAG_THRESHOLD_PX`, `MAX_DRAG_THRESHOLD_PX`]。
#[must_use]
pub fn drag_threshold_physical_px(system_drag_px: i32, dpi: u32) -> i32 {
    if system_drag_px <= 0 {
        return MIN_DRAG_THRESHOLD_PX;
    }
    let dpi = if dpi == 0 { 96 } else { dpi };
    let scaled = (system_drag_px as i64 * dpi as i64 / 96)
        .clamp(MIN_DRAG_THRESHOLD_PX as i64, MAX_DRAG_THRESHOLD_PX as i64);
    scaled as i32
}

/// 位移是否超过拖拽阈值（任一轴超过即算，与旧实现判据一致）。
#[must_use]
pub fn drag_threshold_exceeded(start: (i32, i32), current: (i32, i32), threshold: i32) -> bool {
    let dx = (current.0 as i64 - start.0 as i64).abs();
    let dy = (current.1 as i64 - start.1 as i64).abs();
    dx > threshold as i64 || dy > threshold as i64
}

/// 点是否落在矩形内（半开区间 `[left,right)` / `[top,bottom)`）。
#[must_use]
pub fn point_in_rect(x: i32, y: i32, rect: &[i32; 4]) -> bool {
    x >= rect[0] && x < rect[2] && y >= rect[1] && y < rect[3]
}

/// 自定义拖拽残影是否应升级为「拖往外部窗口」的 OLE 拖拽。
///
/// 判据：光标已离开起拖时记录的分类框内容矩形。
/// 未离开时不得起 OLE —— `DoDragDrop` 会阻塞本框消息线程并把窗口置为
/// 鼠标透明，双击/后续点击全部丢失（见本段顶部注释）。
#[must_use]
pub fn should_start_external_ole_drag(
    cursor: (i32, i32),
    source_box_rect: &[i32; 4],
    external_window_found: bool,
) -> bool {
    // 源矩形无效时必须失败关闭：宁可本次不升级 OLE 拖拽，也不能在无法证明
    // 光标已离开分类框时进入阻塞式 DoDragDrop（会吞掉双击并误复制文件）。
    let source_rect_valid =
        source_box_rect[2] > source_box_rect[0] && source_box_rect[3] > source_box_rect[1];
    external_window_found
        && source_rect_valid
        && !point_in_rect(cursor.0, cursor.1, source_box_rect)
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

    // ===== 拖拽启动判据 =====

    #[test]
    fn drag_threshold_falls_back_when_system_value_missing() {
        assert_eq!(drag_threshold_physical_px(0, 96), MIN_DRAG_THRESHOLD_PX);
        assert_eq!(drag_threshold_physical_px(-4, 192), MIN_DRAG_THRESHOLD_PX);
    }

    #[test]
    fn drag_threshold_scales_with_dpi() {
        // 100%：系统值 4 → 取下限 5（保持历史行为）
        assert_eq!(drag_threshold_physical_px(4, 96), 5);
        // 150%：4*1.5=6
        assert_eq!(drag_threshold_physical_px(4, 144), 6);
        // 200%：4*2=8
        assert_eq!(drag_threshold_physical_px(4, 192), 8);
        // 极端系统值必须被上限夹住，否则高 DPI 下永远拖不起来
        assert_eq!(drag_threshold_physical_px(64, 192), MAX_DRAG_THRESHOLD_PX);
    }

    #[test]
    fn drag_threshold_boundary_is_exclusive() {
        // 恰好等于阈值不算拖拽（与旧 `dx <= THRESHOLD` 判据一致）
        assert!(!drag_threshold_exceeded((100, 100), (105, 100), 5));
        assert!(drag_threshold_exceeded((100, 100), (106, 100), 5));
        assert!(drag_threshold_exceeded((100, 100), (100, 106), 5));
    }

    #[test]
    fn drag_threshold_handles_negative_coordinates() {
        // 负坐标副屏上不得因 i32 相减溢出误判
        assert!(drag_threshold_exceeded((-4000, -200), (-3990, -200), 5));
        assert!(!drag_threshold_exceeded((-4000, -200), (-3996, -200), 5));
    }

    #[test]
    fn point_in_rect_is_half_open() {
        let rect = [0, 0, 100, 100];
        assert!(point_in_rect(0, 0, &rect));
        assert!(point_in_rect(99, 99, &rect));
        assert!(!point_in_rect(100, 50, &rect));
        assert!(!point_in_rect(-1, 50, &rect));
    }

    #[test]
    fn external_ole_drag_suppressed_while_cursor_inside_box() {
        let box_rect = [100, 100, 500, 400];
        // 光标还在框内（哪怕几何上被资源管理器盖住）→ 绝不起 OLE
        assert!(!should_start_external_ole_drag((300, 200), &box_rect, true));
        // 光标已离开框且命中外部窗口 → 允许起 OLE
        assert!(should_start_external_ole_drag((900, 200), &box_rect, true));
        // 没命中外部窗口时永远不起
        assert!(!should_start_external_ole_drag(
            (900, 200),
            &box_rect,
            false
        ));
    }

    #[test]
    fn external_ole_drag_fails_closed_for_invalid_source_rect() {
        // 取不到内容矩形（全 0）时不得退回历史“全放行”行为。
        assert!(!should_start_external_ole_drag(
            (900, 200),
            &[0, 0, 0, 0],
            true
        ));
        assert!(!should_start_external_ole_drag(
            (900, 200),
            &[500, 500, 100, 100],
            true
        ));
    }
}
