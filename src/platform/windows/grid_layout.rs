//! 分类框布局计算
//!
//! 提供网格布局、按钮矩形计算、命中测试等功能。
//! 所有坐标计算都考虑 DPI 缩放。

use crate::box_core::layout_calc::{calculate_grid, hit_test as layout_hit_test};
use crate::box_core::view_engine::get_view_params;

/// 重新计算网格布局
///
/// 根据窗口宽度、高度和 DPI 重新计算网格的列数、单元格尺寸等。
pub(crate) fn recalculate_layout(
    data: &mut super::grid_window::GridWindowData,
    width: i32,
    _height: i32,
    dpi: u32,
) {
    data.title_h = scale_for_dpi(40, dpi);

    // 通过 view_engine 获取当前视图模式的参数（支持 ExtraLarge/Large/Medium/Small 等）
    let view_params = get_view_params(&data.view_mode, dpi);
    data.icon_size = view_params.icon_size;

    // 通过 layout_calc 计算网格（自动 clamp：窄窗口时 cell_w 缩小到 body_w，保证图标不超出可视区域）
    let grid_layout = calculate_grid(width, data.title_h, &view_params, dpi);
    data.cols = grid_layout.cols;
    data.cell_w = grid_layout.cell_w;
    data.cell_h = grid_layout.cell_h;
}

/// 命中测试：根据鼠标位置获取网格项索引
///
/// 委托给 layout_calc::hit_test，保证与渲染使用完全相同的布局计算公式，
/// 消除 hover/选中/双击/右键的目标偏移问题。
pub(crate) fn grid_hit_test(
    data: &super::grid_window::GridWindowData,
    window_width: i32,
    click_x: i32,
    click_y: i32,
    dpi: u32,
) -> i32 {
    let view_params = get_view_params(&data.view_mode, dpi);
    let layout = calculate_grid(window_width, data.title_h, &view_params, dpi);

    layout_hit_test(&layout, data.items.len(), click_x, click_y)
        .map(|i| i as i32)
        .unwrap_or(-1)
}

/// 计算锁定按钮在窗口坐标中的矩形区域
///
/// 位于标题栏右侧（"更多"按钮左边），用于渲染和命中测试。
/// 返回 (x, y, btn_size) 其中 btn_size 为按钮边长。
pub(super) fn lock_btn_rect(win_w: i32, title_h: i32, dpi: u32) -> (i32, i32, i32) {
    let btn_size = scale_for_dpi(26, dpi);
    // 锁定按钮在"更多"按钮左侧
    let (_, _, more_size) = more_btn_rect(win_w, title_h, dpi);
    let btn_x = win_w - more_size - btn_size - scale_for_dpi(10, dpi);
    let btn_y = (title_h - btn_size) / 2;
    (btn_x, btn_y, btn_size)
}

/// 计算"更多"按钮在窗口坐标中的矩形区域
///
/// 位于锁定按钮右侧，用于渲染和命中测试。
/// 返回 (x, y, btn_size) 其中 btn_size 为按钮边长。
pub(super) fn more_btn_rect(win_w: i32, title_h: i32, dpi: u32) -> (i32, i32, i32) {
    let btn_size = scale_for_dpi(26, dpi);
    // "更多"按钮在最右侧
    let btn_x = win_w - btn_size - scale_for_dpi(6, dpi);
    let btn_y = (title_h - btn_size) / 2;
    (btn_x, btn_y, btn_size)
}

/// 计算名称编辑模式下的输入框矩形
///
/// 位于标题栏左侧，为右侧的清除/保存按钮预留空间。
/// 返回 (x, y, w, h)。
pub(super) fn input_box_rect(win_w: i32, title_h: i32, dpi: u32) -> (i32, i32, i32, i32) {
    let pad = scale_for_dpi(8, dpi);
    let box_h = scale_for_dpi(28, dpi);
    let box_y = (title_h - box_h) / 2;
    let gap = scale_for_dpi(4, dpi);
    let (_, _, save_w, _) = save_btn_rect(win_w, title_h, dpi);
    let (_, _, clear_w, _) = clear_btn_rect(win_w, title_h, dpi);
    // 右侧为间隙 + 清除按钮 + 间隙 + 保存按钮 预留空间
    let right_reserve = gap + clear_w + gap + save_w + scale_for_dpi(6, dpi);
    let box_w = (win_w - pad - right_reserve).max(scale_for_dpi(80, dpi));
    (pad, box_y, box_w, box_h)
}

