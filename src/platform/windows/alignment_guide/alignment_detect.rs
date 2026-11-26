//! Alignment detection for moving/resizing boxes.
//!
//! The four edge pairings on each axis are checked independently:
//! horizontal checks top/top, top/bottom, bottom/top and bottom/bottom;
//! vertical checks left/left, left/right, right/left and right/right.
//! Matches at the same guide position are merged, and the guide segment
//! spans the union of the participating rectangles.

use crate::platform::windows::ffi_types::RECT;

/// A single alignment guide.
#[derive(Debug, Clone)]
pub struct AlignmentMatch {
    /// Screen-space position of the guide in physical pixels.
    pub position: i32,
    /// Screen-space guide start point.
    pub line_start: (i32, i32),
    /// Screen-space guide end point.
    pub line_end: (i32, i32),
}

/// Alignment detection result.
#[derive(Default)]
pub struct AlignmentResult {
    /// Horizontal matches (horizontal guide lines).
    pub horizontal_matches: Vec<AlignmentMatch>,
    /// Vertical matches (vertical guide lines).
    pub vertical_matches: Vec<AlignmentMatch>,
}

/// 默认对齐阈值（**逻辑**像素，96 DPI 基准）。
pub const DEFAULT_ALIGNMENT_THRESHOLD: i32 = 5;

/// 把逻辑阈值折算成当前 DPI 下的物理像素阈值。
///
/// 检测用的是 `GetWindowRect` 的物理坐标，固定 5 物理像素在 192 DPI 主屏上只相当
/// 2.5 逻辑像素（副屏的一半），表现为「主屏吸不住、副屏吸得太轻松」，
/// 跨屏对齐尤其明显。夹到 [3, 12]：下限保证任何 DPI 都吸得住，
/// 上限避免超高缩放下吸力过强造成误对齐。
#[must_use]
pub fn alignment_threshold_for_dpi(dpi: u32) -> i32 {
    let scaled = DEFAULT_ALIGNMENT_THRESHOLD.saturating_mul(dpi.max(96) as i32) / 96;
    scaled.clamp(3, 12)
}

/// Detect alignment relationships between the moving rectangle and other boxes.
///
/// # Parameters
/// - `moving_rect`: content rectangle of the moving/resizing box in physical pixels.
/// - `threshold`: hit threshold in physical pixels.
#[must_use]
pub fn detect_alignments(moving_rect: &RECT, others: &[RECT], threshold: i32) -> AlignmentResult {
    let mut result = AlignmentResult::default();

    let moving_edges = RectEdges::from_rect(moving_rect);

    for other_rect in others {
        let other_edges = RectEdges::from_rect(other_rect);

        check_horizontal_alignment(&mut result, &moving_edges, &other_edges, threshold);

        check_vertical_alignment(&mut result, &moving_edges, &other_edges, threshold);
    }

    result
}

/// Rect edges.
struct RectEdges {
    top: i32,
    bottom: i32,
    left: i32,
    right: i32,
}

impl RectEdges {
    fn from_rect(rect: &RECT) -> Self {
        Self {
            top: rect.top,
            bottom: rect.bottom,
            left: rect.left,
            right: rect.right,
        }
    }
}

/// 检测水平对齐（top/bottom 边缘，参考线为横线）
fn check_horizontal_alignment(
    result: &mut AlignmentResult,
    moving: &RectEdges,
    other: &RectEdges,
    threshold: i32,
) {
    let pairs = [
        (moving.top, other.top),
        (moving.top, other.bottom),
        (moving.bottom, other.top),
        (moving.bottom, other.bottom),
    ];

    for (moving_edge, other_edge) in pairs {
        let diff = (moving_edge - other_edge).abs();
        if diff <= threshold {
            let left_x = moving.left.min(other.left);
            let right_x = moving.right.max(other.right);
            let existing = result
                .horizontal_matches
                .iter_mut()
                .find(|m| (m.position - other_edge).abs() <= 1);

            if let Some(existing) = existing {
                existing.line_start.0 = existing.line_start.0.min(left_x);
                existing.line_end.0 = existing.line_end.0.max(right_x);
            } else {
                result.horizontal_matches.push(AlignmentMatch {
                    position: other_edge,
                    line_start: (left_x, other_edge),
                    line_end: (right_x, other_edge),
                });
            }
        }
    }
}

