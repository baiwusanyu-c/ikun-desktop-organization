//! Grid snap-to-cell layout for desktop overlay icons.
//!
//! Provides grid cell tracking, snap-to-grid, cell occupation detection,
//! and swap logic for drag-and-drop within the desktop overlay window.
//! All calculations use overlay_layout_params for consistent layout
//! across different DPI/resolution settings.

use super::overlay_window::{
    overlay_layout_params, logical_to_physical, physical_to_logical,
    OverlayWindowData,
};

// =============================================================================
// Grid Snap
// =============================================================================

pub(crate) fn overlay_grid_col_row(data: &OverlayWindowData, physical_x: i32, physical_y: i32) -> (i32, i32) {
    let layout = overlay_layout_params(data.width, data.dpi);
    let col = ((physical_x - layout.padding) / layout.cell_w).max(0);
    let row = ((physical_y - layout.padding) / layout.cell_h).max(0);
    (col, row)
}

pub(crate) fn overlay_grid_to_logical(data: &OverlayWindowData, col: i32, row: i32) -> [f32; 2] {
    let layout = overlay_layout_params(data.width, data.dpi);
    let physical_x = layout.padding + col * layout.cell_w;
    let physical_y = layout.padding + row * layout.cell_h;
    [
        physical_to_logical(physical_x, data.dpi),
        physical_to_logical(physical_y, data.dpi),
    ]
}

pub(crate) fn overlay_grid_occupied_at(
    data: &OverlayWindowData,
    target_col: i32,
    target_row: i32,
    skip_index: i32,
) -> Option<usize> {
    let layout = overlay_layout_params(data.width, data.dpi);
    for (index, icon) in data.icons.iter().enumerate() {
        if index as i32 == skip_index {
            continue;
        }
        let icon_col;
        let icon_row;
        if icon.x >= 0.0 && icon.y >= 0.0 {
            let px = logical_to_physical(icon.x, data.dpi);
            let py = logical_to_physical(icon.y, data.dpi);
            icon_col = ((px - layout.padding) / layout.cell_w).max(0);
            icon_row = ((py - layout.padding) / layout.cell_h).max(0);
        } else {
            let total = data.icons.len().max(1) as i32;
            let rows = (total + layout.cols - 1) / layout.cols;
            icon_col = (index as i32) / rows;
            icon_row = (index as i32) % rows;
        }
        if icon_col == target_col && icon_row == target_row {
            return Some(index);
        }
    }
    None
}

pub(crate) fn overlay_grid_max_rows(data: &OverlayWindowData) -> i32 {
    let layout = overlay_layout_params(data.width, data.dpi);
    ((data.height - layout.padding * 2) / layout.cell_h).max(1)
}

pub(crate) fn overlay_first_free_cell(data: &OverlayWindowData) -> (i32, i32) {
    let layout = overlay_layout_params(data.width, data.dpi);
    let max_rows = overlay_grid_max_rows(data);
    let mut occupied = std::collections::HashSet::new();
    for (index, icon) in data.icons.iter().enumerate() {
        let col;
        let row;
        if icon.x >= 0.0 && icon.y >= 0.0 {
            let px = logical_to_physical(icon.x, data.dpi);
            let py = logical_to_physical(icon.y, data.dpi);
            col = ((px - layout.padding) / layout.cell_w).max(0);
            row = ((py - layout.padding) / layout.cell_h).max(0);
        } else {
            let total = data.icons.len().max(1) as i32;
            let rows = (total + layout.cols - 1) / layout.cols;
            col = (index as i32) / rows;
            row = (index as i32) % rows;
        }
        occupied.insert((col, row));
    }
    for col in 0..layout.cols {
        for row in 0..max_rows {
            if !occupied.contains(&(col, row)) {
                return (col, row);
            }
        }
    }
    let fallback_col = layout.cols.saturating_sub(1);
    let fallback_row = max_rows;
    (fallback_col, fallback_row)
}

