//! 网格布局计算
//!
//! 根据窗口尺寸、视图参数和 DPI 计算网格布局。

use super::view_engine::ViewParams;

/// 网格布局计算结果
#[derive(Debug, Clone)]
pub struct GridLayout {
    /// 列数
    pub cols: i32,
    /// 网格区域左边距
    pub grid_left: i32,
    /// 网格区域上边距（标题栏下方）
    pub grid_top: i32,
    /// 单元格宽度
    pub cell_w: i32,
    /// 单元格高度
    pub cell_h: i32,
}

/// 计算网格布局
pub fn calculate_grid(
    window_width: i32,
    title_bar_height: i32,
    params: &ViewParams,
    _dpi: u32,
) -> GridLayout {
    let padding = 8; // 左右各 8px
    let body_w = window_width - padding * 2;

    let cols = if let Some(fixed) = params.fixed_cols {
        fixed
    } else {
        (body_w / params.cell_w).max(1)
    };

    let grid_left = (window_width - cols * params.cell_w) / 2;
    let grid_top = title_bar_height + 8;

    GridLayout {
        cols,
        grid_left,
        grid_top,
        cell_w: params.cell_w,
        cell_h: params.cell_h,
    }
}

/// 命中测试：根据点击位置返回项目索引
pub fn hit_test(
    layout: &GridLayout,
    item_count: usize,
    click_x: i32,
    click_y: i32,
) -> Option<usize> {
    if click_y < layout.grid_top || click_x < layout.grid_left {
        return None;
    }

    let col = (click_x - layout.grid_left) / layout.cell_w;
    let row = (click_y - layout.grid_top) / layout.cell_h;

    if col < 0 || col >= layout.cols {
        return None;
    }

    let idx = (row * layout.cols + col) as usize;
    if idx < item_count {
        Some(idx)
    } else {
        None
    }
}
