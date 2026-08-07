//! 跨平台共享类型定义
//!
//! 平台无关的基础类型，供 shared/ 各子模块和 platform/ 各平台实现共用。

/// 跨平台矩形（对应 Windows RECT / macOS CGRect 的逻辑抽象）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// 跨平台坐标点
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// 32-bit RGBA 像素（对应 Windows Pixel32 / macOS RGBA）
#[derive(Debug, Clone, Copy, Default)]
pub struct Pixel32 {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_dimensions() {
        let r = Rect::new(10, 20, 110, 120);
        assert_eq!(r.width(), 100);
        assert_eq!(r.height(), 100);
    }

    #[test]
    fn test_rect_default() {
        let r = Rect::default();
        assert_eq!(r.left, 0);
        assert_eq!(r.width(), 0);
    }
}
