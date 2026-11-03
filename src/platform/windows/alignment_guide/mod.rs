//! 对齐参考线模块
//!
//! 提供分类框移动/缩放时的对齐检测和参考线显示功能。

pub mod alignment_detect;
pub mod guide_overlay;

use std::sync::Mutex;

use alignment_detect::{
    detect_alignments_with_gap, DEFAULT_ALIGNMENT_THRESHOLD, MAX_PERPENDICULAR_GAP,
};
use guide_overlay::{
    create_guide_overlay, hide_guide_lines, is_guide_overlay_created, update_guide_lines,
};
use std::sync::atomic::{AtomicI32, Ordering};

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

/// 参与对齐检测的矩形：**优先取玻璃块矩形**。
///
/// 引擎模式下块几何由库逐帧驱动，代理窗口只是按宿主 tick 跟随（实测块矩形变化
/// 30~46 次/秒、tick 55~65 次/秒），拿 `GetWindowRect` 反推的内容矩形会稳定落后
/// 画面 1~2 拍：用户看到的就是「框还没贴上就冒出参考线」「贴上时线又慢半拍」。
/// 旧路径（引擎未激活 / 块未注册）保持窗口矩形口径不变。
fn alignment_rect(hwnd: *mut std::ffi::c_void) -> Option<RECT> {
    // 用代理窗口的内容矩形（物理像素），不用库块快照：会话期间库快照可能
    // 落后 1~2 拍（snapshot_dirty 不在几何变化时置位），拿快照检测 = 命中间歇
    // 性 → 参考线闪一帧就消失（2026-11-03 实测日志：8 次拖拽只命中 1 次）。
    // 窗口矩形由 follow_glass_session 每 tick 跟随块几何，响应更及时。
    grid_window::get_grid_content_rect(hwnd)
}

/// 关联间距上限（**物理像素**）。
///
/// [`MAX_PERPENDICULAR_GAP`] 是逻辑像素口径，而分类框坐标全是物理像素：
/// 200% 屏上不折算就等于只允许 120 逻辑像素的关联半径，用户那几个散布在
/// 3840 宽屏上的框会一条参考线都不出（2026-11-03 回归）。
fn perpendicular_gap_px(hwnd: *mut std::ffi::c_void) -> i32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    crate::platform::windows::ffi_types::scale_for_dpi(MAX_PERPENDICULAR_GAP, dpi)
}

/// 会话内「参考线首次命中」只记一条 info：这是用户报「参考线不出现」时
/// 唯一能区分「检测没命中」与「overlay 没画出来」的硬证据。
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
    let result = detect_alignments_with_gap(
        &current_rect,
        &others,
        DEFAULT_ALIGNMENT_THRESHOLD,
        perpendicular_gap_px(hwnd),
    );
    log_first_hits(result.horizontal_matches.len() + result.vertical_matches.len());
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
    let result = detect_alignments_with_gap(
        &content_rect,
        &others,
        DEFAULT_ALIGNMENT_THRESHOLD,
        perpendicular_gap_px(hwnd),
    );
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
        if let Some(rect) = alignment_rect(grid.hwnd) {
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