/// 计算清除按钮在窗口坐标中的矩形区域
///
/// 位于保存按钮左侧，与输入框/保存按钮水平对齐（同高，统一间距）。
/// 返回 (x, y, w, h)。
pub(super) fn clear_btn_rect(win_w: i32, title_h: i32, dpi: u32) -> (i32, i32, i32, i32) {
    let btn_w = scale_for_dpi(54, dpi);
    let btn_h = scale_for_dpi(28, dpi);
    let btn_y = (title_h - btn_h) / 2;
    let (_, _, save_w, _) = save_btn_rect(win_w, title_h, dpi);
    let gap = scale_for_dpi(4, dpi);
    let btn_x = win_w - save_w - scale_for_dpi(6, dpi) - gap - btn_w;
    (btn_x, btn_y, btn_w, btn_h)
}

/// 计算保存按钮在窗口坐标中的矩形区域
///
/// 位于清除按钮右侧，显示 "保存" 文字。
/// 返回 (x, y, w, h)。
pub(super) fn save_btn_rect(win_w: i32, title_h: i32, dpi: u32) -> (i32, i32, i32, i32) {
    let btn_w = scale_for_dpi(54, dpi);
    let btn_h = scale_for_dpi(28, dpi);
    let btn_x = win_w - btn_w - scale_for_dpi(6, dpi);
    let btn_y = (title_h - btn_h) / 2;
    (btn_x, btn_y, btn_w, btn_h)
}

#[allow(dead_code)] // 菜单渲染时使用
pub(super) fn menu_row_h(dpi: u32) -> i32 {
    scale_for_dpi(36, dpi)
}

#[allow(dead_code)] // 菜单渲染时使用
pub(super) fn menu_width(dpi: u32) -> i32 {
    scale_for_dpi(160, dpi)
}

#[allow(dead_code)] // 菜单渲染时使用
pub(super) fn submenu_width(dpi: u32) -> i32 {
    scale_for_dpi(160, dpi)
}

#[allow(dead_code)] // 菜单渲染时使用
pub(super) fn menu_separator_h(dpi: u32) -> i32 {
    scale_for_dpi(1, dpi)
}

#[allow(dead_code)] // 菜单渲染时使用
pub(super) fn main_menu_rect(win_w: i32, title_h: i32, dpi: u32) -> (i32, i32, i32, i32) {
    let mw = menu_width(dpi);
    let mx = win_w - mw - scale_for_dpi(6, dpi);
    let my = title_h + scale_for_dpi(2, dpi);
    // 4 项 + 1 分隔线 + padding (top 4 + bottom 12)
    let mh = menu_row_h(dpi) * 4 + menu_separator_h(dpi) + scale_for_dpi(16, dpi);
    (mx, my, mw, mh)
}

#[allow(dead_code)] // 菜单渲染时使用
/// 子菜单在窗口坐标中的矩形
///
/// on_left 为 true 时显示在主菜单左侧，否则显示在右侧。
/// 返回 (x, y, w, h)
pub(super) fn sub_menu_rect(
    win_w: i32,
    title_h: i32,
    dpi: u32,
    sub_item_count: i32,
    on_left: bool,
) -> (i32, i32, i32, i32) {
    let (mx, my, mw, _mh) = main_menu_rect(win_w, title_h, dpi);
    let sw = submenu_width(dpi);
    let sx = if on_left {
        mx - sw - scale_for_dpi(2, dpi)
    } else {
        mx + mw + scale_for_dpi(2, dpi)
    };
    let sy = my;
    // padding top(4) + items + padding bottom(12) 确保圆角不裁剪
    let sh = menu_row_h(dpi) * sub_item_count + scale_for_dpi(16, dpi);
    (sx, sy, sw, sh)
}

#[allow(dead_code)] // 菜单渲染时使用
pub(super) fn menu_extra_height(dpi: u32) -> i32 {
    // 4 主项 + 1 分隔线 + padding + 额外空间
    menu_row_h(dpi) * 4 + menu_separator_h(dpi) + scale_for_dpi(32, dpi)
}

/// DPI 缩放辅助函数
fn scale_for_dpi(px: i32, dpi: u32) -> i32 {
    (px * dpi as i32) / 96
}
