//! 对齐参考线模块
//!
//! 提供分类框移动/缩放时的对齐检测和参考线显示功能。
//!
//! # 组件
//! - `alignment_detect`: 对齐检测算法（纯逻辑）
//! - `guide_overlay`: 参考线 overlay 窗口（D2D 渲染）

pub mod alignment_detect;
pub mod guide_overlay;

use std::sync::Mutex;

use alignment_detect::{detect_alignments, DEFAULT_ALIGNMENT_THRESHOLD};
use guide_overlay::{
    create_guide_overlay, hide_guide_lines, is_guide_overlay_created,
    update_guide_lines,
};

#[allow(unused_imports)]
pub use alignment_detect::{AlignmentDirection, AlignmentMatch, AlignmentResult};

use crate::platform::windows::ffi_types::RECT;
use crate::platform::windows::grid_window;

/// HWND wrapper that implements Send (HWND is thread-safe for our use case)
#[derive(Clone, Copy, Debug, PartialEq)]
struct SendHwnd(*mut std::ffi::c_void);
unsafe impl Send for SendHwnd {}

/// 当前正在移动/缩放的窗口 HWND
static MOVING_WINDOW: Mutex<Option<SendHwnd>> = Mutex::new(None);

/// 进入移动/缩放模式时调用
pub fn on_enter_size_move(hwnd: *mut std::ffi::c_void) {
    if let Ok(mut guard) = MOVING_WINDOW.lock() {
        *guard = Some(SendHwnd(hwnd));
    }
    if !is_guide_overlay_created() {
        if let Err(e) = create_guide_overlay() {
            log::error!("创建 guide overlay 失败: {}", e);
        }
    }
    log::debug!("进入移动/缩放模式");
}

/// 退出移动/缩放模式时调用
pub fn on_exit_size_move(_hwnd: *mut std::ffi::c_void) {
    if let Ok(mut guard) = MOVING_WINDOW.lock() {
        *guard = None;
    }
    hide_guide_lines();
    log::debug!("退出移动/缩放模式");
}

/// 移动/缩放中调用：检测对齐并更新参考线
pub fn on_moving(hwnd: *mut std::ffi::c_void, proposed_rect: &mut RECT) {
    if let Some(data) = unsafe { grid_window::get_data(hwnd) } {
        if data.locked {
            hide_guide_lines();
            return;
        }
    }
    let others = collect_other_box_rects(hwnd);
    let result = detect_alignments(proposed_rect, &others, DEFAULT_ALIGNMENT_THRESHOLD);
    let (work_x, work_y, _, _) =
        crate::platform::windows::monitor_utils::get_virtual_desktop_bounds();
    update_guide_lines(&result, (work_x, work_y));
}

fn collect_other_box_rects(exclude_hwnd: *mut std::ffi::c_void) -> Vec<(*mut std::ffi::c_void, RECT)> {
    let grids = match grid_window::get_grid_list() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let mut result = Vec::new();
    for grid in grids.iter() {
        if grid.hwnd == exclude_hwnd {
            continue;
        }
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe {
            crate::platform::windows::ffi_imports::GetWindowRect(grid.hwnd, &mut rect);
        }
        result.push((grid.hwnd, rect));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_moving_window_tracking() {
        let fake_hwnd = 0x12345 as *mut std::ffi::c_void;
        on_enter_size_move(fake_hwnd);
        {
            let guard = MOVING_WINDOW.lock().unwrap();
            assert_eq!(guard.map(|h| h.0), Some(fake_hwnd));
        }
        on_exit_size_move(fake_hwnd);
        {
            let guard = MOVING_WINDOW.lock().unwrap();
            assert_eq!(*guard, None);
        }
    }
}