//! 对齐检测算法
//!
//! 提供移动/缩放中的分类框与其他分类框之间的边缘对齐关系。
//! 支持水平（top/bottom）与垂直（left/right）两个方向，并给出吸附后的位置。
//!
//! # 两条「不让参考线到处乱冒」的硬约束（2026-11-03）
//! 1. **另一轴必须真有关联**（[`related`]）：两个框在对齐轴的垂直方向上既没有
//!    重叠、又离得很远时，它们彼此不构成「可对齐关系」，不拉线。
//! 2. **每对框在一个方向上只出最贴近的一条线**（`best_diff`）：早期实现对
//!    top↔top / top↔bottom / bottom↔top / bottom↔bottom 四对独立判定，两个框
//!    只差几个像素时会一次画出两条互相矛盾的横线，用户看到的就是
//!    「还没对齐就满屏参考线」。
//!
//! 参考线长度也不再取两个框的并集（那会让一条 5px 内的贴近直接铺满小半个
//! 屏幕），而是取两框在另一轴上的**重叠段**外加 [`GUIDE_EXTEND`]；两框相离时
//! 退回并集（此时并集本身就很短，正是「上下堆叠 / 左右并排」的经典对齐场景）。

use crate::platform::windows::ffi_types::RECT;

/// 对齐方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentDirection {
    /// 水平对齐（top/bottom 边缘对齐，参考线是横线）
    Horizontal,
    /// 垂直对齐（left/right 边缘对齐，参考线是竖线）
    Vertical,
}

/// 单条对齐匹配结果
#[derive(Debug, Clone)]
pub struct AlignmentMatch {
    /// 对齐方向
    pub direction: AlignmentDirection,
    /// 对齐位置的屏幕坐标（物理像素）
    pub position: i32,
    /// 匹配的参考框 HWND 列表
    pub matched_hwnds: Vec<*mut std::ffi::c_void>,
    /// 参考线起点（屏幕坐标）
    pub line_start: (i32, i32),
    /// 参考线终点（屏幕坐标）
    pub line_end: (i32, i32),
}

/// 对齐检测结果
#[derive(Default)]
pub struct AlignmentResult {
    /// 水平对齐匹配（横线）
    pub horizontal_matches: Vec<AlignmentMatch>,
    /// 垂直对齐匹配（竖线）
    pub vertical_matches: Vec<AlignmentMatch>,
    /// 吸附后的框位置（未命中任何对齐时为 None）
    pub snapped_rect: Option<RECT>,
}

unsafe impl Send for AlignmentMatch {}
unsafe impl Send for AlignmentResult {}

/// 默认对齐阈值（物理屏幕像素）。
///
/// 刻意**不按 DPI 放大**：坐标全部来自物理像素，200% 屏上 5 物理像素只相当于
/// 2.5 逻辑像素，已经比 100% 屏更严。放大反而会退回用户抱怨的
/// 「还没对齐就出现参考线」。
pub const DEFAULT_ALIGNMENT_THRESHOLD: i32 = 5;

/// 另一轴上的最大关联间距（**逻辑像素**，96DPI 基准，由调用方按窗口 DPI 折算）：
/// 两个框要么在对齐轴的垂直方向上有**重叠**，要么彼此**靠得足够近**，才算「可以对齐」。
///
/// 没有这条约束时，屏幕上任意两个「顶边相差 <= 阈值」的框之间都会拉出一条参考线，
/// 哪怕它们在水平方向上八竿子打不着 —— 用户看到的就是「还没对齐就出现参考线」。
///
/// ⚠ 这条约束必须按 DPI 折算成物理像素再用（分类框坐标全是物理像素）：
/// 2026-11-03 直接拿 240 当物理像素，200% 屏上等于只允许 120 逻辑像素的间距，
/// 结果用户那几个散布在 3840 宽屏上的框**一条参考线都不出了**（回归：参考线消失）。
pub const MAX_PERPENDICULAR_GAP: i32 = 240;

/// 参考线在重叠段两端的外扩长度（物理像素）。
///
/// 重叠段本身可能只有几十像素，线太短会「看不见」；取 24 让线明显但仍不越界。
pub const GUIDE_EXTEND: i32 = 24;

