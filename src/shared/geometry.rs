//! 几何计算
//!
//! 从 platform/windows/alignment_detect.rs 和 icon_bitmap.rs 提取的平台无关几何算法。
//! macOS 重新实现时可直接复用。

use super::types::{Pixel32, Rect};

// ============================================================================
// 对齐检测（从 alignment_detect.rs 提取，已删除废弃的吸附代码）
// ============================================================================

/// 对齐方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentDirection {
    /// 水平对齐（top/bottom 边缘对齐）
    Horizontal,
    /// 垂直对齐（left/right 边缘对齐）
    Vertical,
}

/// 对齐匹配结果
#[derive(Debug, Clone)]
pub struct AlignmentMatch {
    /// 对齐方向
    pub direction: AlignmentDirection,
    /// 对齐位置（像素坐标）
    pub position: i32,
    /// 匹配的对象标识（平台无关，Windows 用 HWND，macOS 用窗口 ID）
    pub matched_ids: Vec<u64>,
    /// 参考线起点（屏幕坐标）
    pub line_start: (i32, i32),
    /// 参考线终点（屏幕坐标）
    pub line_end: (i32, i32),
}

/// 对齐检测结果
#[derive(Default)]
pub struct AlignmentResult {
    /// 水平对齐匹配（水平方向对齐）
    pub horizontal_matches: Vec<AlignmentMatch>,
    /// 垂直对齐匹配（垂直方向对齐）
    pub vertical_matches: Vec<AlignmentMatch>,
}

/// 默认对齐阈值（像素）
pub const DEFAULT_ALIGNMENT_THRESHOLD: i32 = 5;

/// 检测移动矩形与其他矩形之间的对齐关系
pub fn detect_alignments(
    moving_rect: &Rect,
    others: &[(u64, Rect)],
    threshold: i32,
) -> AlignmentResult {
    let mut result = AlignmentResult::default();

    let moving_edges = RectEdges::from_rect(moving_rect);

    for (id, other_rect) in others {
        let other_edges = RectEdges::from_rect(other_rect);

        check_horizontal_alignment(&mut result, &moving_edges, &other_edges, *id, threshold);
        check_vertical_alignment(&mut result, &moving_edges, &other_edges, *id, threshold);
    }

    result
}

struct RectEdges {
    top: i32,
    bottom: i32,
    left: i32,
    right: i32,
}

impl RectEdges {
    fn from_rect(rect: &Rect) -> Self {
        Self {
            top: rect.top,
            bottom: rect.bottom,
            left: rect.left,
            right: rect.right,
        }
    }
}

fn check_horizontal_alignment(
    result: &mut AlignmentResult,
    moving: &RectEdges,
    other: &RectEdges,
    other_id: u64,
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
            let existing = result
                .horizontal_matches
                .iter_mut()
                .find(|m| (m.position - other_edge).abs() <= 1);

            if let Some(existing) = existing {
                if !existing.matched_ids.contains(&other_id) {
                    existing.matched_ids.push(other_id);
                    existing.line_start.0 = existing.line_start.0.min(moving.left).min(other.left);
                    existing.line_end.0 = existing.line_end.0.max(moving.right).max(other.right);
                }
            } else {
                result.horizontal_matches.push(AlignmentMatch {
                    direction: AlignmentDirection::Horizontal,
                    position: other_edge,
                    matched_ids: vec![other_id],
                    line_start: (moving.left.min(other.left), other_edge),
                    line_end: (moving.right.max(other.right), other_edge),
                });
            }
        }
    }
}

fn check_vertical_alignment(
    result: &mut AlignmentResult,
    moving: &RectEdges,
    other: &RectEdges,
    other_id: u64,
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
            let existing = result
                .vertical_matches
                .iter_mut()
                .find(|m| (m.position - other_edge).abs() <= 1);

            if let Some(existing) = existing {
                if !existing.matched_ids.contains(&other_id) {
                    existing.matched_ids.push(other_id);
                    existing.line_start.1 = existing.line_start.1.min(moving.top).min(other.top);
                    existing.line_end.1 = existing.line_end.1.max(moving.bottom).max(other.bottom);
                }
            } else {
                result.vertical_matches.push(AlignmentMatch {
                    direction: AlignmentDirection::Vertical,
                    position: other_edge,
                    matched_ids: vec![other_id],
                    line_start: (other_edge, moving.top.min(other.top)),
                    line_end: (other_edge, moving.bottom.max(other.bottom)),
                });
            }
        }
    }
}

// ============================================================================
// 图标位图计算（从 icon_bitmap.rs 提取）
// ============================================================================