pub(crate) fn overlay_snap_to_grid(
    data: &OverlayWindowData,
    physical_x: i32,
    physical_y: i32,
    skip_index: i32,
) -> ([f32; 2], Option<usize>) {
    let (col, row) = overlay_grid_col_row(data, physical_x, physical_y);
    let layout = overlay_layout_params(data.width, data.dpi);
    let max_rows = overlay_grid_max_rows(data);
    let col = col.min(layout.cols.saturating_sub(1).max(0));
    let row = row.min(max_rows.saturating_sub(1).max(0));
    let occupied = overlay_grid_occupied_at(data, col, row, skip_index);
    let logical = overlay_grid_to_logical(data, col, row);
    (logical, occupied)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::windows::desktop_overlay::overlay_window::{OverlayIconItem, OverlayWindowData};

// ===== Grid Snap Tests =====

#[test]
fn test_overlay_grid_col_row_basic() {
    let data = OverlayWindowData::test_default();
    let layout = overlay_layout_params(data.width, data.dpi);
    // At (padding, padding) should be (0, 0)
    let (col, row) = overlay_grid_col_row(&data, layout.padding, layout.padding);
    assert_eq!(col, 0);
    assert_eq!(row, 0);
    // At (padding + cell_w, padding) should be (1, 0)
    let (col, row) = overlay_grid_col_row(&data, layout.padding + layout.cell_w, layout.padding);
    assert_eq!(col, 1);
    assert_eq!(row, 0);
    // At (padding, padding + cell_h) should be (0, 1)
    let (col, row) = overlay_grid_col_row(&data, layout.padding, layout.padding + layout.cell_h);
    assert_eq!(col, 0);
    assert_eq!(row, 1);
}

#[test]
fn test_overlay_grid_col_row_negative_coords() {
    let data = OverlayWindowData::test_default();
    // Negative coords should clamp to (0, 0)
    let (col, row) = overlay_grid_col_row(&data, -100, -100);
    assert_eq!(col, 0);
    assert_eq!(row, 0);
}

#[test]
fn test_overlay_grid_to_logical_roundtrip() {
    let data = OverlayWindowData::test_default();
    let layout = overlay_layout_params(data.width, data.dpi);
    // (0, 0) should map to (padding, padding) in physical
    let logical = overlay_grid_to_logical(&data, 0, 0);
    let back_x = logical_to_physical(logical[0], data.dpi);
    let back_y = logical_to_physical(logical[1], data.dpi);
    assert_eq!(back_x, layout.padding);
    assert_eq!(back_y, layout.padding);
}

#[test]
fn test_overlay_grid_max_rows() {
    let data = OverlayWindowData::test_default();
    let max_rows = overlay_grid_max_rows(&data);
    assert!(max_rows > 0, "max_rows should be positive");
    let layout = overlay_layout_params(data.width, data.dpi);
    // Should fit within height
    assert!(max_rows * layout.cell_h + layout.padding * 2 <= data.height + layout.cell_h);
}

#[test]
fn test_overlay_grid_occupied_empty() {
    let data = OverlayWindowData::test_default();
    // No icons, nothing occupied
    let result = overlay_grid_occupied_at(&data, 0, 0, -1);
    assert!(result.is_none());
}

#[test]
fn test_overlay_grid_occupied_with_icon() {
    let mut data = OverlayWindowData::test_default();
    // Place an icon at grid (0, 0)
    let pos = overlay_grid_to_logical(&data, 0, 0);
    data.icons.push(OverlayIconItem {
        file_path: "test.lnk".to_string(),
        display_name: "Test".to_string(),
        icon_handle: std::ptr::null_mut(),
        x: pos[0],
        y: pos[1],
    });
    // (0, 0) should be occupied by index 0
    let result = overlay_grid_occupied_at(&data, 0, 0, -1);
    assert_eq!(result, Some(0));
    // (1, 0) should be free
    let result = overlay_grid_occupied_at(&data, 1, 0, -1);
    assert!(result.is_none());
}

#[test]
fn test_overlay_grid_occupied_skip_index() {
    let mut data = OverlayWindowData::test_default();
    let pos = overlay_grid_to_logical(&data, 0, 0);
    data.icons.push(OverlayIconItem {
        file_path: "test.lnk".to_string(),
        display_name: "Test".to_string(),
        icon_handle: std::ptr::null_mut(),
        x: pos[0],
        y: pos[1],
    });
    // Skip index 0 -> should be None (skipping itself)
    let result = overlay_grid_occupied_at(&data, 0, 0, 0);
    assert!(result.is_none());
}

#[test]
fn test_overlay_first_free_cell_empty() {
    let data = OverlayWindowData::test_default();
    let (col, row) = overlay_first_free_cell(&data);
    assert_eq!(col, 0);
    assert_eq!(row, 0);
}

#[test]
fn test_overlay_first_free_cell_with_icons() {
    let mut data = OverlayWindowData::test_default();
    // Place icons at (0,0) and (0,1)
    let pos0 = overlay_grid_to_logical(&data, 0, 0);
    let pos1 = overlay_grid_to_logical(&data, 0, 1);
    data.icons.push(OverlayIconItem {
        file_path: "a.lnk".to_string(),
        display_name: "A".to_string(),
        icon_handle: std::ptr::null_mut(),
        x: pos0[0],
        y: pos0[1],
    });
    data.icons.push(OverlayIconItem {
        file_path: "b.lnk".to_string(),
        display_name: "B".to_string(),
        icon_handle: std::ptr::null_mut(),
        x: pos1[0],
        y: pos1[1],
    });
    let (col, row) = overlay_first_free_cell(&data);
    // Column-first: (0,0) and (0,1) occupied, so next is (0,2)
    assert_eq!(col, 0);
    assert_eq!(row, 2);
}

#[test]
fn test_overlay_snap_to_grid_basic() {
    let data = OverlayWindowData::test_default();
    let layout = overlay_layout_params(data.width, data.dpi);
    // Snap to (0, 0)
    let (pos, occupied) = overlay_snap_to_grid(&data, layout.padding + 10, layout.padding + 10, -1);
    assert!(occupied.is_none());
    let expected = overlay_grid_to_logical(&data, 0, 0);
    assert!((pos[0] - expected[0]).abs() < 0.1);
    assert!((pos[1] - expected[1]).abs() < 0.1);
}

#[test]
fn test_overlay_snap_to_grid_occupied() {
    let mut data = OverlayWindowData::test_default();
    let pos0 = overlay_grid_to_logical(&data, 0, 0);
    data.icons.push(OverlayIconItem {
        file_path: "test.lnk".to_string(),
        display_name: "Test".to_string(),
        icon_handle: std::ptr::null_mut(),
        x: pos0[0],
        y: pos0[1],
    });
    let layout = overlay_layout_params(data.width, data.dpi);
    // Snap to (0, 0) which is occupied
    let (_pos, occupied) = overlay_snap_to_grid(&data, layout.padding + 10, layout.padding + 10, -1);
    assert_eq!(occupied, Some(0));
}

#[test]
fn test_overlay_snap_to_grid_skip_self() {
    let mut data = OverlayWindowData::test_default();
    let pos0 = overlay_grid_to_logical(&data, 0, 0);
    data.icons.push(OverlayIconItem {
        file_path: "test.lnk".to_string(),
        display_name: "Test".to_string(),
        icon_handle: std::ptr::null_mut(),
        x: pos0[0],
        y: pos0[1],
    });
    let layout = overlay_layout_params(data.width, data.dpi);
    // Snap to (0, 0) with skip_index=0 (dragging self)
    let (_pos, occupied) = overlay_snap_to_grid(&data, layout.padding + 10, layout.padding + 10, 0);
    assert!(occupied.is_none(), "Should not detect self as occupied");
}

#[test]
fn test_overlay_grid_multi_dpi() {
    let mut data = OverlayWindowData::test_default();
    // Test at different DPI values
    for dpi in [96, 120, 144, 192] {
        data.dpi = dpi;
        let layout = overlay_layout_params(data.width, dpi);
        let (col, row) = overlay_grid_col_row(&data, layout.padding, layout.padding);
        assert_eq!(col, 0, "col should be 0 at dpi={}", dpi);
        assert_eq!(row, 0, "row should be 0 at dpi={}", dpi);
        let max_rows = overlay_grid_max_rows(&data);
        assert!(max_rows > 0, "max_rows should be positive at dpi={}", dpi);
    }
}

    // ===== Edge Case Tests =====

    #[test]
    fn test_overlay_grid_swap_logic() {
        let mut data = OverlayWindowData::test_default();
        let pos_a = overlay_grid_to_logical(&data, 0, 0);
        let pos_b = overlay_grid_to_logical(&data, 1, 0);
        data.icons.push(OverlayIconItem {
            file_path: "a.lnk".to_string(),
            display_name: "A".to_string(),
            icon_handle: std::ptr::null_mut(),
            x: pos_a[0],
            y: pos_a[1],
        });
        data.icons.push(OverlayIconItem {
            file_path: "b.lnk".to_string(),
            display_name: "B".to_string(),
            icon_handle: std::ptr::null_mut(),
            x: pos_b[0],
            y: pos_b[1],
        });

        let layout = overlay_layout_params(data.width, data.dpi);
        // Dragging A (index 0) to (1,0) should detect B (index 1)
        let target_x = layout.padding + layout.cell_w + 10;
        let target_y = layout.padding + 10;
        let (_snap_pos, occupied) = overlay_snap_to_grid(&data, target_x, target_y, 0);
        assert_eq!(occupied, Some(1));

        // Dragging B (index 1) to (0,0) should detect A (index 0)
        let target_x = layout.padding + 10;
        let target_y = layout.padding + 10;
        let (_snap_pos, occupied) = overlay_snap_to_grid(&data, target_x, target_y, 1);
        assert_eq!(occupied, Some(0));
    }

    #[test]
    fn test_overlay_grid_mixed_positioned_and_auto() {
        let mut data = OverlayWindowData::test_default();
        let layout = overlay_layout_params(data.width, data.dpi);

        // Icon 0: user-positioned at (2, 3)
        let pos = overlay_grid_to_logical(&data, 2, 3);
        data.icons.push(OverlayIconItem {
            file_path: "user.lnk".to_string(),
            display_name: "User".to_string(),
            icon_handle: std::ptr::null_mut(),
            x: pos[0],
            y: pos[1],
        });

        // Icon 1: auto-layout (x=-1, y=-1)
        data.icons.push(OverlayIconItem {
            file_path: "auto.lnk".to_string(),
            display_name: "Auto".to_string(),
            icon_handle: std::ptr::null_mut(),
            x: -1.0,
            y: -1.0,
        });

        // (2, 3) should be occupied by icon 0
        let result = overlay_grid_occupied_at(&data, 2, 3, -1);
        assert_eq!(result, Some(0));

        // Auto-layout icon position
        let total = 2i32;
        let rows = (total + layout.cols - 1) / layout.cols;
        let auto_col = 1 / rows;
        let auto_row = 1 % rows;
        let result = overlay_grid_occupied_at(&data, auto_col, auto_row, -1);
        assert_eq!(result, Some(1));
    }

    #[test]
    fn test_overlay_snap_to_grid_clamps_to_bounds() {
        let data = OverlayWindowData::test_default();
        let layout = overlay_layout_params(data.width, data.dpi);

        let huge_x = data.width * 2;
        let huge_y = data.height * 2;
        let (pos, _occupied) = overlay_snap_to_grid(&data, huge_x, huge_y, -1);

        let max_rows = overlay_grid_max_rows(&data);
        let expected = overlay_grid_to_logical(
            &data,
            layout.cols.saturating_sub(1).max(0),
            max_rows.saturating_sub(1).max(0),
        );
        assert!((pos[0] - expected[0]).abs() < 0.1);
        assert!((pos[1] - expected[1]).abs() < 0.1);
    }

    #[test]
    fn test_overlay_first_free_cell_skips_full_column() {
        let mut data = OverlayWindowData::test_default();
        let layout = overlay_layout_params(data.width, data.dpi);
        let max_rows = ((data.height - layout.padding * 2) / layout.cell_h).max(1);

        for row in 0..max_rows {
            let pos = overlay_grid_to_logical(&data, 0, row);
            data.icons.push(OverlayIconItem {
                file_path: format!("icon_{}.lnk", row),
                display_name: format!("Icon {}", row),
                icon_handle: std::ptr::null_mut(),
                x: pos[0],
                y: pos[1],
            });
        }

        let (col, row) = overlay_first_free_cell(&data);
        assert!(col >= 1, "First free cell should be in column >= 1, got col={}", col);
        assert_eq!(row, 0, "First free cell in new column should be row 0");
    }

    #[test]
    fn test_overlay_grid_consistent_across_dpi_values() {
        let mut data = OverlayWindowData::test_default();

        for dpi in [96u32, 120, 144, 192, 240] {
            data.dpi = dpi;
            let layout = overlay_layout_params(data.width, dpi);

            let logical = overlay_grid_to_logical(&data, 0, 0);
            let back_x = (logical[0] * dpi as f32 / 96.0).round() as i32;
            let back_y = (logical[1] * dpi as f32 / 96.0).round() as i32;
            assert_eq!(back_x, layout.padding,
                "Grid (0,0) physical X mismatch at dpi={}", dpi);
            assert_eq!(back_y, layout.padding,
                "Grid (0,0) physical Y mismatch at dpi={}", dpi);

            assert!(layout.cols >= 2, "cols should be >= 2 at dpi={}", dpi);
            let max_rows = overlay_grid_max_rows(&data);
            assert!(max_rows >= 1, "max_rows should be >= 1 at dpi={}", dpi);
        }
    }

    #[test]
    fn test_overlay_grid_narrow_window() {
        let mut data = OverlayWindowData::test_default();
        data.width = 200;

        let layout = overlay_layout_params(data.width, data.dpi);
        assert!(layout.cols >= 2, "cols should be at least 2 even for narrow window");

        let (col, row) = overlay_first_free_cell(&data);
        assert!(col >= 0);
        assert!(row >= 0);

        let (pos, _) = overlay_snap_to_grid(&data, 50, 50, -1);
        assert!(pos[0] >= 0.0);
        assert!(pos[1] >= 0.0);
    }
}
