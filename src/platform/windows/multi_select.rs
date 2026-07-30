#![allow(dead_code)]
//! Global multi-selection registry for managing selected icons
//! across multiple containers (classification boxes and desktop overlay).
//!
//! This module provides a thread-safe, global selection state that can be
//! accessed from any window or component. All public functions acquire a
//! mutex lock, perform their operation, and release immediately.

use std::collections::BTreeSet;
use std::sync::Mutex;

use lazy_static::lazy_static;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Container type for selection keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContainerType {
    /// Desktop overlay (icons on the desktop itself).
    Overlay,
    /// A classification grid box.
    GridBox,
}

/// Unique key identifying a selectable item across all containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SelectionKey {
    /// Which kind of container holds this item.
    pub container_type: ContainerType,
    /// Container identifier. For GridBox this is derived from the
    /// config_id UUID (high 64 bits). For Overlay this is 0.
    pub container_id: u64,
    /// Zero-based index of the item within its container.
    pub item_index: usize,
}
// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

/// Global multi-select state.
struct MultiSelectState {
    /// All currently selected items, kept sorted for deterministic iteration.
    selected: BTreeSet<SelectionKey>,
    /// Anchor key for Shift+click range selection.
    anchor: Option<SelectionKey>,
    /// Marquee selection: tracks the container where the marquee is active.
    marquee_container: Option<(ContainerType, u64)>,
}