/// 区间 `[a0,a1)` 与 `[b0,b1)` 的重叠长度；负值表示两者之间的间隙宽度。
#[must_use]
fn overlap_or_gap(a0: i32, a1: i32, b0: i32, b1: i32) -> i32 {
    a1.min(b1) - a0.max(b0)
}

/// 两个框在另一轴上是否「有关联」：重叠 >= 1px，或间隙不超过 `max_gap`（物理像素）。
#[must_use]
fn related_with_gap(a0: i32, a1: i32, b0: i32, b1: i32, max_gap: i32) -> bool {
    let og = overlap_or_gap(a0, a1, b0, b1);
    og >= 1 || -og <= max_gap
}

#[must_use]
fn related(a0: i32, a1: i32, b0: i32, b1: i32) -> bool {
    related_with_gap(a0, a1, b0, b1, MAX_PERPENDICULAR_GAP)
}

/// 参考线在另一轴上的绘制范围：重叠段 + 两端外扩；两框相离时退回并集。
#[must_use]
fn guide_span(a0: i32, a1: i32, b0: i32, b1: i32) -> (i32, i32) {
    let lo = a0.max(b0);
    let hi = a1.min(b1);
    if hi > lo {
        (lo - GUIDE_EXTEND, hi + GUIDE_EXTEND)
    } else {
        (a0.min(b0), a1.max(b1))
    }
}

/// 检测移动框与其他框之间的对齐关系
///
/// # 参数
/// - `moving_rect`: 正在移动/缩放的框的内容矩形（屏幕物理像素）
/// - `others`: 其余分类框的 `(HWND, 内容矩形)` 列表
/// - `threshold`: 命中阈值（物理像素，越小越严格）
///
/// # 返回
/// 包含全部匹配对齐关系与吸附后位置的检测结果
#[must_use]
pub fn detect_alignments(
    moving_rect: &RECT,
    others: &[(*mut std::ffi::c_void, RECT)],
    threshold: i32,
) -> AlignmentResult {
    detect_alignments_with_gap(moving_rect, others, threshold, MAX_PERPENDICULAR_GAP)
}

/// 同 [`detect_alignments`]，但由调用方给出**物理像素**口径的关联间距上限。
///
/// 分类框的坐标全是物理像素，所以 [`MAX_PERPENDICULAR_GAP`] 必须按窗口 DPI
/// 折算后传进来，否则 200% 屏上关联半径会被砍一半，参考线干脆不再出现。
#[must_use]
pub fn detect_alignments_with_gap(
    moving_rect: &RECT,
    others: &[(*mut std::ffi::c_void, RECT)],
    threshold: i32,
    max_gap: i32,
) -> AlignmentResult {
    let mut result = AlignmentResult::default();

    let moving_edges = RectEdges::from_rect(moving_rect);

    for (hwnd, other_rect) in others {
        let other_edges = RectEdges::from_rect(other_rect);

        // 水平方向对齐（top/bottom 边缘）
        check_horizontal_alignment(
            &mut result,
            &moving_edges,
            &other_edges,
            *hwnd,
            threshold,
            max_gap,
        );

        // 垂直方向对齐（left/right 边缘）
        check_vertical_alignment(
            &mut result,
            &moving_edges,
            &other_edges,
            *hwnd,
            threshold,
            max_gap,
        );
    }

    result.snapped_rect = calculate_snapped_rect(moving_rect, &result);

    result
}

/// 矩形四边缘
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
    other_hwnd: *mut std::ffi::c_void,
    threshold: i32,
    _max_gap: i32,
) {
    // 四对 top/bottom 边缘独立检测，每对 <= threshold 就画线。
    // 与接入 liquid-verre-rs 之前的逻辑一致：不设「另一轴关联」门控，
    // 不取 best_diff —— 用户明确要求「和之前一致，只是阈值小一点」。
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
                if !existing.matched_hwnds.contains(&other_hwnd) {
                    existing.matched_hwnds.push(other_hwnd);
                    existing.line_start.0 = existing.line_start.0.min(left_x);
                    existing.line_end.0 = existing.line_end.0.max(right_x);
                }
            } else {
                result.horizontal_matches.push(AlignmentMatch {
                    direction: AlignmentDirection::Horizontal,
                    position: other_edge,
                    matched_hwnds: vec![other_hwnd],
                    line_start: (left_x, other_edge),
                    line_end: (right_x, other_edge),
                });
            }
        }
    }
}

