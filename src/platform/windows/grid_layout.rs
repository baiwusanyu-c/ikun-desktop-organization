//! 分类框布局计算
//!
//! 提供网格布局、按钮矩形计算、命中测试等功能。
//! 所有坐标计算都考虑 DPI 缩放。

use crate::box_core::layout::{content_rows, scroll_limit};
use crate::box_core::layout_calc::{
    calculate_grid, grid_side_pad, hit_test as layout_hit_test, snap_width_for_cols,
};
use crate::box_core::view_engine::get_view_params;

/// 用**显式**旧列数把固定位重排到当前 `data.cols`
///
/// `old_cols <= 0` 时由纯函数退回「max(col)+1」推断（仅供历史数据兜底）。
pub(crate) fn reflow_to(data: &mut super::grid_window::GridWindowData, old_cols: i32) {
    let new_cols = data.cols;
    // Extract positions, delegate to shared, write back
    let mut positions: Vec<(i32, i32)> = data
        .items
        .iter()
        .map(|item| (item.grid_col, item.grid_row))
        .collect();
    crate::box_core::layout::reflow_positions(&mut positions, new_cols, old_cols);
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
    // 记下本轮真实列数，供下一次重排当作「旧列数」（跨屏/缩放时判据来源）。
    data.layout_cols = grid_layout.cols;
    data.cell_w = grid_layout.cell_w;
    data.cell_h = grid_layout.cell_h;
}

/// 尺寸变化 -> 重算网格 + 重排固定位（需求 2.1 的唯一入口）
///
/// **先重算后重排**，旧列数在重算前抄走（重算会把 `layout_cols` 刷成新值）。
/// 返回列数是否变化：变化即意味着条目已自动换行/换上行。
pub(crate) fn relayout_for_size(
    data: &mut super::grid_window::GridWindowData,
    width: i32,
    height: i32,
    dpi: u32,
) -> bool {
    let prev_cols = data.layout_cols;
    recalculate_layout(data, width, height, dpi);
    if prev_cols > 0 && prev_cols != data.cols {
        reflow_to(data, prev_cols);
        return true;
    }
    false
}

/// 跨屏 DPI 重采样 -> 先吸附宽度保住上一轮列数，再重算重排（需求 1）
///
/// 返回实际采用的内容宽度，调用方必须按它摆窗口。列数只在「取整噪声」级别被修正，
/// 用户真正把框缩小到装不下时仍会放行给自动换行（见 `snap_width_for_cols`）。
pub(crate) fn relayout_for_dpi(
    data: &mut super::grid_window::GridWindowData,
    width: i32,
    height: i32,
    dpi: u32,
) -> i32 {
    let prev_cols = data.layout_cols;
    let view_params = get_view_params(&data.view_mode, dpi);
    let pad = grid_side_pad(dpi);
    let snapped = snap_width_for_cols(width, prev_cols, view_params.cell_w, pad, (pad * 2).max(2));
    recalculate_layout(data, snapped, height, dpi);
    if prev_cols > 0 && prev_cols != data.cols {
        reflow_to(data, prev_cols);
    }
    snapped
}

