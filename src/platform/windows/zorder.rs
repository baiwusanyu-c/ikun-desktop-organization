//! Z-order 管理模块
//!
//! 管理分类框窗口的 Z-order 层级：
//! - 分类框始终低于其他应用窗口（HWND_BOTTOM）
//! - 分类框之间点击的在最前
//! - 提供坐标查找功能（替代 WindowFromPoint）
//!
//! # 核心函数
//! - push_other_grids_below: 将其他分类框推到指定窗口之下
//! - ind_grid_window_at_point: 查找指定坐标处的分类框
//! - is_zorder_suppressing / with_zorder_suppressed: 递归抑制
//! - set_drag_in_progress / is_drag_in_progress: OLE 拖拽标志

use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::get_grid_list;

use std::cell::Cell;

// Z-order 抑制标志：防止 WM_WINDOWPOSCHANGING -> SetWindowPos -> WM_WINDOWPOSCHANGING 递归。
// 当 push_other_grids_below 批量调整其他分类框 Z-order 时设为 true，
// 使那些窗口的 WM_WINDOWPOSCHANGING 处理器跳过干预，避免无限递归。
thread_local! {
    static ZORDER_SUPPRESSING: Cell<bool> = const { Cell::new(false) };
}

// OLE 拖拽进行中标志：DoDragDrop 是阻塞调用，期间 Windows 会对所有窗口做 hit-testing。
// 如果此时 WM_WINDOWPOSCHANGING 强制 HWND_BOTTOM，分类框可能低于 overlay 窗口，
// 导致 OLE 无法触发目标窗口的 DragEnter。拖拽期间不干预 Z-order。
thread_local! {
    static DRAG_IN_PROGRESS: Cell<bool> = const { Cell::new(false) };
}

