//! Global multi-selection registry for managing selected icons
//! across multiple containers (classification boxes).
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
    /// A classification grid box.
    GridBox,
}

/// Unique key identifying a selectable item across all containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SelectionKey {
    /// Which kind of container holds this item.
    pub container_type: ContainerType,
    /// Container identifier. For GridBox this is derived from the
    /// config_id UUID (high 64 bits).
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
}

impl MultiSelectState {
    fn new() -> Self {
        Self {
            selected: BTreeSet::new(),
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

/// Clear all selections.
pub fn clear() {
    let mut s = STATE.lock().unwrap();
    s.selected.clear();
}

/// Add an item to the selection set.
pub fn add(key: SelectionKey) {
    let mut s = STATE.lock().unwrap();
    s.selected.insert(key);
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

// ---------------------------------------------------------------------------
// Public API - misc helpers
// ---------------------------------------------------------------------------

/// Derive a container_id from a UUID (uses the high 64 bits).
pub fn container_id_from_uuid(id: uuid::Uuid) -> u64 {
    let (hi, _lo) = id.as_u64_pair();
    hi
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 串行化锁：本模块所有测试共享全局 STATE，必须互斥执行，
    /// 否则并行 cargo test 下 reset() 会清掉其他测试的状态。
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn grid_key(id: u64, idx: usize) -> SelectionKey {
        SelectionKey {
            container_type: ContainerType::GridBox,
            container_id: id,
            item_index: idx,
        }
    }

    fn reset() {
        clear();
    }

    #[test]
    fn test_clear_resets_everything() {
        let _serial = serial();
        reset();
        add(grid_key(1, 0));
        add(grid_key(1, 1));
        add(grid_key(2, 5));
        assert_eq!(count(), 3);
        clear();
        assert_eq!(count(), 0);
        assert!(all_keys().is_empty());
    }

    #[test]
    fn test_container_id_from_uuid_consistency() {
        let _serial = serial();
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
        let _serial = serial();
        let id = uuid::Uuid::from_u64_pair(0xDEAD_BEEF_CAFE_BABE, 0x1234_5678_9ABC_DEF0);
        assert_eq!(container_id_from_uuid(id), 0xDEAD_BEEF_CAFE_BABE);
    }

    #[test]
    fn test_duplicate_add_is_idempotent() {
        let _serial = serial();
        reset();
        let k = grid_key(1, 0);
        add(k);
        add(k);
        add(k);
        assert_eq!(count(), 1);
    }
}
