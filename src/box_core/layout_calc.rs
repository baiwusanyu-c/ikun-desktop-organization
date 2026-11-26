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

/// 逻辑像素 -> 物理像素（与 `view_engine::get_view_params` 里的 `scale` 同一公式）。
///
/// `box_core` 不依赖平台层，所以这里自带换算；改公式必须同时改两处
/// （平台层的同名换算在 `ffi_types::scale_for_dpi`）。
#[must_use]
pub fn scale_view_px(logical: i32, dpi: u32) -> i32 {
    logical.saturating_mul(dpi.max(1) as i32) / 96
}

/// 网格左右留白（**必须随 DPI 缩放**）。
///
/// 前科（2026-11-26 用户报「主副屏之间拖拽后图标位置变乱」的另一半根因）：
/// 旧实现写死 `padding = 8`，于是 `cols = (W - 16) / cell_w` 的分子不再是
/// DPI 同比量 —— 主屏 192 DPI 与副屏 96 DPI 下同一只框（物理宽减半、cell_w 也减半）
/// 会算出相差整整一列的列数，跨屏瞬间列数跳变，固定位图标集体错位换行。
/// 留白 DPI 化后分子分母同比，**列数对 DPI 恒定**。
#[must_use]
pub fn grid_side_pad(dpi: u32) -> i32 {
    scale_view_px(8, dpi)
}

/// 标题栏与网格区之间的间隙（同样必须 DPI 化，理由同 `grid_side_pad`）。
///
/// 另外渲染端与滚轮上限都以 `grid_top` 起算可见高，两处口径必须一致，
/// 否则会出现「滚得动却没有滚动条」或「有条但滚不到底」。
#[must_use]
pub fn grid_top_gap(dpi: u32) -> i32 {
    scale_view_px(8, dpi)
}

/// 计算网格布局
///
/// 自动换行逻辑：
/// - 列数 = 可用宽度 / 单元格宽度（`fixed_cols` 指定的视图模式除外）；
///   窗口变窄 -> 列数变少 -> 条目自动换到下一行，窗口变宽 -> 自动换回上一行（需求 2.1）
/// - 当窗口宽度连一个 cell 都放不下时，强制 cols=1 并把 cell_w clamp 到可用宽度，
///   保证图标始终可见、不会被「挤到看不到」
pub fn calculate_grid(
    window_width: i32,
    title_bar_height: i32,
    params: &ViewParams,
    dpi: u32,
) -> GridLayout {
    let padding = grid_side_pad(dpi);
    let body_w = (window_width - padding * 2).max(1);

    // 列数：自动计算或固定（cell_w 兜底 >=1，避免 0 除）
    let cols = if let Some(fixed) = params.fixed_cols {
        fixed.max(1)
    } else {
        (body_w / params.cell_w.max(1)).max(1)
    };

    // 关键：当单列 cell 宽度超出可用宽度时，clamp 到 body_w（防止图标超出可视区域）
    let effective_cell_w = if cols == 1 && params.cell_w > body_w {
        body_w
    } else {
        params.cell_w
    };

    let grid_left = (window_width - cols * effective_cell_w) / 2;
    let grid_top = title_bar_height + grid_top_gap(dpi);

    GridLayout {
        cols,
        grid_left,
        grid_top,
        cell_w: effective_cell_w,
        cell_h: params.cell_h,
    }
}

