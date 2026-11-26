//! 布局重排逻辑
//!
//! 从 platform/windows/grid_layout.rs 提取的平台无关布局算法。
//! 分类框与桌面覆盖层共用的纯逻辑，无平台依赖。

/// 在窗口宽度变化时重新映射图标的固定网格位置
///
/// 将每个图标的 (grid_col, grid_row) 按线性索引映射到新的列数下，
/// 保持视觉顺序和空位不变。
///
/// 例如 4 列时 A, B, _, C（C 在 col=3），变为 2 列后变成
/// A, B / _, C（C 仍然在第 2 列），而不是 A, B / C, _。
///
/// # 参数
/// - positions: 每项的 (grid_col, grid_row) 位置，负数表示自动布局（不处理）
/// - new_cols: 新的列数
/// - old_cols_hint: **上一轮真实生效的列数**。必须优先使用它：
///   框里只要存在空洞（删除过图标、手动留空位）或最后一行不满，
///   「max(col)+1 当旧列数」就会算小，线性重映射随之整体错位
///   （2026-11-26 用户报「主副屏之间拖拽后图标位置变乱」的根因之一）。
///   传 `None`/<=0 时才退回位置推断。
pub fn reflow_positions(positions: &mut [(i32, i32)], new_cols: i32, old_cols_hint: i32) {
    if new_cols <= 0 {
        return;
    }

    // 列数没变 → 绝不重映射（跨屏 DPI 重采样时物理尺寸变了但列数通常不变，
    // 任何一次多余的线性重映射都不可逆，会把用户布局打乱）。
    let has_fixed = positions.iter().any(|(col, _)| *col >= 0);
    if !has_fixed {
        return;
    }
    let inferred = positions
        .iter()
        .filter(|(col, _)| *col >= 0)
        .map(|(col, _)| col + 1)
        .max()
        .unwrap_or(new_cols);
    // 有调用方真实值时**直接采信**（这正是本次修复的核心：推断值在有空洞时必然偏小，
    // 而「列数变多」时推断值又必然偏大——两种方向都不能靠猜）。
    // 仅在没给 hint 时退回推断，并保证不高于「已使用的最大列 + 1」以免映射越界。
    let old_cols = if old_cols_hint > 0 {
        old_cols_hint.max(1)
    } else {
        inferred.max(1)
    };
    if old_cols == new_cols {
        return;
    }

    for (col, row) in positions.iter_mut() {
        if *col < 0 || *row < 0 {
            // 无固定位置的项保持自动布局
            continue;
        }
        let linear_idx = *row * old_cols + *col;
        *col = linear_idx % new_cols;
        *row = linear_idx / new_cols;
    }
}

/// 单个条目实际占据的行号
///
/// 固定位（`grid_col`/`grid_row` 均 >= 0）用自己的 `grid_row`；自动位按当前列数
/// 线性排布。必须与 `d2d_render::render_icon_grid` 的落位公式**完全同式**，
/// 否则滚动上限会算短，底部图标永远滚不到。
#[must_use]
pub fn row_extent(index: usize, grid_col: i32, grid_row: i32, cols: i32) -> i32 {
    if grid_col >= 0 && grid_row >= 0 {
        grid_row.max(0)
    } else {
        (index as i32) / cols.max(1)
    }
}

/// 内容总行数（按条目实际占据的行取最大，**不能**用 `len()/cols`）
///
/// 前科：`total_rows = (len + cols - 1) / cols` 在「有空洞」或「用户把图标手动
/// 放到很靠下的行」时直接算小，于是 `max_scroll` 偏小、滚动条也不出现，
/// 表现就是用户报的「缩小时展示不下也没有滚动条」。
#[must_use]
pub fn content_rows<T>(
    items: &[T],
    cols: i32,
    position_of: impl Fn(usize, &T) -> (i32, i32),
) -> i32 {
    let cols = cols.max(1);
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let (col, row) = position_of(i, it);
            row_extent(i, col, row, cols) + 1
        })
        .max()
        .unwrap_or(0)
}

