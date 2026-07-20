//! �������㷨
//!
//! ����ƶ�/�����еķ���������������֮��ı�Ե�����ϵ��
//! ֧��ˮƽ�ʹ�ֱ��������Ķ����⣬���ṩ�������ܡ�

use crate::platform::windows::ffi_types::RECT;

/// ���뷽��
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentDirection {
    /// ˮƽ���루top/bottom ��Ե���룩
    Horizontal,
    /// ��ֱ���루left/right ��Ե���룩
    Vertical,
}

/// ����ƥ����
#[derive(Debug, Clone)]
pub struct AlignmentMatch {
    /// ���뷽��
    pub direction: AlignmentDirection,
    /// ���������ֵ����Ļ�������أ�
    pub position: i32,
    /// ƥ��Ĳο��� HWND �б�
    pub matched_hwnds: Vec<*mut std::ffi::c_void>,
    /// 参考线起点（屏幕坐标）
    pub line_start: (i32, i32),
    /// 参考线终点（屏幕坐标）
    pub line_end: (i32, i32),
}

/// ��������
pub struct AlignmentResult {
    /// ˮƽ����ƥ�䣨ˮƽ�ο��ߣ�
    pub horizontal_matches: Vec<AlignmentMatch>,
    /// ��ֱ����ƥ�䣨��ֱ�ο��ߣ�
    pub vertical_matches: Vec<AlignmentMatch>,
    /// �����������λ�ã������������
    pub snapped_rect: Option<RECT>,
}

unsafe impl Send for AlignmentMatch {}
unsafe impl Send for AlignmentResult {}


impl Default for AlignmentResult {
    fn default() -> Self {
        Self {
            horizontal_matches: Vec::new(),
            vertical_matches: Vec::new(),
            snapped_rect: None,
        }
    }
}



/// Ĭ�϶�����ֵ���������أ�
pub const DEFAULT_ALIGNMENT_THRESHOLD: i32 = 5;

/// ����ƶ�����������֮��Ķ����ϵ
///
/// # ����
/// - `moving_rect`: �����ƶ��Ŀ����Ļ RECT
/// - `others`: ������� (HWND, RECT) �б�
/// - `threshold`: ������ֵ���������أ�
///
/// # ����
/// ������������������ƥ��Ķ����ϵ�ͽ��������λ��
pub fn detect_alignments(
    moving_rect: &RECT,
    others: &[(*mut std::ffi::c_void, RECT)],
    threshold: i32,
) -> AlignmentResult {
    let mut result = AlignmentResult::default();
    
    // �ռ����п��ܶ���ı�Ե
    let moving_edges = RectEdges::from_rect(moving_rect);
    
    for (hwnd, other_rect) in others {
        let other_edges = RectEdges::from_rect(other_rect);
        
        // ���ˮƽ���루top/bottom ��Ե��
        check_horizontal_alignment(&mut result, &moving_edges, &other_edges, *hwnd, threshold);
        
        // ��ⴹֱ���루left/right ��Ե��
        check_vertical_alignment(&mut result, &moving_edges, &other_edges, *hwnd, threshold);
    }
    
    // �����������λ��
    result.snapped_rect = calculate_snapped_rect(moving_rect, &result);
    
    result
}

/// ���α�Ե����
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

/// ���ˮƽ���루top/bottom ��Ե��
fn check_horizontal_alignment(
    result: &mut AlignmentResult,
    moving: &RectEdges,
    other: &RectEdges,
    other_hwnd: *mut std::ffi::c_void,
    threshold: i32,
) {
    // ������� top/bottom ���
    let pairs = [
        (moving.top, other.top),
        (moving.top, other.bottom),
        (moving.bottom, other.top),
        (moving.bottom, other.bottom),
    ];
    
    for (moving_edge, other_edge) in pairs {
        let diff = (moving_edge - other_edge).abs();
        if diff <= threshold {
            // �����Ƿ�������ͬλ�õĶ���
            let existing = result.horizontal_matches.iter_mut().find(|m| {
                (m.position - other_edge).abs() <= 1
            });
            
            if let Some(existing) = existing {
                if !existing.matched_hwnds.contains(&other_hwnd) {
                    existing.matched_hwnds.push(other_hwnd);
                }
            } else {
                // 计算线段端点：水平线从左框的左边缘到右框的右边缘
                let left_x = moving.left.min(other.left);
                let right_x = moving.right.max(other.right);
                let line_y = other_edge;
                result.horizontal_matches.push(AlignmentMatch {
                    direction: AlignmentDirection::Horizontal,
                    position: other_edge,
                    matched_hwnds: vec![other_hwnd],
                    line_start: (left_x, line_y),
                    line_end: (right_x, line_y),
                });
            }
        }
    }
}