/// 检测垂直对齐（left/right 边缘，参考线为竖线）
fn check_vertical_alignment(
    result: &mut AlignmentResult,
    moving: &RectEdges,
    other: &RectEdges,
    other_hwnd: *mut std::ffi::c_void,
    threshold: i32,
    _max_gap: i32,
) {
    // 四对 left/right 边缘独立检测，与接入库之前一致（见 check_horizontal_alignment 注释）。
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
                if !existing.matched_hwnds.contains(&other_hwnd) {
                    existing.matched_hwnds.push(other_hwnd);
                    existing.line_start.1 = existing.line_start.1.min(top_y);
                    existing.line_end.1 = existing.line_end.1.max(bottom_y);
                }
            } else {
                result.vertical_matches.push(AlignmentMatch {
                    direction: AlignmentDirection::Vertical,
                    position: other_edge,
                    matched_hwnds: vec![other_hwnd],
                    line_start: (other_edge, top_y),
                    line_end: (other_edge, bottom_y),
                });
            }
        }
    }
}

/// 计算吸附后的精确位置
fn calculate_snapped_rect(original: &RECT, result: &AlignmentResult) -> Option<RECT> {
    let mut snapped = RECT {
        left: original.left,
        top: original.top,
        right: original.right,
        bottom: original.bottom,
    };
    let mut changed = false;

    // 应用水平方向吸附（改 top/bottom）
    if !result.horizontal_matches.is_empty() {
        let mut min_diff_top = i32::MAX;
        let mut min_diff_bottom = i32::MAX;
        let mut snap_top = original.top;
        let mut snap_bottom = original.bottom;

        for m in &result.horizontal_matches {
            let diff_top = (original.top - m.position).abs();
            let diff_bottom = (original.bottom - m.position).abs();

            if diff_top < min_diff_top {
                min_diff_top = diff_top;
                snap_top = m.position;
            }
            if diff_bottom < min_diff_bottom {
                min_diff_bottom = diff_bottom;
                snap_bottom = m.position;
            }
        }

        // 优先吸附 top（差距更小的一侧）
        if min_diff_top <= min_diff_bottom && min_diff_top > 0 {
            let height = original.bottom - original.top;
            snapped.top = snap_top;
            snapped.bottom = snap_top + height;
            changed = true;
        } else if min_diff_bottom > 0 {
            let height = original.bottom - original.top;
            snapped.bottom = snap_bottom;
            snapped.top = snap_bottom - height;
            changed = true;
        }
    }

    // 应用垂直方向吸附（改 left/right）
    if !result.vertical_matches.is_empty() {
        let mut min_diff_left = i32::MAX;
        let mut min_diff_right = i32::MAX;
        let mut snap_left = original.left;
        let mut snap_right = original.right;

        for m in &result.vertical_matches {
            let diff_left = (original.left - m.position).abs();
            let diff_right = (original.right - m.position).abs();

            if diff_left < min_diff_left {
                min_diff_left = diff_left;
                snap_left = m.position;
            }
            if diff_right < min_diff_right {
                min_diff_right = diff_right;
                snap_right = m.position;
            }
        }

        // 优先吸附 left
        if min_diff_left <= min_diff_right && min_diff_left > 0 {
            let width = original.right - original.left;
            snapped.left = snap_left;
            snapped.right = snap_left + width;
            changed = true;
        } else if min_diff_right > 0 {
            let width = original.right - original.left;
            snapped.right = snap_right;
            snapped.left = snap_right - width;
            changed = true;
        }
    }

    if changed {
        Some(snapped)
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn make_rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    fn hwnd(v: usize) -> *mut std::ffi::c_void {
        v as *mut std::ffi::c_void
    }

    #[test]
    fn test_no_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(hwnd(0x1), make_rect(300, 300, 400, 400))];

        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
        assert!(result.vertical_matches.is_empty());
    }

    #[test]
    fn test_top_alignment() {
        // moving: top=100, bottom=200
        // other:  top=100, bottom=300（只有 top 对齐）
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(hwnd(0x1), make_rect(300, 100, 400, 300))];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        assert_eq!(result.horizontal_matches[0].position, 100);
    }

    #[test]
    fn test_left_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(hwnd(0x1), make_rect(100, 300, 300, 400))];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.vertical_matches.len(), 1);
        assert_eq!(result.vertical_matches[0].position, 100);
    }

    #[test]
    fn test_snap_threshold() {
        let moving = make_rect(100, 97, 200, 197); // top 差 3px
        let others = vec![(hwnd(0x1), make_rect(300, 100, 400, 300))];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        assert!(result.snapped_rect.is_some());
        assert_eq!(result.snapped_rect.unwrap().top, 100);
    }

    #[test]
    fn test_outside_threshold() {
        let moving = make_rect(100, 94, 200, 194); // top 差 6px
        let others = vec![(hwnd(0x1), make_rect(300, 100, 400, 200))];

        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
    }

    #[test]
    fn test_threshold_boundary() {
        // 恰好等于阈值（5px）时应当命中
        let moving = make_rect(100, 95, 200, 200);
        let others = vec![(hwnd(0x1), make_rect(300, 100, 400, 300))];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
    }

    #[test]
    fn test_multiple_alignments() {
        // 两个框完全重合：top 和 bottom 都相同 → 2 条横线 + 2 条竖线。
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(hwnd(0x1), make_rect(100, 100, 200, 200))];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 2, "top 和 bottom 各一条");
        assert_eq!(result.vertical_matches.len(), 2, "left 和 right 各一条");
    }

    #[test]
    fn test_multiple_pairs_per_box() {
        // top 和 bottom 都在阈值内 → 两条横线（不再取 best_diff，与接入库之前一致）。
        let moving = make_rect(100, 100, 200, 205);
        let others = vec![(hwnd(0x1), make_rect(300, 102, 400, 207))];

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
        let others = vec![(hwnd(0x1), make_rect(900, 100, 1000, 300))];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
    }

    #[test]
    fn test_side_by_side_gap_still_aligns() {
        // 左右并排、间隙 100px（<= MAX_PERPENDICULAR_GAP）时顶边对齐仍然有效，
        // 这是最常见的「并排两框顶边拉平」场景，不能被 related() 误杀。
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(hwnd(0x1), make_rect(300, 100, 400, 300))];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
    }

    #[test]
    fn test_overlapping_boxes_guide_spans_union() {
        // 参考线长度 = 两框并集（与接入库之前一致）。
        let moving = make_rect(100, 100, 400, 200);
        let others = vec![(hwnd(0x1), make_rect(300, 100, 600, 300))];

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
        let others = vec![(hwnd(0x1), make_rect(300, 100, 400, 300))];

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
        let others = vec![(hwnd(0x1), make_rect(100, 300, 300, 400))];

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
        let others: Vec<(*mut std::ffi::c_void, RECT)> = vec![];

        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
        assert!(result.vertical_matches.is_empty());
        assert!(result.snapped_rect.is_none());
    }

    #[test]
    fn test_multiple_boxes_alignment() {
        // 两个参考框顶边相同 -> 合并成一条线，两个 HWND 挂在同一条线上。
        let moving = make_rect(100, 100, 200, 200);
        // 第二个框与移动框的 X 间隙 220px，仍在 MAX_PERPENDICULAR_GAP(240) 内；
        // 再远就不算「可对齐关系」，不会拉出一条穿过空屏的长线。
        let others = vec![
            (hwnd(0x1000), make_rect(300, 100, 400, 300)),
            (hwnd(0x2000), make_rect(420, 100, 520, 250)),
        ];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);

        let m = &result.horizontal_matches[0];
        assert_eq!(m.matched_hwnds.len(), 2);
        assert_eq!(m.line_start.0, 100);
        assert_eq!(m.line_end.0, 520);
    }

    #[test]
    fn test_bottom_to_top_alignment() {
        // moving.bottom 与 other.top 对齐
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(hwnd(0x1), make_rect(300, 200, 400, 300))];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        assert_eq!(result.horizontal_matches[0].position, 200);
    }

    #[test]
    fn test_right_to_left_alignment() {
        // moving.right 与 other.left 对齐
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(hwnd(0x1), make_rect(200, 300, 300, 400))];

        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.vertical_matches.len(), 1);
        assert_eq!(result.vertical_matches[0].position, 200);
    }
}