impl MultiSelectState {
    fn new() -> Self {
        Self {
            selected: BTreeSet::new(),
            anchor: None,
            marquee_container: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

lazy_static! {
    static ref STATE: Mutex<MultiSelectState> = Mutex::new(MultiSelectState::new());
}

// ---------------------------------------------------------------------------
// Public API - selection manipulation
// ---------------------------------------------------------------------------

/// Clear all selections and reset the anchor.
pub fn clear() {
    let mut s = STATE.lock().unwrap();
    s.selected.clear();
    s.anchor = None;
    s.marquee_container = None;
}

/// Add an item to the selection set.
pub fn add(key: SelectionKey) {
    let mut s = STATE.lock().unwrap();
    s.selected.insert(key);
}

/// Remove an item from the selection set.
pub fn remove(key: SelectionKey) {
    let mut s = STATE.lock().unwrap();
    s.selected.remove(&key);
}

/// Toggle an item\'s selection state.
pub fn toggle(key: SelectionKey) {
    let mut s = STATE.lock().unwrap();
    if !s.selected.remove(&key) {
        s.selected.insert(key);
    }
}

/// Check if an item is currently selected.
pub fn is_selected(key: SelectionKey) -> bool {
    let s = STATE.lock().unwrap();
    s.selected.contains(&key)
}

/// Get the number of currently selected items.
pub fn count() -> usize {
    let s = STATE.lock().unwrap();
    s.selected.len()
}

/// Get the current anchor key (for Shift+click range selection).
pub fn anchor() -> Option<SelectionKey> {
    let s = STATE.lock().unwrap();
    s.anchor
}

/// Set the anchor key.
pub fn set_anchor(key: Option<SelectionKey>) {
    let mut s = STATE.lock().unwrap();
    s.anchor = key;
}

// ---------------------------------------------------------------------------
// Public API - container-scoped queries
// ---------------------------------------------------------------------------

/// Get all selected item indices in a specific container.
pub fn selected_in_container(container_type: ContainerType, container_id: u64) -> Vec<usize> {
    let s = STATE.lock().unwrap();
    s.selected
        .iter()
        .filter(|k| k.container_type == container_type && k.container_id == container_id)
        .map(|k| k.item_index)
        .collect()
}

/// Get all selected keys (snapshot).
pub fn all_keys() -> Vec<SelectionKey> {
    let s = STATE.lock().unwrap();
    s.selected.iter().copied().collect()
}
/// Set range selection: clear current selection, then add all items in the
/// inclusive [start, end] range within the same container.
///
/// Returns the list of keys that were added.
pub fn set_range(
    container_type: ContainerType,
    container_id: u64,
    start: usize,
    end: usize,
) -> Vec<SelectionKey> {
    let (lo, hi) = if start <= end { (start, end) } else { (end, start) };
    let mut s = STATE.lock().unwrap();
    s.selected.clear();
    let added: Vec<SelectionKey> = (lo..=hi)
        .map(|idx| SelectionKey {
            container_type,
            container_id,
            item_index: idx,
        })
        .collect();
    for key in &added {
        s.selected.insert(*key);
    }
    added
}

/// Add range selection: add all items in the inclusive [start, end] range
/// within the same container **without** clearing existing selections.
///
/// Returns the list of keys that were newly added.
pub fn add_range(
    container_type: ContainerType,
    container_id: u64,
    start: usize,
    end: usize,
) -> Vec<SelectionKey> {
    let (lo, hi) = if start <= end { (start, end) } else { (end, start) };
    let mut s = STATE.lock().unwrap();
    let mut added = Vec::new();
    for idx in lo..=hi {
        let key = SelectionKey {
            container_type,
            container_id,
            item_index: idx,
        };
        if s.selected.insert(key) {
            added.push(key);
        }
    }
    added
}

/// Set range selection using explicit indices (visual order).
/// Clears current selection, then adds all specified indices.
///
/// Returns the list of keys that were added.
pub fn set_range_visual(
    container_type: ContainerType,
    container_id: u64,
    indices: &[usize],
) -> Vec<SelectionKey> {
    let mut s = STATE.lock().unwrap();
    s.selected.clear();
    let added: Vec<SelectionKey> = indices
        .iter()
        .map(|&idx| SelectionKey {
            container_type,
            container_id,
            item_index: idx,
        })
        .collect();
    for key in &added {
        s.selected.insert(*key);
    }
    added
}

/// Add range selection using explicit indices (visual order).
/// Does NOT clear existing selections.
///
/// Returns the list of keys that were newly added.
pub fn add_range_visual(
    container_type: ContainerType,
    container_id: u64,
    indices: &[usize],
) -> Vec<SelectionKey> {
    let mut s = STATE.lock().unwrap();
    let mut added = Vec::new();
    for &idx in indices {
        let key = SelectionKey {
            container_type,
            container_id,
            item_index: idx,
        };
        if s.selected.insert(key) {
            added.push(key);
        }
    }
    added
}

// ---------------------------------------------------------------------------
// Public API - misc helpers
// ---------------------------------------------------------------------------

/// Derive a container_id from a UUID (uses the high 64 bits).
pub fn container_id_from_uuid(id: uuid::Uuid) -> u64 {
    let (hi, _lo) = id.as_u64_pair();
    hi
}

/// Check if any items are selected.
pub fn has_selection() -> bool {
    let s = STATE.lock().unwrap();
    !s.selected.is_empty()
}

/// Get count of selected items in a specific container.
pub fn count_in_container(container_type: ContainerType, container_id: u64) -> usize {
    let s = STATE.lock().unwrap();
    s.selected
        .iter()
        .filter(|k| k.container_type == container_type && k.container_id == container_id)
        .count()
}

/// Get selected file paths from a container by mapping each selected index
/// through the provided closure.
///
/// Returns a Vec of (item_index, file_path) pairs. Items where the
/// closure returns None are silently skipped.
pub fn selected_paths_in_container<F>(
    container_type: ContainerType,
    container_id: u64,
    path_fn: F,
) -> Vec<(usize, String)>
where
    F: Fn(usize) -> Option<String>,
{
    let indices = selected_in_container(container_type, container_id);
    indices
        .into_iter()
        .filter_map(|idx| path_fn(idx).map(|p| (idx, p)))
        .collect()
}

// ---------------------------------------------------------------------------
// Marquee helpers
// ---------------------------------------------------------------------------

/// Get the current marquee container, if any.
pub fn marquee_container() -> Option<(ContainerType, u64)> {
    let s = STATE.lock().unwrap();
    s.marquee_container
}

/// Set the marquee container.
pub fn set_marquee_container(container: Option<(ContainerType, u64)>) {
    let mut s = STATE.lock().unwrap();
    s.marquee_container = container;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------


/// Select items in the visual rectangle between anchor and target indices.
/// For GridBox: uses grid_col/grid_row for positioned items, index-based for auto-layout.
/// Returns the list of selected keys.
pub fn set_visual_range(
    container_type: ContainerType,
    container_id: u64,
    item_positions: &[(i32, i32)], // (col, row) for each item by index
    anchor: usize,
    target: usize,
) -> Vec<SelectionKey> {
    let a = anchor.min(item_positions.len().saturating_sub(1));
    let t = target.min(item_positions.len().saturating_sub(1));
    if a >= item_positions.len() || t >= item_positions.len() {
        return Vec::new();
    }
    let (ac, ar) = item_positions[a];
    let (tc, tr) = item_positions[t];
    let min_c = ac.min(tc);
    let max_c = ac.max(tc);
    let min_r = ar.min(tr);
    let max_r = ar.max(tr);

    let mut s = STATE.lock().unwrap();
    s.selected.clear();
    let mut added = Vec::new();
    for (i, &(col, row)) in item_positions.iter().enumerate() {
        if col >= min_c && col <= max_c && row >= min_r && row <= max_r {
            let key = SelectionKey {
                container_type,
                container_id,
                item_index: i,
            };
            s.selected.insert(key);
            added.push(key);
        }
    }
    added
}

/// Add items in the visual rectangle to existing selection (Ctrl+Shift).
pub fn add_visual_range(
    container_type: ContainerType,
    container_id: u64,
    item_positions: &[(i32, i32)],
    anchor: usize,
    target: usize,
) -> Vec<SelectionKey> {
    let a = anchor.min(item_positions.len().saturating_sub(1));
    let t = target.min(item_positions.len().saturating_sub(1));
    if a >= item_positions.len() || t >= item_positions.len() {
        return Vec::new();
    }
    let (ac, ar) = item_positions[a];
    let (tc, tr) = item_positions[t];
    let min_c = ac.min(tc);
    let max_c = ac.max(tc);
    let min_r = ar.min(tr);
    let max_r = ar.max(tr);

    let mut s = STATE.lock().unwrap();
    let mut added = Vec::new();
    for (i, &(col, row)) in item_positions.iter().enumerate() {
        if col >= min_c && col <= max_c && row >= min_r && row <= max_r {
            let key = SelectionKey {
                container_type,
                container_id,
                item_index: i,
            };
            if s.selected.insert(key) {
                added.push(key);
            }
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_key(id: u64, idx: usize) -> SelectionKey {
        SelectionKey {
            container_type: ContainerType::GridBox,
            container_id: id,
            item_index: idx,
        }
    }

    fn overlay_key(idx: usize) -> SelectionKey {
        SelectionKey {
            container_type: ContainerType::Overlay,
            container_id: 0,
            item_index: idx,
        }
    }

    fn reset() {
        clear();
    }

    #[test]
    fn test_set_range_basic() {
        reset();
        let added = set_range(ContainerType::GridBox, 1, 2, 5);
        assert_eq!(added.len(), 4);
        assert_eq!(count(), 4);
        let sel = selected_in_container(ContainerType::GridBox, 1);
        assert_eq!(sel, vec![2, 3, 4, 5]);
    }

    #[test]
    fn test_set_range_reversed() {
        reset();
        let added = set_range(ContainerType::GridBox, 1, 5, 2);
        assert_eq!(added.len(), 4);
        assert_eq!(count(), 4);
        let sel = selected_in_container(ContainerType::GridBox, 1);
        assert_eq!(sel, vec![2, 3, 4, 5]);
    }

    #[test]
    fn test_set_range_clears_previous() {
        reset();
        add(grid_key(1, 10));
        assert_eq!(count(), 1);
        set_range(ContainerType::GridBox, 1, 0, 2);
        assert_eq!(count(), 3);
        let sel = selected_in_container(ContainerType::GridBox, 1);
        assert_eq!(sel, vec![0, 1, 2]);
    }

    #[test]
    fn test_add_range_preserves_existing() {
        reset();
        add(grid_key(1, 0));
        add(grid_key(1, 1));
        let added = add_range(ContainerType::GridBox, 1, 1, 3);
        assert_eq!(added.len(), 2);
        assert_eq!(count(), 4);
        let sel = selected_in_container(ContainerType::GridBox, 1);
        assert_eq!(sel, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_anchor_management() {
        reset();
        assert!(anchor().is_none());
        let k = grid_key(1, 3);
        set_anchor(Some(k));
        assert_eq!(anchor(), Some(k));
        set_anchor(None);
        assert!(anchor().is_none());
    }

    #[test]
    fn test_clear_resets_everything() {
        reset();
        add(grid_key(1, 0));
        add(grid_key(1, 1));
        add(overlay_key(0));
        set_anchor(Some(grid_key(1, 0)));
        set_marquee_container(Some((ContainerType::GridBox, 1)));
        assert_eq!(count(), 3);
        assert!(anchor().is_some());
        assert!(marquee_container().is_some());
        clear();
        assert_eq!(count(), 0);
        assert!(!has_selection());
        assert!(anchor().is_none());
        assert!(marquee_container().is_none());
        assert!(all_keys().is_empty());
    }

    #[test]
    fn test_container_id_from_uuid_consistency() {
        let id = uuid::Uuid::new_v4();
        let cid1 = container_id_from_uuid(id);
        let cid2 = container_id_from_uuid(id);
        assert_eq!(cid1, cid2);
        let id2 = uuid::Uuid::new_v4();
        let cid3 = container_id_from_uuid(id2);
        assert_ne!(cid1, cid3);
    }

    #[test]
    fn test_container_id_from_uuid_uses_high_bits() {
        let id = uuid::Uuid::from_u64_pair(0xDEAD_BEEF_CAFE_BABE, 0x1234_5678_9ABC_DEF0);
        assert_eq!(container_id_from_uuid(id), 0xDEAD_BEEF_CAFE_BABE);
    }

    #[test]
    fn test_has_selection() {
        reset();
        assert!(!has_selection());
        add(overlay_key(0));
        assert!(has_selection());
        clear();
        assert!(!has_selection());
    }

    #[test]
    fn test_count_in_container() {
        reset();
        add(grid_key(1, 0));
        add(grid_key(1, 1));
        add(grid_key(1, 2));
        add(grid_key(2, 0));
        assert_eq!(count_in_container(ContainerType::GridBox, 1), 3);
        assert_eq!(count_in_container(ContainerType::GridBox, 2), 1);
        assert_eq!(count_in_container(ContainerType::Overlay, 0), 0);
    }

    #[test]
    fn test_marquee_container() {
        reset();
        assert!(marquee_container().is_none());
        set_marquee_container(Some((ContainerType::GridBox, 42)));
        assert_eq!(marquee_container(), Some((ContainerType::GridBox, 42)));
        set_marquee_container(None);
        assert!(marquee_container().is_none());
    }

    #[test]
    fn test_duplicate_add_is_idempotent() {
        reset();
        let k = grid_key(1, 0);
        add(k);
        add(k);
        add(k);
        assert_eq!(count(), 1);
    }

    #[test]
    fn test_remove_nonexistent_is_safe() {
        reset();
        let k = grid_key(1, 99);
        remove(k);
        assert_eq!(count(), 0);
    }

    #[test]
    fn test_set_range_visual() {
        reset();
        let indices = vec![5, 2, 8, 1];
        let added = set_range_visual(ContainerType::GridBox, 1, &indices);
        assert_eq!(added.len(), 4);
        assert_eq!(count(), 4);
        let sel = selected_in_container(ContainerType::GridBox, 1);
        assert_eq!(sel, vec![1, 2, 5, 8]);
    }

    #[test]
    fn test_add_range_visual() {
        reset();
        add(grid_key(1, 0));
        add(grid_key(1, 3));
        let indices = vec![1, 3, 5];
        let added = add_range_visual(ContainerType::GridBox, 1, &indices);
        assert_eq!(added.len(), 2);
        assert_eq!(count(), 4);
        let sel = selected_in_container(ContainerType::GridBox, 1);
        assert_eq!(sel, vec![0, 1, 3, 5]);
    }
}

