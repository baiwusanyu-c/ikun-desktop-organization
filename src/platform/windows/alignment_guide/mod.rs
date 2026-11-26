//! Alignment guide module.
//!
//! The guide overlay and its timing state are UI-thread-only. All entry points,
//! including timer callbacks and the delayed WM_USER_DESTROY_GUIDE handler,
//! must run on the thread that owns the overlay window.

pub mod alignment_detect;
pub mod guide_overlay;

use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use alignment_detect::{alignment_threshold_for_dpi, detect_alignments};
use guide_overlay::{
    create_guide_overlay, hide_guide_lines, reset_guide_render_state, update_guide_lines,
};

pub use guide_overlay::destroy_guide_overlay;

use crate::platform::windows::ffi_imports::{GetDpiForWindow, KillTimer, PostMessageW, SetTimer};
use crate::platform::windows::ffi_types::{RECT, TIMER_ID_ALIGNMENT, WM_USER_DESTROY_GUIDE};
use crate::platform::windows::grid_window;

static SESSION_GEN: AtomicU64 = AtomicU64::new(0);

/// 进入移动/缩放模式：启动定时器轮询窗口位置
pub fn on_enter_size_move(hwnd: *mut std::ffi::c_void) {
    SESSION_GEN.fetch_add(1, Ordering::SeqCst);
    reset_guide_render_state();

    let no_guide = std::env::var("IKUN_NO_GUIDE")
        .map(|v| v == "1")
        .unwrap_or(false);
    // 每次进入会话都调：内部幂等，但显示器布局变了会自动重建每屏 overlay。
    // 否则热插拔 / 改分辨率后新屏上没有窗口，跨屏对齐就看不到参考线。
    if !no_guide {
        if let Err(e) = create_guide_overlay() {
            log::error!("create guide overlay failed: {}", e);
        }
    }
    unsafe { SetTimer(hwnd, TIMER_ID_ALIGNMENT, 16, std::ptr::null_mut()) };
    log::debug!("enter size/move, timer started");
}

pub fn on_exit_size_move(hwnd: *mut std::ffi::c_void) {
    let request_gen = SESSION_GEN.load(Ordering::SeqCst);
    reset_guide_render_state();

    unsafe { KillTimer(hwnd, TIMER_ID_ALIGNMENT) };
    hide_guide_lines();
    log::debug!("exit size/move, timer stopped");
    unsafe {
        PostMessageW(hwnd, WM_USER_DESTROY_GUIDE, request_gen as usize, 0);
    }
}

fn should_destroy_guide(request_gen: usize, current_gen: u64) -> bool {
    request_gen as u64 == current_gen
}

/// Destroy the overlay only if the delayed message belongs to the current session.
pub fn handle_destroy_guide(request_gen: usize) {
    let current_gen = SESSION_GEN.load(Ordering::SeqCst);
    if should_destroy_guide(request_gen, current_gen) {
        destroy_guide_overlay();
    } else {
        log::debug!(
            "ignore stale guide destroy request: request_gen={}, current_gen={}",
            request_gen,
            current_gen
        );
    }
}

/// Return the current content rectangle used for alignment detection.
fn alignment_rect(hwnd: *mut std::ffi::c_void) -> Option<RECT> {
    // 用代理窗口的内容矩形（物理像素），不用库块快照：会话期间库快照可能
    // 落后 1~2 拍（snapshot_dirty 不在几何变化时置位），拿快照检测 = 命中间歇
    // 性 → 参考线闪一帧就消失（2026-11-03 实测日志：8 次拖拽只命中 1 次）。
    // 窗口矩形由 follow_glass_session 每 tick 跟随块几何，响应更及时。
    grid_window::get_grid_content_rect(hwnd)
}

fn log_first_hits(count: usize) {
    static LAST: AtomicI32 = AtomicI32::new(-1);
    let prev = LAST.swap(count as i32, Ordering::Relaxed);
    if prev == 0 && count > 0 {
        log::info!("对齐参考线命中: {count} 条");
    }
}

/// Timer 回调：检测对齐并更新参考线
pub fn on_alignment_timer(hwnd: *mut std::ffi::c_void) {
    let Some(current_rect) = alignment_rect(hwnd) else {
        return;
    };

    if let Some(data) = unsafe { grid_window::get_data(hwnd) } {
        if data.locked {
            hide_guide_lines();
            return;
        }
    }

    let others = collect_other_box_rects(hwnd);
    // 阈值按**被拖窗口的 DPI** 折算：DEFAULT 是逻辑像素，直接用 5 物理像素时
    // 主屏(192) 的容差只有副屏(96) 的一半，跨屏对齐手感不一致。
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let result = detect_alignments(&current_rect, &others, alignment_threshold_for_dpi(dpi));
    log_first_hits(result.horizontal_matches.len() + result.vertical_matches.len());
    update_guide_lines(&result);
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
    let result = detect_alignments(&content_rect, &others, alignment_threshold_for_dpi(dpi));
    update_guide_lines(&result);
}

fn collect_other_box_rects(exclude_hwnd: *mut std::ffi::c_void) -> Vec<RECT> {
    let grids = match grid_window::get_grid_list() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let mut result = Vec::new();
    for grid in grids.iter() {
        if grid.hwnd == exclude_hwnd {
            continue;
        }
        if let Some(rect) = alignment_rect(grid.hwnd) {
            result.push(rect);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_destroy_guide_rejects_stale_generation() {
        assert!(should_destroy_guide(8, 8));
        assert!(!should_destroy_guide(7, 8));
    }
}