/// ��鴹ֱ���루left/right ��Ե��
fn check_vertical_alignment(
    result: &mut AlignmentResult,
    moving: &RectEdges,
    other: &RectEdges,
    other_hwnd: *mut std::ffi::c_void,
    threshold: i32,
) {
    // ������� left/right ���
    let pairs = [
        (moving.left, other.left),
        (moving.left, other.right),
        (moving.right, other.left),
        (moving.right, other.right),
    ];
    
    for (moving_edge, other_edge) in pairs {
        let diff = (moving_edge - other_edge).abs();
        if diff <= threshold {
            // �����Ƿ�������ͬλ�õĶ���
            let existing = result.vertical_matches.iter_mut().find(|m| {
                (m.position - other_edge).abs() <= 1
            });
            
            if let Some(existing) = existing {
                if !existing.matched_hwnds.contains(&other_hwnd) {
                    existing.matched_hwnds.push(other_hwnd);
                }
            } else {
                // 计算线段端点：垂直线从上框的上边缘到下框的下边缘
                let top_y = moving.top.min(other.top);
                let bottom_y = moving.bottom.max(other.bottom);
                let line_x = other_edge;
                result.vertical_matches.push(AlignmentMatch {
                    direction: AlignmentDirection::Vertical,
                    position: other_edge,
                    matched_hwnds: vec![other_hwnd],
                    line_start: (line_x, top_y),
                    line_end: (line_x, bottom_y),
                });
            }
        }
    }
}

/// ����������ľ���λ��
fn calculate_snapped_rect(original: &RECT, result: &AlignmentResult) -> Option<RECT> {
    let mut snapped = RECT { left: original.left, top: original.top, right: original.right, bottom: original.bottom };
    let mut changed = false;
    
    // Ӧ��ˮƽ���������� top/bottom��
    if !result.horizontal_matches.is_empty() {
        // �ҵ������ˮƽ����
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
        
        // �������� top����� top ����С
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
    
    // Ӧ�ô�ֱ���������� left/right��
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
        
        // �������� left
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
    
    if changed { Some(snapped) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn test_no_alignment() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![
            (std::ptr::null_mut(), make_rect(300, 300, 400, 400)),
        ];
        
        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
        assert!(result.vertical_matches.is_empty());
    }

    #[test]
    fn test_top_alignment() {
        // moving: top=100, bottom=200
        // other:  top=100, bottom=300 (different bottom, so only top aligns)
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![
            (std::ptr::null_mut(), make_rect(300, 100, 400, 300)),
        ];
        
        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        assert_eq!(result.horizontal_matches[0].position, 100);
    }

    #[test]
    fn test_left_alignment() {
        // moving: left=100, right=200
        // other:  left=100, right=300 (different right, so only left aligns)
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![
            (std::ptr::null_mut(), make_rect(100, 300, 300, 400)),
        ];
        
        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.vertical_matches.len(), 1);
        assert_eq!(result.vertical_matches[0].position, 100);
    }

    #[test]
    fn test_snap_threshold() {
        let moving = make_rect(100, 97, 200, 197);  // top �� 3px
        let others = vec![
            (std::ptr::null_mut(), make_rect(300, 100, 400, 300)),
        ];
        
        let result = detect_alignments(&moving, &others, 5);
        assert_eq!(result.horizontal_matches.len(), 1);
        assert!(result.snapped_rect.is_some());
        assert_eq!(result.snapped_rect.unwrap().top, 100);
    }

    #[test]
    fn test_outside_threshold() {
        let moving = make_rect(100, 94, 200, 194);  // top �� 6px
        let others = vec![
            (std::ptr::null_mut(), make_rect(300, 100, 400, 200)),
        ];
        
        let result = detect_alignments(&moving, &others, 5);
        assert!(result.horizontal_matches.is_empty());
    }

    #[test]
    fn test_multiple_alignments() {
        let moving = make_rect(100, 100, 200, 200);
        let others = vec![
            (std::ptr::null_mut(), make_rect(100, 100, 200, 200)),  // ��ȫ�ص�
        ];
        
        let result = detect_alignments(&moving, &others, 5);
        // top �� bottom ������
        assert_eq!(result.horizontal_matches.len(), 2);
        // left �� right ������
        assert_eq!(result.vertical_matches.len(), 2);
    }
}

