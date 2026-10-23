//! 拖拽状态常量和工具函数
//!
//! 从 platform/windows 提取的平台无关拖拽常量和判断逻辑。
//! 分类框与桌面覆盖层共用的纯逻辑，无平台依赖。

/// 拖拽触发阈值（像素）
/// 鼠标移动距离超过此值后才视为拖拽开始，防止误触。
pub const DRAG_THRESHOLD: i32 = 5;

/// 判断鼠标移动距离是否超过拖拽阈值
pub fn is_drag_threshold_exceeded(dx: i32, dy: i32) -> bool {
    dx.abs() > DRAG_THRESHOLD || dy.abs() > DRAG_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_exceeded() {
        assert!(!is_drag_threshold_exceeded(3, 2));
        assert!(!is_drag_threshold_exceeded(5, 5));
        assert!(!is_drag_threshold_exceeded(0, 0));
    }

    #[test]
    fn test_exceeded() {
        assert!(is_drag_threshold_exceeded(6, 0));
        assert!(is_drag_threshold_exceeded(0, 6));
        assert!(is_drag_threshold_exceeded(20, 15));
    }

    #[test]
    fn test_negative_delta() {
        assert!(is_drag_threshold_exceeded(-10, 0));
        assert!(!is_drag_threshold_exceeded(-3, -4));
    }
}