/// 滚动口径的唯一来源：返回 (内容总行数, max_scroll)
///
/// 行数按条目**实际占据的行**算（固定位可能把行号撑得比 `len/cols` 更深），
/// 可见高以 `grid_top` 起算 —— 与 `d2d_render::render_icon_grid` 画滚动条
/// 完全同式。滚轮、WM_SIZE 收敛、滚动条绘制三处都必须走这里，否则会
/// 「滚得动却没滚动条」或「有滚动条却滚不到底」。
pub(crate) fn scroll_metrics(
    data: &super::grid_window::GridWindowData,
    win_w: i32,
    win_h: i32,
    dpi: u32,
) -> (i32, i32) {
    let view_params = get_view_params(&data.view_mode, dpi);
    let grid = calculate_grid(win_w, data.title_h, &view_params, dpi);
    let rows = content_rows(&data.items, grid.cols, |_, item| {
        (item.grid_col, item.grid_row)
    });
    let visible_h = (win_h - grid.grid_top).max(0);
    (rows, scroll_limit(rows, grid.cell_h, visible_h))
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
            layout_cols: cols,
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
            drag_box_rect: [0, 0, 0, 0],
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
            glass_session_seen_resizing: false,
            glass_session_idle_ticks: 0,
            hover_out_ticks: 0,
            cross_dpi_fix_ms: 0,
            glass_diag_at: 0,
            glass_session_ended_at: 0,
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
                                            // test_data 把 layout_cols 设成了本轮新列数（2），这里必须还原「上一轮真实列数 4」
        data.layout_cols = 4;

        let old_cols = data.layout_cols;
        reflow_to(&mut data, old_cols);

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
        // 本轮新列数 4、上一轮真实列数 2（必须由 layout_cols 提供，
        // 而不是从「已有项的最大列 + 1」猜——有空洞时那个值必然偏小）。
        let mut data = test_data(4, items);
        data.layout_cols = 2;

        let old_cols = data.layout_cols;
        reflow_to(&mut data, old_cols);

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
    fn test_reflow_items_hole_grid_uses_layout_cols_not_inference() {
        // 真实 6 列、只用过 col 0..3（行内有空洞）：推断值只有 4，
        // 用它当「旧列数」会把项挪到错误的行列（= 用户看到的「图标位置变乱」）。
        let items = vec![
            test_item("A", 0, 0),
            test_item("B", 3, 0),
            test_item("C", 1, 1),
        ];
        let mut data = test_data(3, items);
        data.layout_cols = 6;
        let old_cols = data.layout_cols;
        reflow_to(&mut data, old_cols);
        // 线性位：0 -> (0,0)；3 -> (0,1)；7 -> (1,2)
        assert_eq!((data.items[0].grid_col, data.items[0].grid_row), (0, 0));
        assert_eq!((data.items[1].grid_col, data.items[1].grid_row), (0, 1));
        assert_eq!((data.items[2].grid_col, data.items[2].grid_row), (1, 2));

        // 对照：不给 layout_cols（退回推断=4）时结果确实不同 —— 钉住本修复的必要性。
        let mut blind = test_data(
            3,
            vec![
                test_item("A", 0, 0),
                test_item("B", 3, 0),
                test_item("C", 1, 1),
            ],
        );
        blind.layout_cols = 0;
        let old_cols = blind.layout_cols;
        reflow_to(&mut blind, old_cols);
        assert_ne!(
            (blind.items[2].grid_col, blind.items[2].grid_row),
            (1, 2),
            "推断口径与真实口径必须给出不同结果"
        );
    }

    #[test]
    fn test_reflow_items_uses_layout_cols_across_dpi_change() {
        // 6 列真实布局：A(0,0) B(3,0) C(5,1) —— 行内有空洞。
        // 跨屏后新算出 4 列：必须按「旧列数 6」重映射，而不是按 max(col)+1=6 猜；
        // 这里同时钉住 layout_cols 参与判据（回归「主副屏拖拽后图标变乱」）。
        let items = vec![
            test_item("A", 0, 0),
            test_item("B", 3, 0),
            test_item("C", 5, 1),
        ];
        let mut data = test_data(4, items);
        data.layout_cols = 6;
        let old_cols = data.layout_cols;
        reflow_to(&mut data, old_cols);
        // 真实 6 列 → 新 4 列：(3,0) 线性 3 -> (3,0)；(5,1) 线性 11 -> (3,2)
        assert_eq!((data.items[0].grid_col, data.items[0].grid_row), (0, 0));
        assert_eq!((data.items[1].grid_col, data.items[1].grid_row), (3, 0));
        assert_eq!((data.items[2].grid_col, data.items[2].grid_row), (3, 2));
    }

    #[test]
    fn test_reflow_items_noop_when_cols_unchanged() {
        // 跨屏 DPI 重采样后列数往往没变，此时任何重映射都是净伤害（不可逆打乱布局）。
        let items = vec![
            test_item("A", 0, 0),
            test_item("B", 3, 2),
            test_item("C", 1, 1),
        ];
        let mut data = test_data(4, items);
        data.layout_cols = 4;
        let old_cols = data.layout_cols;
        reflow_to(&mut data, old_cols);
        assert_eq!((data.items[1].grid_col, data.items[1].grid_row), (3, 2));
        assert_eq!((data.items[2].grid_col, data.items[2].grid_row), (1, 1));
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

        let old_cols = data.layout_cols;
        reflow_to(&mut data, old_cols);

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

        let old_cols = data.layout_cols;
        reflow_to(&mut data, old_cols);

        // Should not panic
        assert_eq!(data.items.len(), 0);
    }

    #[test]
    fn test_reflow_items_all_auto_layout() {
        let items = vec![test_item("A", -1, -1), test_item("B", -1, -1)];
        let mut data = test_data(3, items);

        let old_cols = data.layout_cols;
        reflow_to(&mut data, old_cols);

        // All items should remain auto-layout
        assert_eq!(data.items[0].grid_col, -1);
        assert_eq!(data.items[1].grid_col, -1);
    }

    #[test]
    fn test_reflow_items_invalid_cols() {
        let items = vec![test_item("A", 0, 0)];
        let mut data = test_data(0, items); // Invalid cols

        let old_cols = data.layout_cols;
        reflow_to(&mut data, old_cols);

        // Should not panic and should not modify items
        assert_eq!(data.items[0].grid_col, 0);
        assert_eq!(data.items[0].grid_row, 0);
    }

    /// 需求 2.1：缩小宽度 -> 列数变少 -> 固定位自动换行且必须落在框内；
    /// 放大宽度 -> 列数变多 -> 自动换回上一行。
    ///
    /// 顺序教训：这两步以前在 wnd_proc 里顺序颠倒（reflow 先于 recalculate），
    /// 于是 `cols == layout_cols` 恒成立、重排永远早退，缩小后图标直接出框。
    #[test]
    fn relayout_for_size_wraps_fixed_items_into_narrower_box() {
        let items = vec![
            test_item("A", 0, 0),
            test_item("B", 1, 0),
            test_item("C", 2, 0),
            test_item("D", 3, 0),
        ];
        let mut data = test_data(4, items);
        data.title_h = scale_for_dpi(40, 96);

        // 放大：Medium 在 96 DPI 下 cell_w = 80；宽 816 -> body 800 -> 10 列
        assert!(
            relayout_for_size(&mut data, 816, 600, 96),
            "列数 4 -> 10 必须触发重排"
        );
        assert_eq!(data.cols, 10);
        let up: Vec<(i32, i32)> = data
            .items
            .iter()
            .map(|i| (i.grid_col, i.grid_row))
            .collect();
        assert_eq!(
            up,
            vec![(0, 0), (1, 0), (2, 0), (3, 0)],
            "列数变多时四个图标应并排回到第一行"
        );

        // 缩小：宽 208 -> body 192 -> 2 列
        assert!(
            relayout_for_size(&mut data, 208, 600, 96),
            "列数 10 -> 2 必须触发重排"
        );
        assert_eq!(data.cols, 2);
        let down: Vec<(i32, i32)> = data
            .items
            .iter()
            .map(|i| (i.grid_col, i.grid_row))
            .collect();
        assert_eq!(down, vec![(0, 0), (1, 0), (0, 1), (1, 1)], "缩小必须换行");
        for (col, _) in &down {
            assert!(*col < data.cols, "固定位越界 = 图标画到框外: col={col}");
        }

        // 尺寸没变 -> 列数没变 -> 一律不重排（幂等，保护用户手工布局）
        let before = down;
        assert!(!relayout_for_size(&mut data, 208, 600, 96));
        let after: Vec<(i32, i32)> = data
            .items
            .iter()
            .map(|i| (i.grid_col, i.grid_row))
            .collect();
        assert_eq!(before, after, "列数不变时不得动任何位置");
    }

    /// 需求 1：主屏(192 DPI) 与副屏(96 DPI) 之间切换，列数与图标位置必须保持不变。
    #[test]
    fn relayout_for_dpi_keeps_cols_across_monitors() {
        let items = vec![
            test_item("A", 0, 0),
            test_item("B", 2, 1),
            test_item("C", 5, 3),
        ];
        let mut data = test_data(6, items);
        data.title_h = scale_for_dpi(40, 96);
        // 先在 192 DPI 上按真实宽度定列：cell_w=160、pad=16 -> 738 宽 = 4 列
        let w192 = 738;
        relayout_for_size(&mut data, w192, 600, 192);
        let cols192 = data.cols;
        assert_eq!(cols192, 4, "738 @ (cell 160, pad 16) 应为 4 列");
        let pos192: Vec<(i32, i32)> = data
            .items
            .iter()
            .map(|i| (i.grid_col, i.grid_row))
            .collect();

        // 拖到 96 DPI 副屏：物理宽同比折半
        let w96 = relayout_for_dpi(&mut data, w192 / 2, 300, 96);
        assert_eq!(data.cols, cols192, "跨屏 DPI 重采样不得改变列数");
        assert_eq!(w96, w192 / 2, "宽度落在合法区间内时不该被吸附改动");
        let pos96: Vec<(i32, i32)> = data
            .items
            .iter()
            .map(|i| (i.grid_col, i.grid_row))
            .collect();
        assert_eq!(pos192, pos96, "列数没变 -> 图标位置必须一模一样");

        // 取整噪声（比 4 列下界小 1px）要吸附回去，而不是少算一列把图标挤乱
        let just_below = 4 * scale_for_dpi(80, 96) + 2 * scale_for_dpi(8, 96) - 1;
        let snapped = relayout_for_dpi(&mut data, just_below, 300, 96);
        assert_eq!(data.cols, 4, "1px 取整误差不该让列数从 4 变成 3");
        assert_eq!(snapped, just_below + 1);

        // 用户真的把框缩到只够 2 列：必须放行，交给自动换行（不能被吸附顶回去）
        let two = 2 * scale_for_dpi(80, 96) + 2 * scale_for_dpi(8, 96);
        relayout_for_dpi(&mut data, two, 300, 96);
        assert_eq!(data.cols, 2);
        for (col, _) in data.items.iter().map(|i| (i.grid_col, i.grid_row)) {
            assert!(col < 2, "真缩小后仍要保持在框内: col={col}");
        }
    }

    /// 需求 2.2：滚动上限按「条目实际占据的行」算，且与渲染端可见高同口径。
    #[test]
    fn scroll_metrics_counts_deep_fixed_rows() {
        let items = vec![test_item("A", 0, 0), test_item("B", 0, 11)];
        let mut data = test_data(4, items);
        data.title_h = scale_for_dpi(40, 96);

        let (rows, max_scroll) = scroll_metrics(&data, 816, 600, 96);
        assert_eq!(rows, 12, "最后一个固定位在第 12 行 -> 内容 12 行");
        // cell_h=100 -> 内容高 1200；可见高 = 600 - (40+8) = 552 -> 上限 648
        assert_eq!(max_scroll, 648);

        // 框够高时没有滚动，也不该出现滚动条（渲染端用同一个 scroll_limit）
        let (_r2, fit) = scroll_metrics(&data, 816, 2000, 96);
        assert_eq!(fit, 0, "装得下就不该滚");

        // 纯自动位：行数按当前列数线性排布
        let auto: Vec<GridFileItem> = (0..7)
            .map(|i| test_item(&format!("F{i}"), -1, -1))
            .collect();
        let mut d2 = test_data(3, auto);
        d2.title_h = scale_for_dpi(40, 96);
        let (rows3, _) = scroll_metrics(&d2, 336, 600, 96); // body 320 / 80 = 4 列 -> 2 行
        assert_eq!(rows3, 2);
    }
}
