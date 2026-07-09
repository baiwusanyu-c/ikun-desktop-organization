//! 图标布局算法
//!
//! 实现桌面图标的网格布局

/// 图标尺寸常量
pub const ICON_SIZE: f64 = 64.0;
pub const ICON_SPACING_X: f64 = 32.0;
pub const ICON_SPACING_Y: f64 = 16.0;
pub const LABEL_HEIGHT: f64 = 32.0;
pub const PADDING: f64 = 20.0;

/// 计算图标在网格中的位置
pub struct GridLayout {
    pub cell_width: f64,
    pub cell_height: f64,
    pub cols: usize,
    pub rows: usize,
}

impl GridLayout {
    pub fn new(screen_width: f64, screen_height: f64) -> Self {
        let cell_width = ICON_SIZE + ICON_SPACING_X;
        let cell_height = ICON_SIZE + LABEL_HEIGHT + ICON_SPACING_Y;

        let available_width = screen_width - PADDING * 2.0;
        let cols = (available_width / cell_width).floor() as usize;
        let cols = cols.max(1);

        let available_height = screen_height - PADDING * 2.0;
        let rows = (available_height / cell_height).ceil() as usize;
        let rows = rows.max(1);

        GridLayout {
            cell_width,
            cell_height,
            cols,
            rows,
        }
    }

    /// 获取第 index 个图标的左上角位置
    pub fn get_position(&self, index: usize) -> (f64, f64) {
        let col = index % self.cols;
        let row = index / self.cols;

        let x = PADDING + (col as f64) * self.cell_width;
        let y = PADDING + (row as f64) * self.cell_height;

        (x, y)
    }

    /// 获取最大容量
    pub fn max_icons(&self) -> usize {
        self.cols * self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_layout() {
        let layout = GridLayout::new(1920.0, 1080.0);
        assert!(layout.cols > 0);
        assert!(layout.rows > 0);

        let (x, y) = layout.get_position(0);
        assert_eq!(x, PADDING);
        assert_eq!(y, PADDING);
    }
}