/// 跨屏 DPI 重采样时，把内容宽度微调回「上一轮列数」所在的区间。
///
/// 留白 DPI 化后列数已对 DPI 恒定，剩下的误差只有物理像素折半取整带来的 1~2 px；
/// 这点误差正好落在一列的临界边上，列数就会莫名 +/-1（用户看到的仍是「图标变乱」）。
/// 因此**只在偏差 <= tol 时吸附**；超出 tol 说明是用户真的改了尺寸，
/// 那种情况必须放行，交给自动换行（需求 2.1）。
#[must_use]
pub fn snap_width_for_cols(
    target_w: i32,
    want_cols: i32,
    cell_w: i32,
    side_pad: i32,
    tol: i32,
) -> i32 {
    if want_cols <= 0 || cell_w <= 0 {
        return target_w;
    }
    let lo = want_cols
        .saturating_mul(cell_w)
        .saturating_add(side_pad.saturating_mul(2));
    let hi = lo + cell_w - 1;
    if target_w >= lo && target_w <= hi {
        return target_w;
    }
    let tol = tol.max(0);
    if target_w < lo && lo - target_w <= tol {
        return lo;
    }
    if target_w > hi && target_w - hi <= tol {
        return hi;
    }
    target_w
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::box_core::view_engine::ViewParams;

    fn params(logical_cell_w: i32, logical_cell_h: i32, dpi: u32) -> ViewParams {
        ViewParams {
            icon_size: scale_view_px(48, dpi),
            cell_w: scale_view_px(logical_cell_w, dpi),
            cell_h: scale_view_px(logical_cell_h, dpi),
            show_label: true,
            show_details: false,
            fixed_cols: None,
        }
    }

    #[test]
    fn cols_are_dpi_invariant_across_monitors() {
        // 同一只框：主屏 192 DPI 物理宽 738 / 副屏 96 DPI 物理宽 369（严格同比）。
        // 旧实现留白写死 8px，两侧算出的列数会差一列 -> 跨屏即「图标变乱」。
        let a = calculate_grid(738, scale_view_px(40, 192), &params(120, 140, 192), 192);
        let b = calculate_grid(369, scale_view_px(40, 96), &params(120, 140, 96), 96);
        assert_eq!(
            a.cols, b.cols,
            "列数必须与 DPI 无关: {} vs {}",
            a.cols, b.cols
        );
        assert_eq!(a.cell_w, b.cell_w * 2, "cell 宽度应严格同比");
        // grid_left 是 (W - cols*cell_w)/2 的居中取整，物理宽为奇数时本身带 +/-0.5px 误差，
        // 所以只要求近似同比；必须严格相等的是 cols（列数）与 cell_w（步长）。
        assert!(
            (a.grid_left - b.grid_left * 2).abs() <= 1,
            "网格原点应近似同比: {} vs {}",
            a.grid_left,
            b.grid_left
        );
        assert_eq!(
            a.grid_top,
            b.grid_top * 2,
            "grid_top 应严格同比（滚动可见高同口径）"
        );
    }

    #[test]
    fn cols_react_to_width_changes_for_auto_wrap() {
        // 需求 2.1：变窄 -> 列数变少（换到下一行）；变宽 -> 列数变多（换回上一行）。
        let wide = calculate_grid(816, 40, &params(120, 140, 96), 96);
        let mid = calculate_grid(408, 40, &params(120, 140, 96), 96);
        let narrow = calculate_grid(200, 40, &params(120, 140, 96), 96);
        assert_eq!((wide.cols, mid.cols, narrow.cols), (6, 3, 1));
    }

    #[test]
    fn ultra_narrow_window_clamps_single_cell_instead_of_hiding_icons() {
        let g = calculate_grid(60, 40, &params(120, 140, 96), 96);
        assert_eq!(g.cols, 1);
        assert!(
            g.cell_w <= 60,
            "cell_w 必须被夹进可视宽度, got {}",
            g.cell_w
        );
    }

    #[test]
    fn snap_width_only_absorbs_rounding_noise() {
        // want 3 列、cell_w=120、左右各 8 -> 合法区间 [376, 495]
        assert_eq!(
            snap_width_for_cols(376, 3, 120, 8, 4),
            376,
            "区间内原样返回"
        );
        assert_eq!(snap_width_for_cols(495, 3, 120, 8, 4), 495);
        assert_eq!(
            snap_width_for_cols(374, 3, 120, 8, 4),
            376,
            "2px 取整误差 -> 吸附"
        );
        assert_eq!(snap_width_for_cols(497, 3, 120, 8, 4), 495);
        // 用户真的把框缩小成 2 列了 -> 不能强撑回 3 列（否则需求 2.1 的换行失效）
        assert_eq!(
            snap_width_for_cols(300, 3, 120, 8, 4),
            300,
            "超出容差必须放行"
        );
        assert_eq!(snap_width_for_cols(900, 3, 120, 8, 4), 900);
        assert_eq!(
            snap_width_for_cols(300, 0, 120, 8, 4),
            300,
            "没有旧列数时不干预"
        );
    }

    #[test]
    fn snapped_width_keeps_cols_after_rescale() {
        // 吸附后的宽度必须真的算出目标列数（否则吸附无意义）
        let vp = params(120, 140, 96);
        let w = snap_width_for_cols(375, 3, vp.cell_w, grid_side_pad(96), 4);
        assert_eq!(calculate_grid(w, 40, &vp, 96).cols, 3);
    }
}
