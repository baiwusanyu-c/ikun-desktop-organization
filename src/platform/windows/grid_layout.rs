//! 分类框布局计算
//!
//! 提供网格布局、按钮矩形计算、命中测试等功能。
//! 所有坐标计算都考虑 DPI 缩放。

use crate::box_core::layout_calc::{calculate_grid, hit_test as layout_hit_test};
use crate::box_core::view_engine::get_view_params;

/// 在窗口宽度变化时重新映射图标的固定网格位置
///
/// 将每个图标的 (grid_col, grid_row) 按线性索引映射到新的列数下，
/// 保持视觉顺序和空位不变。
///
/// 例如 4 列时 `A, B, _, C`（C 在 col=3），变为 2 列后变成
/// `A, B` / `_, C`（C 仍然在第 2 列），而不是 `A, B` / `C, _`。
pub(crate) fn reflow_items(data: &mut super::grid_window::GridWindowData) {
    let new_cols = data.cols;
    // Extract positions, delegate to shared, write back
    let mut positions: Vec<(i32, i32)> = data
        .items
        .iter()
        .map(|item| (item.grid_col, item.grid_row))
        .collect();
    crate::box_core::layout::reflow_positions(&mut positions, new_cols);
    for (item, (col, row)) in data.items.iter_mut().zip(positions) {
        item.grid_col = col;
        item.grid_row = row;
    }
}

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
    // Account for scroll offset: convert screen click to virtual coordinate
    let click_y = click_y + data.scroll_offset;

    // First check positioned items (grid_col >= 0, grid_row >= 0)
    for (i, item) in data.items.iter().enumerate() {
        if item.grid_col >= 0 && item.grid_row >= 0 {
            let cell_x = layout.grid_left + item.grid_col * layout.cell_w;
            let cell_y = layout.grid_top + item.grid_row * layout.cell_h;
            if click_x >= cell_x
                && click_x < cell_x + layout.cell_w
                && click_y >= cell_y
                && click_y < cell_y + layout.cell_h
            {
                return i as i32;
            }
        }
    }

    // Fall back to index-based hit test for auto-layout items
    // Skip items that have per-item positions (already checked above)
    if let Some(i) = layout_hit_test(&layout, data.items.len(), click_x, click_y) {
        if i < data.items.len() && data.items[i].grid_col < 0 {
            return i as i32;
        }
    }
    -1
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app_config::{GroupBy, SortMode, ViewMode};
    use crate::platform::windows::grid_window::{GridFileItem, GridWindowData, MenuState};
    use std::ptr::null_mut;

    /// Helper to create a test GridFileItem
    fn test_item(name: &str, col: i32, row: i32) -> GridFileItem {
        GridFileItem {
            file_path: format!("{}.lnk", name),
            display_name: name.to_string(),
            icon_handle: null_mut(),
            grid_col: col,
            grid_row: row,
        }
    }

    /// Helper to create a minimal GridWindowData for testing
    fn test_data(cols: i32, items: Vec<GridFileItem>) -> GridWindowData {
        GridWindowData {
            title: "Test".to_string(),
            items,
            grid_id: 0,
            config_id: uuid::Uuid::new_v4(),
            title_h: 40,
            cell_w: 80,
            cell_h: 100,
            cols,
            icon_size: 48,
            selected_item: -1,
            hovered_item: -1,
            tooltip_hwnd: null_mut(),
            view_mode: ViewMode::Medium,
            locked: false,
            lock_hovered: false,
            hover_edge: 0,
            more_hovered: false,
            menu: MenuState::default(),
            edit_box_callback: None,
            sort_mode: SortMode::Name,
            sort_ascending: true,
            group_by: GroupBy::None,
            base_window_x: 0,
            base_window_y: 0,
            base_window_w: 400,
            base_window_h: 600,
            drag_pending: false,
            drag_start_x: 0,
            drag_start_y: 0,
            drag_active: false,
            drag_item_index: -1,
            drop_highlight: false,
            drag_over_item: -1,
            drag_over_insert_col: -1,
            drag_over_insert_row: -1,
            drop_target_ptr: null_mut(),
            scroll_offset: 0,
            shift_anchor: -1,
            marquee_active: false,
            marquee_start_x: 0,
            marquee_start_y: 0,
            marquee_current_x: 0,
            marquee_current_y: 0,
            marquee_prev_rect: [0; 4],
            marquee_pristine_valid: false,
            marquee_pristine: Vec::new(),
            marquee_last_render_ms: 0,
            marquee_force_full: false,
            multi_drag_items: Vec::new(),
            dcomp: None,
            dib_section: std::ptr::null_mut(),
            dib_bits: std::ptr::null_mut(),
            dib_dc: std::ptr::null_mut(),
            dib_old_bmp: std::ptr::null_mut(),
            dib_w: 0,
            dib_h: 0,
            content_dib: std::ptr::null_mut(),
            content_dc: std::ptr::null_mut(),
            content_old_bmp: std::ptr::null_mut(),
            content_bits: std::ptr::null_mut(),
            content_w: 0,
            content_h: 0,
            content_bitmap: None,
            content_fingerprint: 0,
            content_valid: false,
            render_padding: 0,
            glass_session_active: false,
            submitted_w: 0,
            submitted_h: 0,
            submitted_glass: false,
            content_last_render_ms: 0,
            box_drag_active: false,
            glass_session_seen_resizing: false,
            glass_session_idle_ticks: 0,
            box_drag_moved: false,
            hover_out_ticks: 0,
            d2d_target: None,
            icon_bitmap_cache: std::collections::HashMap::new(),
            renaming_item_index: -1,
        }
    }

    #[test]
    fn test_reflow_items_4cols_to_2cols_with_empty_space() {
        // Original: 4 cols, A(0,0), B(1,0), _(2,0), C(3,0)
        // Expected: 2 cols, A(0,0), B(1,0), _(0,1), C(1,1)
        let items = vec![
            test_item("A", 0, 0),
            test_item("B", 1, 0),
            test_item("C", 3, 0),
        ];
        let mut data = test_data(2, items); // Change to 2 cols

        reflow_items(&mut data);

        // A should be at (0, 0) - linear index 0 -> row 0, col 0
        assert_eq!(data.items[0].grid_col, 0);
        assert_eq!(data.items[0].grid_row, 0);

        // B should be at (1, 0) - linear index 1 -> row 0, col 1
        assert_eq!(data.items[1].grid_col, 1);
        assert_eq!(data.items[1].grid_row, 0);

        // C should be at (1, 1) - linear index 3 -> row 1, col 1
        // This preserves the empty space at (0, 1)
        assert_eq!(data.items[2].grid_col, 1);
        assert_eq!(data.items[2].grid_row, 1);
    }

    #[test]
    fn test_reflow_items_2cols_to_4cols() {
        // Original: 2 cols, A(0,0), B(1,0), C(0,1)
        // Expected: 4 cols, A(0,0), B(1,0), C(2,0)
        let items = vec![
            test_item("A", 0, 0),
            test_item("B", 1, 0),
            test_item("C", 0, 1),
        ];
        let mut data = test_data(4, items); // Change to 4 cols

        reflow_items(&mut data);

        // A should be at (0, 0) - linear index 0
        assert_eq!(data.items[0].grid_col, 0);
        assert_eq!(data.items[0].grid_row, 0);

        // B should be at (1, 0) - linear index 1
        assert_eq!(data.items[1].grid_col, 1);
        assert_eq!(data.items[1].grid_row, 0);

        // C should be at (2, 0) - linear index 2
        assert_eq!(data.items[2].grid_col, 2);
        assert_eq!(data.items[2].grid_row, 0);
    }

    #[test]
    fn test_reflow_items_preserves_auto_layout() {
        // Items with grid_col=-1 (auto-layout) should not be modified
        let items = vec![
            test_item("A", -1, -1),
            test_item("B", 1, 0),
            test_item("C", -1, -1),
        ];
        let mut data = test_data(3, items);

        reflow_items(&mut data);

        // Auto-layout items should remain unchanged
        assert_eq!(data.items[0].grid_col, -1);
        assert_eq!(data.items[0].grid_row, -1);
        assert_eq!(data.items[2].grid_col, -1);
        assert_eq!(data.items[2].grid_row, -1);

        // Fixed position item should be reflowed
        assert_eq!(data.items[1].grid_col, 1);
        assert_eq!(data.items[1].grid_row, 0);
    }

    #[test]
    fn test_reflow_items_empty_list() {
        let items: Vec<GridFileItem> = vec![];
        let mut data = test_data(3, items);

        reflow_items(&mut data);

        // Should not panic
        assert_eq!(data.items.len(), 0);
    }

    #[test]
    fn test_reflow_items_all_auto_layout() {
        let items = vec![test_item("A", -1, -1), test_item("B", -1, -1)];
        let mut data = test_data(3, items);

        reflow_items(&mut data);

        // All items should remain auto-layout
        assert_eq!(data.items[0].grid_col, -1);
        assert_eq!(data.items[1].grid_col, -1);
    }

    #[test]
    fn test_reflow_items_invalid_cols() {
        let items = vec![test_item("A", 0, 0)];
        let mut data = test_data(0, items); // Invalid cols

        reflow_items(&mut data);

        // Should not panic and should not modify items
        assert_eq!(data.items[0].grid_col, 0);
        assert_eq!(data.items[0].grid_row, 0);
    }
}