/// 返回非透明像素的边界范围 (left, top, width, height)
pub fn visible_pixel_bounds(
    pixels: &[Pixel32],
    width: usize,
    height: usize,
) -> Option<(usize, usize, usize, usize)> {
    let expected_len = width.checked_mul(height)?;
    if width == 0 || height == 0 || pixels.len() < expected_len {
        return None;
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;

    for (index, pixel) in pixels[..expected_len].iter().enumerate() {
        if pixel.a == 0 {
            continue;
        }

        let x = index % width;
        let y = index / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    if min_x == width {
        None
    } else {
        Some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
    }
}

/// 将位图尺寸适配到正方形区域内，保持宽高比并居中
pub fn icon_destination_rect(
    bitmap_width: u32,
    bitmap_height: u32,
    x: i32,
    y: i32,
    size: i32,
) -> (f32, f32, f32, f32) {
    if bitmap_width == 0 || bitmap_height == 0 || size <= 0 {
        return (x as f32, y as f32, x as f32, y as f32);
    }

    let scale = size as f32 / bitmap_width.max(bitmap_height) as f32;
    let draw_width = bitmap_width as f32 * scale;
    let draw_height = bitmap_height as f32 * scale;
    let left = x as f32 + (size as f32 - draw_width) / 2.0;
    let top = y as f32 + (size as f32 - draw_height) / 2.0;
    (left, top, left + draw_width, top + draw_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect::new(left, top, right, bottom)
    }

    #[test]
    fn test_no_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(1u64, make_rect(300, 300, 400, 400))];
        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
        assert!(result.vertical_matches.is_empty());
    }

    #[test]
    fn test_top_edge_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(1u64, make_rect(300, 102, 400, 300))];
        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        assert_eq!(result.horizontal_matches[0].position, 102);
    }

    #[test]
    fn test_left_edge_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(1u64, make_rect(100, 300, 300, 400))];
        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.vertical_matches.len(), 1);
        assert_eq!(result.vertical_matches[0].position, 100);
    }

    #[test]
    fn test_exact_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(1u64, make_rect(300, 100, 400, 200))];
        let result = detect_alignments(&moving, &others, 5);
        assert!(!result.horizontal_matches.is_empty());
    }

    #[test]
    fn test_multiple_alignments() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(1u64, make_rect(100, 100, 200, 200))];
        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 2);
        assert_eq!(result.vertical_matches.len(), 2);
    }

    #[test]
    fn test_empty_others() {
        let moving = make_rect(100, 100, 200, 200);
        let others: Vec<(u64, Rect)> = vec![];
        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
        assert!(result.vertical_matches.is_empty());
    }

    #[test]
    fn test_line_endpoints_horizontal() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![(1u64, make_rect(300, 100, 400, 300))];
        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        let m = &result.horizontal_matches[0];
        assert_eq!(m.line_start.0, 100);
        assert_eq!(m.line_start.1, 100);
        assert_eq!(m.line_end.0, 400);
        assert_eq!(m.line_end.1, 100);
    }

    fn pixel(alpha: u8) -> Pixel32 {
        Pixel32 { b: 0, g: 0, r: 0, a: alpha }
    }

    #[test]
    fn visible_pixel_bounds_removes_transparent_padding() {
        let mut pixels = vec![pixel(0); 8 * 6];
        for y in 1..5 {
            for x in 2..6 {
                pixels[y * 8 + x] = pixel(255);
            }
        }
        assert_eq!(visible_pixel_bounds(&pixels, 8, 6), Some((2, 1, 4, 4)));
    }

    #[test]
    fn visible_pixel_bounds_keeps_antialiased_edge_pixels() {
        let mut pixels = vec![pixel(0); 4 * 4];
        pixels[1 * 4 + 1] = pixel(1);
        pixels[2 * 4 + 2] = pixel(255);
        assert_eq!(visible_pixel_bounds(&pixels, 4, 4), Some((1, 1, 2, 2)));
    }

    #[test]
    fn visible_pixel_bounds_returns_none_for_empty_canvas() {
        let pixels = vec![pixel(0); 4 * 4];
        assert_eq!(visible_pixel_bounds(&pixels, 4, 4), None);
    }

    #[test]
    fn visible_pixel_bounds_keeps_full_canvas_icon() {
        let pixels = vec![pixel(255); 3 * 2];
        assert_eq!(visible_pixel_bounds(&pixels, 3, 2), Some((0, 0, 3, 2)));
    }

    #[test]
    fn visible_pixel_bounds_rejects_invalid_dimensions() {
        assert_eq!(visible_pixel_bounds(&[], 0, 4), None);
        assert_eq!(visible_pixel_bounds(&[pixel(255); 3], 2, 2), None);
    }

    #[test]
    fn icon_destination_rect_preserves_aspect_ratio_and_centers() {
        assert_eq!(
            icon_destination_rect(32, 16, 10, 20, 48),
            (10.0, 32.0, 58.0, 56.0)
        );
    }

    #[test]
    fn icon_destination_rect_centers_portrait_content() {
        assert_eq!(
            icon_destination_rect(16, 32, 10, 20, 48),
            (22.0, 20.0, 46.0, 68.0)
        );
    }

    #[test]
    fn icon_destination_rect_handles_invalid_size() {
        assert_eq!(
            icon_destination_rect(32, 32, 10, 20, 0),
            (10.0, 20.0, 10.0, 20.0)
        );
        assert_eq!(
            icon_destination_rect(0, 32, 10, 20, 48),
            (10.0, 20.0, 10.0, 20.0)
        );
    }
}