fn check_vertical_alignment(
    result: &mut AlignmentResult,
    moving: &RectEdges,
    other: &RectEdges,
    threshold: i32,
) {
    let pairs = [
        (moving.left, other.left),
        (moving.left, other.right),
        (moving.right, other.left),
        (moving.right, other.right),
    ];

    for (moving_edge, other_edge) in pairs {
        let diff = (moving_edge - other_edge).abs();
        if diff <= threshold {
            let top_y = moving.top.min(other.top);
            let bottom_y = moving.bottom.max(other.bottom);
            let existing = result
                .vertical_matches
                .iter_mut()
                .find(|m| (m.position - other_edge).abs() <= 1);

            if let Some(existing) = existing {
                existing.line_start.1 = existing.line_start.1.min(top_y);
                existing.line_end.1 = existing.line_end.1.max(bottom_y);
            } else {
                result.vertical_matches.push(AlignmentMatch {
                    position: other_edge,
                    line_start: (other_edge, top_y),
                    line_end: (other_edge, bottom_y),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn threshold_scales_with_dpi_and_stays_clamped() {
        assert_eq!(alignment_threshold_for_dpi(96), 5, "96 DPI = 逻辑基准");
        assert_eq!(alignment_threshold_for_dpi(192), 10, "200% 下容差同步放大");
        assert_eq!(alignment_threshold_for_dpi(144), 7);
        assert_eq!(alignment_threshold_for_dpi(480), 12, "上限夹住，避免误对齐");
        assert_eq!(alignment_threshold_for_dpi(0), 5, "非法 dpi 退回基准");
    }

    #[test]
    fn cross_monitor_pairwise_alignment_is_detected() {
        // 主屏一个框 + 副屏一个框 + 主屏另一个框：拖主屏框时必须同时
        // 命中跨屏的横线与同屏的竖线（检测不得按显示器过滤）。
        let moving = RECT {
            left: 200,
            top: 300,
            right: 600,
            bottom: 500,
        };
        // 副屏框：顶边 302，与 moving.top=300 差 2px -> 横线命中
        let secondary_a = RECT {
            left: 3120,
            top: 302,
            right: 3520,
            bottom: 520,
        };
        // 主屏另一个框：左边 601，与 moving.right=600 差 1px -> 竖线命中
        let primary_c = RECT {
            left: 601,
            top: 800,
            right: 900,
            bottom: 1000,
        };
        let r = detect_alignments(&moving, &[secondary_a, primary_c], 5);

        let h = r
            .horizontal_matches
            .iter()
            .find(|m| m.position == 302)
            .unwrap_or_else(|| panic!("跨屏顶边对齐没命中: {:?}", r.horizontal_matches));
        assert!(
            h.line_start.0 <= 200 && h.line_end.0 >= 3520,
            "横线必须覆盖两个框的并集（跨屏）: {:?}",
            h
        );

        let v = r
            .vertical_matches
            .iter()
            .find(|m| m.position == 601)
            .unwrap_or_else(|| panic!("同屏左边对齐没命中: {:?}", r.vertical_matches));
        assert!(
            v.line_start.1 <= 300 && v.line_end.1 >= 1000,
            "竖线必须覆盖两个框的并集: {:?}",
            v
        );
    }

    use super::*;

    fn make_rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn test_no_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(300, 300, 400, 400)];

        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
        assert!(result.vertical_matches.is_empty());
    }

    #[test]
    fn test_top_alignment() {
        // moving: top=100, bottom=200
        // other:  top=100, bottom=300（只有 top 对齐）
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(300, 100, 400, 300)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        assert_eq!(result.horizontal_matches[0].position, 100);
    }

    #[test]
    fn test_left_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(100, 300, 300, 400)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.vertical_matches.len(), 1);
        assert_eq!(result.vertical_matches[0].position, 100);
    }

    #[test]
    fn test_threshold_hit() {
        let moving = make_rect(100, 97, 200, 197); // top 差 3px
        let others = vec![make_rect(300, 100, 400, 300)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
    }

    #[test]
    fn test_outside_threshold() {
        let moving = make_rect(100, 94, 200, 194); // top 差 6px
        let others = vec![make_rect(300, 100, 400, 200)];

        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
    }

    #[test]
    fn test_threshold_boundary() {
        // 恰好等于阈值（5px）时应当命中
        let moving = make_rect(100, 95, 200, 200);
        let others = vec![make_rect(300, 100, 400, 300)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
    }

    #[test]
    fn test_multiple_alignments() {
        // 两个框完全重合：top 和 bottom 都相同 → 2 条横线 + 2 条竖线。
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(100, 100, 200, 200)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 2, "top 和 bottom 各一条");
        assert_eq!(result.vertical_matches.len(), 2, "left 和 right 各一条");
    }

    #[test]
    fn test_multiple_pairs_per_box() {
        let moving = make_rect(100, 100, 200, 205);
        let others = vec![make_rect(300, 102, 400, 207)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 2);
        let positions: Vec<i32> = result
            .horizontal_matches
            .iter()
            .map(|m| m.position)
            .collect();
        assert!(positions.contains(&102));
        assert!(positions.contains(&207));
    }

    #[test]
    fn test_far_boxes_align_if_edges_match() {
        // 顶边相同 → 横线，即使 X 相距很远也画（与接入库之前一致：不设 gap 门控）。
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(900, 100, 1000, 300)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
    }

    #[test]
    fn test_side_by_side_gap_still_aligns() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(300, 100, 400, 300)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
    }

    #[test]
    fn test_overlapping_boxes_guide_union() {
        // 参考线长度 = 两框并集（与接入库之前一致）。
        let moving = make_rect(100, 100, 400, 200);
        let others = vec![make_rect(300, 100, 600, 300)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        let m = &result.horizontal_matches[0];
        assert_eq!(m.line_start.0, 100, "union left");
        assert_eq!(m.line_end.0, 600, "union right");
    }

    #[test]
    fn test_line_endpoints_horizontal() {
        // 两框相离时用并集，保证「上下堆叠 / 左右并排」的参考线覆盖两个框。
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(300, 100, 400, 300)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);

        let m = &result.horizontal_matches[0];
        assert_eq!(m.line_start.0, 100);
        assert_eq!(m.line_start.1, 100);
        assert_eq!(m.line_end.0, 400);
        assert_eq!(m.line_end.1, 100);
    }

    #[test]
    fn test_line_endpoints_vertical() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(100, 300, 300, 400)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.vertical_matches.len(), 1);

        let m = &result.vertical_matches[0];
        assert_eq!(m.line_start.0, 100);
        assert_eq!(m.line_start.1, 100);
        assert_eq!(m.line_end.0, 100);
        assert_eq!(m.line_end.1, 400);
    }

    #[test]
    fn test_empty_others() {
        let moving = make_rect(100, 100, 200, 200);
        let others: Vec<RECT> = vec![];

        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
        assert!(result.vertical_matches.is_empty());
    }

    #[test]
    fn test_multiple_boxes_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(300, 100, 400, 300), make_rect(420, 100, 520, 250)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);

        let m = &result.horizontal_matches[0];
        assert_eq!(m.line_start.0, 100);
        assert_eq!(m.line_end.0, 520);
    }

    #[test]
    fn test_bottom_to_top_alignment() {
        // moving.bottom 与 other.top 对齐
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(300, 200, 400, 300)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        assert_eq!(result.horizontal_matches[0].position, 200);
    }

    #[test]
    fn test_right_to_left_alignment() {
        // moving.right 与 other.left 对齐
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![make_rect(200, 300, 300, 400)];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.vertical_matches.len(), 1);
        assert_eq!(result.vertical_matches[0].position, 200);
    }
}
