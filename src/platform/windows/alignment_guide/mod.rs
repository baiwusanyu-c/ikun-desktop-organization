//! 对齐参考线模块
//!
//! 提供分类框移动/缩放时的对齐检测和参考线显示功能。

pub mod alignment_detect;
pub mod guide_overlay;

use std::sync::Mutex;

use alignment_detect::{detect_alignments, DEFAULT_ALIGNMENT_THRESHOLD};
use guide_overlay::{
    create_guide_overlay, hide_guide_lines, is_guide_overlay_created, update_guide_lines,
};

pub use guide_overlay::destroy_guide_overlay;

#[allow(unused_imports)]
pub use alignment_detect::{AlignmentDirection, AlignmentMatch, AlignmentResult};

use crate::platform::windows::ffi_imports::{GetDpiForWindow, KillTimer, PostMessageW, SetTimer};
use crate::platform::windows::ffi_types::{RECT, TIMER_ID_ALIGNMENT, WM_USER_DESTROY_GUIDE};
use crate::platform::windows::grid_window;

#[derive(Clone, Copy, Debug, PartialEq)]
struct SendHwnd(*mut std::ffi::c_void);
unsafe impl Send for SendHwnd {}

static MOVING_WINDOW: Mutex<Option<SendHwnd>> = Mutex::new(None);

/// 进入移动/缩放模式：启动定时器轮询窗口位置
pub fn on_enter_size_move(hwnd: *mut std::ffi::c_void) {
    if let Ok(mut guard) = MOVING_WINDOW.lock() {
        *guard = Some(SendHwnd(hwnd));
    }
    let no_guide = std::env::var("IKUN_NO_GUIDE")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !no_guide && !is_guide_overlay_created() {
        if let Err(e) = create_guide_overlay() {
            log::error!("create guide overlay failed: {}", e);
        }
        // Guide color is pre-sampled at app startup via init_guide_color()
    }
    // Start timer to poll window position every 16ms (~60fps)
    unsafe { SetTimer(hwnd, TIMER_ID_ALIGNMENT, 16, std::ptr::null_mut()) };
    log::debug!("enter size/move, timer started");
}

/// 退出移动/缩放模式：停止定时器，隐藏参考线
pub fn on_exit_size_move(hwnd: *mut std::ffi::c_void) {
    if let Ok(mut guard) = MOVING_WINDOW.lock() {
        *guard = None;
    }
    unsafe { KillTimer(hwnd, TIMER_ID_ALIGNMENT) };
    hide_guide_lines();
    log::debug!("exit size/move, timer stopped");
    // 参考线只在移动/缩放期间有用：退出时销毁全屏 overlay
    // （3120x2080 分层窗 + 全屏 DIB 约 50MB+），否则首次拖拽后常驻进程一生
    // （2026-09-14 内存台阶根因）。下次进入移动时 on_enter_size_move 重建。
    // 同步 DestroyWindow 在模态移动退出路径上有重入风险（实测卡死 tick），
    // 改为 PostMessage 延迟到消息循环空闲时销毁。
    unsafe {
        PostMessageW(hwnd, WM_USER_DESTROY_GUIDE, 0, 0);
    }
}

/// Timer 回调：检测对齐并更新参考线
pub fn on_alignment_timer(hwnd: *mut std::ffi::c_void) {
    let Some(current_rect) = grid_window::get_grid_content_rect(hwnd) else {
        return;
    };

    if let Some(data) = unsafe { grid_window::get_data(hwnd) } {
        if data.locked {
            hide_guide_lines();
            return;
        }
    }

    let others = collect_other_box_rects(hwnd);
    let result = detect_alignments(&current_rect, &others, DEFAULT_ALIGNMENT_THRESHOLD);
    let (work_x, work_y, _, _) =
        crate::platform::windows::monitor_utils::get_virtual_desktop_bounds();
    update_guide_lines(&result, (work_x, work_y));
}

/// WM_MOVING/WM_SIZING 中也触发（兼容，双保险）
pub fn on_moving(hwnd: *mut std::ffi::c_void, proposed_rect: &mut RECT) {
    if let Some(data) = unsafe { grid_window::get_data(hwnd) } {
        if data.locked {
            hide_guide_lines();
            return;
        }
    }
    let others = collect_other_box_rects(hwnd);
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    // 留白取窗口当前生效值（而不是按 DPI 重新推算），
    // 保证对齐参考线看到的内容矩形与分类框自己的命中测试一致。
    let content_rect = grid_window::content_rect_with_padding(
        RECT {
            left: proposed_rect.left,
            top: proposed_rect.top,
            right: proposed_rect.right,
            bottom: proposed_rect.bottom,
        },
        grid_window::window_padding(hwnd, dpi),
    );
    let result = detect_alignments(&content_rect, &others, DEFAULT_ALIGNMENT_THRESHOLD);
    let (work_x, work_y, _, _) =
        crate::platform::windows::monitor_utils::get_virtual_desktop_bounds();
    update_guide_lines(&result, (work_x, work_y));
}

fn collect_other_box_rects(
    exclude_hwnd: *mut std::ffi::c_void,
) -> Vec<(*mut std::ffi::c_void, RECT)> {
    let grids = match grid_window::get_grid_list() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let mut result = Vec::new();
    for grid in grids.iter() {
        if grid.hwnd == exclude_hwnd {
            continue;
        }
        if let Some(rect) = grid_window::get_grid_content_rect(grid.hwnd) {
            result.push((grid.hwnd, rect));
        }
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