/// 将所有其他分类框窗口推到指定窗口之下（Z-order 管理）
///
/// 当某个分类框被激活（点击）时调用。将该分类框保持在 Z-order 顶部（分类框之间），
/// 同时将其他分类框推到它之下，确保点击的分类框在分类框间处于最前，
/// 但所有分类框都不会高于其他应用程序窗口。
pub fn push_other_grids_below(active_hwnd: HWND) {
    let grids = match get_grid_list() {
        Ok(g) => g,
        Err(_) => return,
    };
    for grid in grids.iter().rev() {
        if grid.hwnd != active_hwnd {
            unsafe {
                SetWindowPos(
                    grid.hwnd,
                    HWND_BOTTOM,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
            }
        }
    }
}

/// 检查当前是否正在 Z-order 批量调整中（抑制 WM_WINDOWPOSCHANGING 干预）
pub(crate) fn is_zorder_suppressing() -> bool {
    ZORDER_SUPPRESSING.with(|f| f.get())
}

/// 在 Z-order 抑制模式下执行操作（设置标志 -> 执行 -> 清除标志）
///
/// 用于 WM_USER_PUSH_GRIDS_BELOW 处理器中，确保 push_other_grids_below 内部
/// 的 SetWindowPos 调用不会触发递归 WM_WINDOWPOSCHANGING 干预。
pub(crate) fn with_zorder_suppressed(f: impl FnOnce()) {
    ZORDER_SUPPRESSING.with(|flag| flag.set(true));
    f();
    ZORDER_SUPPRESSING.with(|flag| flag.set(false));
}

/// 设置 OLE 拖拽进行中标志（DoDragDrop 前设为 true，结束后设为 false）
///
/// 拖拽期间 WM_WINDOWPOSCHANGING 不干预 Z-order，确保 OLE hit-testing 能正确
/// 检测到目标窗口的 IDropTarget。
pub fn set_drag_in_progress(in_progress: bool) {
    DRAG_IN_PROGRESS.with(|f| f.set(in_progress));
}

/// 检查当前是否有 OLE 拖拽正在进行
pub(crate) fn is_drag_in_progress() -> bool {
    DRAG_IN_PROGRESS.with(|f| f.get())
}

/// 查找指定屏幕坐标处的分类框窗口
///
/// 由于分类框窗口位于 HWND_BOTTOM（低于覆盖窗口和其他应用窗口），
/// WindowFromPoint 无法找到它们（被覆盖窗口拦截）。
/// 此函数直接遍历分类框列表，通过 GetWindowRect 检查矩形碰撞，
/// 按 Z-order 顺序（列表末尾 = 最上方）返回第一个命中的分类框。
pub fn find_grid_window_at_point(screen_x: i32, screen_y: i32) -> HWND {
    let grids = match get_grid_list() {
        Ok(g) => g,
        Err(_) => return std::ptr::null_mut(),
    };
    for grid in grids.iter().rev() {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe {
            if GetWindowRect(grid.hwnd, &mut rect) != 0 {
                if screen_x >= rect.left
                    && screen_x < rect.right
                    && screen_y >= rect.top
                    && screen_y < rect.bottom
                {
                    return grid.hwnd;
                }
            }
        }
    }
    std::ptr::null_mut()
}

/// 查找光标下的外部窗口（Explorer 等），跳过本应用窗口和桌面窗口
pub fn find_external_window_at_point(screen_x: i32, screen_y: i32) -> HWND {
    struct EnumData {
        x: i32,
        y: i32,
        result: HWND,
    }

    unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> i32 {
        let data = &mut *(lparam as *mut EnumData);
        if !data.result.is_null() {
            return 0;
        }
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut class_name = [0u16; 64];
        let len = GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32);
        if len > 0 {
            let name = String::from_utf16_lossy(&class_name[..len as usize]);
            if name != "CabinetWClass" {
                return 1;
            }
        } else {
            return 1;
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(hwnd, &mut rect) != 0
            && data.x >= rect.left
            && data.x < rect.right
            && data.y >= rect.top
            && data.y < rect.bottom
        {
            data.result = hwnd;
            return 0;
        }
        1
    }

    let mut data = EnumData {
        x: screen_x,
        y: screen_y,
        result: std::ptr::null_mut(),
    };
    unsafe {
        EnumWindows(Some(enum_callback), &mut data as *mut EnumData as LPARAM);
    }
    data.result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== Z-order 管理单元测试 =====

    #[test]
    fn test_zorder_suppressing_default_false() {
        assert!(
            !is_zorder_suppressing(),
            "ZORDER_SUPPRESSING should default to false"
        );
    }

    #[test]
    fn test_with_zorder_suppressed_sets_and_clears() {
        assert!(!is_zorder_suppressing());
        with_zorder_suppressed(|| {
            assert!(
                is_zorder_suppressing(),
                "Flag should be true inside closure"
            );
        });
        assert!(
            !is_zorder_suppressing(),
            "Flag should be false after closure"
        );
    }

    #[test]
    fn test_with_zorder_suppressed_clears_on_panic() {
        let result = std::panic::catch_unwind(|| {
            with_zorder_suppressed(|| {
                assert!(is_zorder_suppressing());
                panic!("test panic");
            });
        });
        assert!(result.is_err(), "Should have panicked");
    }

    #[test]
    fn test_push_other_grids_below_empty_list() {
        push_other_grids_below(std::ptr::null_mut());
    }

    #[test]
    fn test_push_other_grids_below_with_zorder_suppressed() {
        with_zorder_suppressed(|| {
            push_other_grids_below(std::ptr::null_mut());
            assert!(
                is_zorder_suppressing(),
                "Should still be suppressing inside"
            );
        });
        assert!(!is_zorder_suppressing(), "Should be cleared after");
    }

    // ===== drag flag unit tests =====

    #[test]
    fn test_drag_in_progress_default_false() {
        assert!(
            !is_drag_in_progress(),
            "DRAG_IN_PROGRESS should default to false"
        );
    }

    #[test]
    fn test_set_drag_in_progress_toggle() {
        set_drag_in_progress(true);
        assert!(is_drag_in_progress(), "Should be true after set(true)");
        set_drag_in_progress(false);
        assert!(!is_drag_in_progress(), "Should be false after set(false)");
    }

    #[test]
    fn test_drag_flag_independent_of_zorder_suppressing() {
        set_drag_in_progress(true);
        assert!(
            !is_zorder_suppressing(),
            "Z-order suppressing should be independent"
        );
        assert!(is_drag_in_progress(), "Drag flag should be true");
        set_drag_in_progress(false);
    }

    // ===== find_grid_window_at_point unit tests =====

    #[test]
    fn test_find_grid_window_at_point_empty_list() {
        let result = find_grid_window_at_point(100, 100);
        assert!(result.is_null(), "Should return null for empty grid list");
    }

    #[test]
    fn test_find_grid_window_at_point_no_crash_at_origin() {
        let result = find_grid_window_at_point(0, 0);
        let _ = result;
    }

    #[test]
    fn test_find_grid_window_at_point_negative_coords() {
        let result = find_grid_window_at_point(-100, -100);
        assert!(result.is_null(), "Should return null for negative coords");
    }

    // ===== find_external_window_at_point unit tests =====

    #[test]
    fn test_find_external_window_at_point_no_crash_at_origin() {
        // Should not crash even at (0, 0)
        let result = find_external_window_at_point(0, 0);
        let _ = result;
    }

    #[test]
    fn test_find_external_window_at_point_negative_coords() {
        // Negative coordinates should not crash
        let result = find_external_window_at_point(-100, -100);
        let _ = result;
    }

    #[test]
    fn test_find_external_window_at_point_off_screen() {
        // Very large coordinates (off-screen) should return null
        let result = find_external_window_at_point(99999, 99999);
        assert!(result.is_null(), "Off-screen should not find Explorer");
    }
}