/// 滚动上限 = 内容总高 - 可视高（非负）
#[must_use]
pub fn scroll_limit(rows: i32, cell_h: i32, visible_h: i32) -> i32 {
    (rows.saturating_mul(cell_h.max(0)) - visible_h.max(0)).max(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reflow_prefers_hint_over_inference_when_cols_have_holes() {
        // 真实 6 列，但项只落在 col 0/2/5：空洞让「max(col)+1」在部分布局下偏小，
        // 一旦偏小，线性重映射就会把项挪到别的行列（用户看到的「位置变乱」）。
        let items = vec![(0, 0), (2, 0), (5, 1)];
        let mut inferred = items.clone();
        reflow_positions(&mut inferred, 3, 0); // 无 hint：只能推断（此例恰好=6）
        let mut hinted = items.clone();
        reflow_positions(&mut hinted, 3, 6); // 显式告知真实 6 列
                                             // 推断恰好等于真实值时两者一致 —— 说明推断只是「碰巧对」，不可依赖。
        assert_eq!(inferred, hinted);
        assert_eq!(hinted, vec![(0, 0), (2, 0), (2, 3)]);

        // 关键用例：真实 8 列、只用过 col 0..3 → 推断 4，比真实值小。
        let holey = vec![(0, 0), (3, 0), (1, 1)];
        let mut wrong = holey.clone();
        reflow_positions(&mut wrong, 3, 0); // 推断 4 列
        let mut right = holey.clone();
        reflow_positions(&mut right, 3, 8); // 真实 8 列
        assert_ne!(wrong, right, "推断偏小时两种口径必然不同");
        // 真实 8 列：(3,0) 线性 3 -> (0,1)；(1,1) 线性 9 -> (0,3)
        assert_eq!(right, vec![(0, 0), (0, 1), (0, 3)]);
        // 被当成 4 列：(1,1) 线性 5 -> (2,1) —— 直接跑到了另一行
        assert_eq!(wrong, vec![(0, 0), (0, 1), (2, 1)]);
    }

    #[test]
    fn reflow_hint_is_authoritative_in_both_directions() {
        // 列数变多（2 -> 4）：采信 hint=2，不能被「max(col)+1」顶上去。
        let mut grow = vec![(0, 0), (1, 0), (0, 1)];
        reflow_positions(&mut grow, 4, 2);
        assert_eq!(grow, vec![(0, 0), (1, 0), (2, 0)], "2 列 -> 4 列");

        // 列数变少（6 -> 4）：同样采信 hint=6。
        let mut shrink = vec![(0, 0), (5, 2)];
        reflow_positions(&mut shrink, 4, 6);
        // 线性 6*2+5 = 17 -> (1, 4)
        assert_eq!(shrink[1], (1, 4));
    }

    #[test]
    fn reflow_is_noop_when_cols_unchanged() {
        // 跨屏 DPI 重采样后列数常常不变，此时任何重映射都是净伤害。
        let mut positions = vec![(0, 0), (2, 0), (1, 3), (-1, -1)];
        let before = positions.clone();
        reflow_positions(&mut positions, 4, 4);
        assert_eq!(positions, before, "列数不变必须原样返回");
    }

    #[test]
    fn reflow_without_fixed_positions_is_noop() {
        let mut positions = vec![(-1, -1), (-1, -1)];
        reflow_positions(&mut positions, 2, 5);
        assert_eq!(positions, vec![(-1, -1), (-1, -1)]);
    }

    use super::*;

    #[test]
    fn test_reflow_4cols_to_2cols() {
        // Original: 4 cols, A(0,0), B(1,0), _(2,0), C(3,0)
        // Expected: 2 cols, A(0,0), B(1,0), _(0,1), C(1,1)
        let mut positions = vec![(0, 0), (1, 0), (3, 0)];
        reflow_positions(&mut positions, 2, 0);
        assert_eq!(positions[0], (0, 0)); // A
        assert_eq!(positions[1], (1, 0)); // B
        assert_eq!(positions[2], (1, 1)); // C (linear 3 -> row 1, col 1)
    }

    #[test]
    fn test_reflow_2cols_to_4cols() {
        // Original: 2 cols, A(0,0), B(1,0), C(0,1)
        let mut positions = vec![(0, 0), (1, 0), (0, 1)];
        reflow_positions(&mut positions, 4, 0);
        assert_eq!(positions[0], (0, 0)); // A
        assert_eq!(positions[1], (1, 0)); // B
        assert_eq!(positions[2], (2, 0)); // C (linear 2 -> row 0, col 2)
    }

    #[test]
    fn test_reflow_preserves_auto_layout() {
        let mut positions = vec![(-1, -1), (1, 0), (-1, -1)];
        reflow_positions(&mut positions, 3, 0);
        assert_eq!(positions[0], (-1, -1));
        assert_eq!(positions[1], (1, 0));
        assert_eq!(positions[2], (-1, -1));
    }

    #[test]
    fn test_reflow_empty() {
        let mut positions: Vec<(i32, i32)> = vec![];
        reflow_positions(&mut positions, 3, 0);
        assert!(positions.is_empty());
    }

    #[test]
    fn test_reflow_all_auto() {
        let mut positions = vec![(-1, -1), (-1, -1)];
        reflow_positions(&mut positions, 3, 0);
        assert_eq!(positions[0], (-1, -1));
        assert_eq!(positions[1], (-1, -1));
    }

    #[test]
    fn test_reflow_invalid_cols() {
        let mut positions = vec![(0, 0)];
        reflow_positions(&mut positions, 0, 0);
        assert_eq!(positions[0], (0, 0));
    }

    #[test]
    fn test_reflow_single_col() {
        // 3 cols -> 1 col: A(0,0), B(1,0), C(2,0) -> A(0,0), B(0,1), C(0,2)
        let mut positions = vec![(0, 0), (1, 0), (2, 0)];
        reflow_positions(&mut positions, 1, 0);
        assert_eq!(positions[0], (0, 0));
        assert_eq!(positions[1], (0, 1));
        assert_eq!(positions[2], (0, 2));
    }

    #[test]
    fn content_rows_counts_fixed_positions_beyond_linear_flow() {
        // 4 个条目，但最后一个被手动放到第 9 行 -> 真实行数 10，而不是 ceil(4/4)=1
        let items = vec![(0, 0), (1, 0), (2, 0), (0, 9)];
        let rows = content_rows(&items, 4, |_, p| *p);
        assert_eq!(rows, 10);
        // 用旧口径会算成 1 行 -> 滚动上限 0 -> 第 9 行的图标永远看不见也滚不到
        assert_eq!((items.len() as i32 + 4 - 1) / 4, 1);
    }
    #[test]
    fn content_rows_matches_linear_flow_for_auto_items() {
        // 全是自动位（负数）：行数 = ceil(len / cols)，与渲染落位一致
        let items = vec![(-1, -1); 7];
        assert_eq!(content_rows(&items, 3, |_, p| *p), 3);
        assert_eq!(content_rows(&[], 3, |_: usize, p: &(i32, i32)| *p), 0);
        assert_eq!(
            content_rows(&items, 0, |_, p| *p),
            7,
            "cols<=0 时按每项一行兜底"
        );
    }
    #[test]
    fn scroll_limit_never_negative_and_matches_overflow() {
        assert_eq!(scroll_limit(5, 100, 400), 100);
        assert_eq!(scroll_limit(3, 100, 400), 0, "放得下就不该滚");
        assert_eq!(scroll_limit(3, 100, -10), 300, "可视高为负也不能算出负上限");
    }
    #[test]
    fn row_extent_prefers_grid_row_for_fixed_items() {
        assert_eq!(row_extent(0, 2, 7, 4), 7);
        assert_eq!(row_extent(9, -1, -1, 4), 2, "自动位按线性索引");
        assert_eq!(row_extent(9, -1, -1, 0), 9, "cols<=0 兜底为一列");
    }
}
