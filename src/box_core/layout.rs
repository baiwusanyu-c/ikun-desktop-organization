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
pub fn reflow_positions(positions: &mut [(i32, i32)], new_cols: i32) {
    if new_cols <= 0 {
        return;
    }

    // 从固定位置推断旧的列数
    let old_cols = positions
        .iter()
        .filter(|(col, _)| *col >= 0)
        .map(|(col, _)| col + 1)
        .max()
        .unwrap_or(new_cols);
    let old_cols = old_cols.max(1);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reflow_4cols_to_2cols() {
        // Original: 4 cols, A(0,0), B(1,0), _(2,0), C(3,0)
        // Expected: 2 cols, A(0,0), B(1,0), _(0,1), C(1,1)
        let mut positions = vec![(0, 0), (1, 0), (3, 0)];
        reflow_positions(&mut positions, 2);
        assert_eq!(positions[0], (0, 0)); // A
        assert_eq!(positions[1], (1, 0)); // B
        assert_eq!(positions[2], (1, 1)); // C (linear 3 -> row 1, col 1)
    }

    #[test]
    fn test_reflow_2cols_to_4cols() {
        // Original: 2 cols, A(0,0), B(1,0), C(0,1)
        let mut positions = vec![(0, 0), (1, 0), (0, 1)];
        reflow_positions(&mut positions, 4);
        assert_eq!(positions[0], (0, 0)); // A
        assert_eq!(positions[1], (1, 0)); // B
        assert_eq!(positions[2], (2, 0)); // C (linear 2 -> row 0, col 2)
    }

    #[test]
    fn test_reflow_preserves_auto_layout() {
        let mut positions = vec![(-1, -1), (1, 0), (-1, -1)];
        reflow_positions(&mut positions, 3);
        assert_eq!(positions[0], (-1, -1));
        assert_eq!(positions[1], (1, 0));
        assert_eq!(positions[2], (-1, -1));
    }

    #[test]
    fn test_reflow_empty() {
        let mut positions: Vec<(i32, i32)> = vec![];
        reflow_positions(&mut positions, 3);
        assert!(positions.is_empty());
    }

    #[test]
    fn test_reflow_all_auto() {
        let mut positions = vec![(-1, -1), (-1, -1)];
        reflow_positions(&mut positions, 3);
        assert_eq!(positions[0], (-1, -1));
        assert_eq!(positions[1], (-1, -1));
    }

    #[test]
    fn test_reflow_invalid_cols() {
        let mut positions = vec![(0, 0)];
        reflow_positions(&mut positions, 0);
        assert_eq!(positions[0], (0, 0));
    }

    #[test]
    fn test_reflow_single_col() {
        // 3 cols -> 1 col: A(0,0), B(1,0), C(2,0) -> A(0,0), B(0,1), C(0,2)
        let mut positions = vec![(0, 0), (1, 0), (2, 0)];
        reflow_positions(&mut positions, 1);
        assert_eq!(positions[0], (0, 0));
        assert_eq!(positions[1], (0, 1));
        assert_eq!(positions[2], (0, 2));
    }
}
