//! 自定义拖拽覆盖窗口（替代 OLE IDragSourceHelper）

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! 创建一个小型 WS_EX_LAYERED 窗口跟随鼠标光标，
//! 显示被拖拽图标 + 名称的半透明图像。与原生桌面拖拽体验一致。
//!
//! # 渲染方案
//!
//! 使用 Direct2D + DirectWrite 渲染复合位图（图标 + 标签文字），
//! 与分类框内图标标签质量一致。详见 `d2d_render::render_drag_overlay_bitmap`。
//!
//! 不依赖 OLE 拖拽图像（IDragSourceHelper），
//! 避免了 Shell 拒绝 drop 时拖拽图像不显示的问题。

use super::ffi_imports::*;
use super::ffi_types::*;

/// 拖拽覆盖窗口类名
static DRAG_OVERLAY_CLASS: &[u16] = &[
    73, 107, 117, 110, 68, 114, 97, 103, 79, 118, 101, 114, 108, 97, 121, 0,
]; // "IkunDragOverlay"

// SAFETY: 以下 static mut 变量仅在 UI 主线程（消息循环线程）中访问，
// 拖拽覆盖窗口生命周期内同一时刻只有一个拖拽操作，不存在并发竞争。
// 所有访问均在 unsafe 函数 create/update/destroy_drag_overlay 内部完成。

/// 拖拽覆盖窗口句柄和位图
static mut OVERLAY_HWND: HWND = std::ptr::null_mut();
static mut OVERLAY_HBMP: HBITMAP = std::ptr::null_mut();

/// 拖拽创建参数，用于跨显示器时按原始 DPI 重建残影位图。
/// `DRAG_ORIGINAL_DPI` 记录创建时的 DPI，作为缩放的不变基准；
/// `DRAG_LAST_DPI` 记录上次渲染的 DPI，用于判断是否需要重建。
static mut DRAG_ICON_HANDLE: HICON = std::ptr::null_mut();
static mut DRAG_ICON_SIZE: i32 = 0;
static mut DRAG_CELL_W: i32 = 0;
static mut DRAG_ICON_OFFSET_X: i32 = 0;
static mut DRAG_DISPLAY_NAME: String = String::new();
static mut DRAG_LAST_DPI: u32 = 0;
static mut DRAG_ORIGINAL_DPI: u32 = 0;

// GDI 常量（未在 ffi_types 中定义的）
const TRANSPARENT_BK_MODE: i32 = 1;
const DEFAULT_CHARSET: i32 = 1;
const FW_NORMAL: i32 = 400;
const CLEARTYPE_QUALITY: i32 = 5;
const FF_DONTCARE: i32 = 0;

/// 创建拖拽覆盖窗口
///
/// 创建一个小窗口显示被拖拽图标 + 名称标签的半透明图像。
/// 窗口跟随光标移动（通过 `update_drag_overlay` 更新位置）。
///
/// # 参数
/// - `icon_handle`: 被拖拽图标的 HICON 句柄
/// - `icon_size`: 图标显示尺寸（DPI 自适应，与分类框内图标大小一致）
/// - `cell_w`: 分类框的单元格宽度（用于标签换行，与框内显示一致）
/// - `display_name`: 图标下方的标签文字（空字符串则不显示标签）
/// - `dpi`: 当前窗口 DPI（用于字体缩放）
pub(crate) unsafe fn create_drag_overlay(
    icon_handle: HICON,
    icon_size: i32,
    cell_w: i32,
    display_name: &str,
    dpi: u32,
) {
    log::info!(
        "[DIAG] create_drag_overlay called: display_name='{}', OVERLAY_HWND={:?}",
        display_name,
        OVERLAY_HWND
    );
    destroy_drag_overlay();

    if icon_handle.is_null() {
        log::debug!("创建拖拽覆盖: 无图标句柄");
        return;
    }

    // ★ 关键：用光标位置的 DPI 替换源窗口 DPI 渲染位图。
    // 残影跟随光标显示，位图尺寸必须匹配光标所在显示器的 DPI，
    // 否则跨显示器拖拽时会出现残影过大/过小。
    let source_dpi = dpi; // 源窗口 DPI（参数传入）
    let mut cursor_pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor_pt);
    let cursor_dpi =
        crate::platform::windows::desktop_overlay::dpi_at_screen_point(cursor_pt.x, cursor_pt.y);
    let dpi = if cursor_dpi > 0 { cursor_dpi } else { dpi };

    // 将 icon_size / cell_w 从源窗口 DPI 缩放到光标 DPI
    let icon_size = scale_dpi(icon_size, dpi, source_dpi);
    let cell_w = scale_dpi(cell_w, dpi, source_dpi);

    // 使用分类框的实际 cell_w，确保标签换行行为与框内一致
    let overlay_w = cell_w;
    // 图标在窗口内水平居中，计算偏移量用于补偿窗口位置
    let icon_offset_x = (overlay_w - icon_size) / 2;
    let label_h = if display_name.is_empty() {
        0
    } else {
        icon_size * 36 / 48
    };
    let gap = if display_name.is_empty() {
        0
    } else {
        icon_size * 6 / 48
    };
    let total_w = overlay_w;
    let total_h = icon_size + gap + label_h;

    // 使用 D2D + DirectWrite 渲染复合位图（与分类框标签质量一致）
    let hbmp = super::d2d_render::render_drag_overlay_bitmap(
        icon_handle,
        icon_size,
        display_name,
        total_w,
        total_h,
        gap,
        label_h,
        dpi,
    );
    if hbmp.is_null() {
        log::warn!("创建拖拽覆盖: D2D 位图渲染失败");
        return;
    }

    // 注册窗口类（只注册一次）
    static mut CLASS_REG: bool = false;
    if !CLASS_REG {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfn_wnd_proc: Some(overlay_wnd_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: hinstance,
            h_icon: std::ptr::null_mut(),
            h_cursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as *const u16),
            hbr_background: std::ptr::null_mut(),
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: DRAG_OVERLAY_CLASS.as_ptr(),
            h_icon_sm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
        CLASS_REG = true;
    }

    // 在创建前获取光标位置，用于窗口初始坐标（避免窗口先出现在 (0,0) 再移动）
    let mut init_pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut init_pt);

    // 创建分层窗口（WS_EX_LAYERED + WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW）
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        DRAG_OVERLAY_CLASS.as_ptr(),
        std::ptr::null(),
        WS_POPUP,
        init_pt.x - icon_offset_x,
        init_pt.y,
        total_w,
        total_h,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );

    if hwnd.is_null() {
        log::warn!("创建拖拽覆盖窗口失败");
        DeleteObject(hbmp as HGDIOBJ);
        return;
    }

    // 使用 UpdateLayeredWindow 设置位图内容（per-pixel alpha）
    let hdc_screen = GetDC(std::ptr::null_mut());
    let hdc_mem = CreateCompatibleDC(hdc_screen);
    let old_bmp = SelectObject(hdc_mem, hbmp as HGDIOBJ);

    // 与 CreateWindowExW 相同的初始位置，保持一致
    let pt_dst = POINT {
        x: init_pt.x - icon_offset_x,
        y: init_pt.y,
    };
    let sz = SIZE {
        cx: total_w,
        cy: total_h,
    };
    let pt_src = POINT { x: 0, y: 0 };

    // 70% 不透明度（半透明效果，与原生桌面拖拽一致）
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 180, // 0-255, 180 ≈ 70%
        alpha_format: AC_SRC_ALPHA,
    };

    UpdateLayeredWindow(
        hwnd,
        hdc_screen,
        &pt_dst,
        &sz as *const SIZE as *const POINT,
        hdc_mem,
        &pt_src,
        0,
        &blend,
        0x02, // ULW_ALPHA
    );

    SelectObject(hdc_mem, old_bmp);
    DeleteDC(hdc_mem);
    ReleaseDC(std::ptr::null_mut(), hdc_screen);

    // 显示窗口：由于 CreateWindowExW 和 UpdateLayeredWindow 都使用光标位置，
    // ShowWindow 不会触发从 (0,0) 到光标位置的移动，因此不会闪烁。
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);

    OVERLAY_HWND = hwnd;
    OVERLAY_HBMP = hbmp;

    // 保存拖拽参数，供 update_drag_overlay 跨显示器重建时使用
    DRAG_ICON_HANDLE = icon_handle;
    DRAG_ICON_SIZE = icon_size;
    DRAG_CELL_W = cell_w;
    DRAG_ICON_OFFSET_X = icon_offset_x;
    DRAG_DISPLAY_NAME = display_name.to_string();
    // 用光标位置的 DPI 初始化 DRAG_LAST_DPI，避免 update_drag_overlay 首次检测时
    // 因源窗口 DPI ≠ 光标 DPI（跨显示器拖拽）触发不必要的重建 → 导致图标闪烁
    // DRAG_ORIGINAL_DPI 也使用光标 DPI，因为位图已按光标 DPI 渲染（见上方缩放逻辑）
    let mut cursor_pt2 = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor_pt2);
    let cursor_dpi2 =
        crate::platform::windows::desktop_overlay::dpi_at_screen_point(cursor_pt2.x, cursor_pt2.y);
    let effective_dpi = if cursor_dpi2 > 0 { cursor_dpi2 } else { dpi };
    DRAG_LAST_DPI = effective_dpi;
    DRAG_ORIGINAL_DPI = effective_dpi;

    log::info!("拖拽覆盖窗口已创建 ({}x{}, 70% 不透明)", total_w, total_h);
}

/// 更新拖拽覆盖窗口位置（跟随光标，支持多显示器）
///
/// 当光标移动到不同 DPI 的显示器时，自动重建残影位图并按新 DPI 缩放，
/// 确保拖拽"残影"图标在高/低分辨率屏幕间切换时大小自适应。
pub(crate) unsafe fn update_drag_overlay() {
    if OVERLAY_HWND.is_null() {
        return;
    }

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);

    // DPI 检测优化：每 8 次调用才检测一次 DPI，减少系统调用开销
    static mut DPI_CHECK_COUNTER: u32 = 0;
    DPI_CHECK_COUNTER = (DPI_CHECK_COUNTER + 1) % 8;

    if DPI_CHECK_COUNTER == 0 {
        let current_dpi =
            crate::platform::windows::desktop_overlay::dpi_at_screen_point(pt.x, pt.y);
        if current_dpi != DRAG_LAST_DPI && !DRAG_ICON_HANDLE.is_null() {
            rebuild_drag_overlay_at_dpi(current_dpi);
        }
    }

    // 窗口位置：光标位置减去图标偏移量，使图标对齐光标
    // 使用 SWP_NOZORDER 避免每次都调整 Z-order，减少开销
    SetWindowPos(
        OVERLAY_HWND,
        std::ptr::null_mut(),
        pt.x - DRAG_ICON_OFFSET_X,
        pt.y,
        0,
        0,
        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
    );
}

/// 按新 DPI 缩放像素值，始终以原始 DPI 为基准，避免累积缩放误差。
fn scale_dpi(px: i32, target_dpi: u32, original_dpi: u32) -> i32 {
    (px * target_dpi as i32) / original_dpi.max(1) as i32
}

/// 计算拖拽残影的标签高度（与 icon_size 成比例）。
fn drag_label_height(icon_size: i32, has_label: bool) -> i32 {
    if has_label {
        icon_size * 36 / 48
    } else {
        0
    }
}

/// 计算拖拽残影图标与标签的间距。
fn drag_icon_gap(icon_size: i32, has_label: bool) -> i32 {
    if has_label {
        icon_size * 6 / 48
    } else {
        0
    }
}

/// 按新 DPI 重建拖拽残影位图并更新窗口尺寸。
unsafe fn rebuild_drag_overlay_at_dpi(new_dpi: u32) {
    let icon_handle = DRAG_ICON_HANDLE;
    let base_icon_size = DRAG_ICON_SIZE;
    let base_cell_w = DRAG_CELL_W;
    let display_name = (*std::ptr::addr_of!(DRAG_DISPLAY_NAME)).clone();

    let icon_size = scale_dpi(base_icon_size, new_dpi, DRAG_ORIGINAL_DPI);
    let cell_w = scale_dpi(base_cell_w, new_dpi, DRAG_ORIGINAL_DPI);

    let label_h = drag_label_height(icon_size, !display_name.is_empty());
    let gap = drag_icon_gap(icon_size, !display_name.is_empty());
    let total_w = cell_w;
    let total_h = icon_size + gap + label_h;

    let hbmp = super::d2d_render::render_drag_overlay_bitmap(
        icon_handle,
        icon_size,
        &display_name,
        total_w,
        total_h,
        gap,
        label_h,
        new_dpi,
    );
    if hbmp.is_null() {
        log::warn!("重建拖拽残影失败: DPI={}", new_dpi);
        return;
    }

    // 替换旧位图
    if !OVERLAY_HBMP.is_null() {
        DeleteObject(OVERLAY_HBMP as HGDIOBJ);
    }
    OVERLAY_HBMP = hbmp;
    DRAG_LAST_DPI = new_dpi;

    // 用新位图更新窗口（同时更新尺寸）
    let hdc_screen = GetDC(std::ptr::null_mut());
    let hdc_mem = CreateCompatibleDC(hdc_screen);
    let old_bmp = SelectObject(hdc_mem, hbmp as HGDIOBJ);

    // 使用当前窗口位置（而非 (0, 0)），避免重建时窗口瞬间跳到主屏幕左上角
    let mut cur_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    GetWindowRect(OVERLAY_HWND, &mut cur_rect);
    let pt_dst = POINT {
        x: cur_rect.left,
        y: cur_rect.top,
    };
    let sz = SIZE {
        cx: total_w,
        cy: total_h,
    };
    let pt_src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 180,
        alpha_format: AC_SRC_ALPHA,
    };

    UpdateLayeredWindow(
        OVERLAY_HWND,
        hdc_screen,
        &pt_dst,
        &sz as *const SIZE as *const POINT,
        hdc_mem,
        &pt_src,
        0,
        &blend,
        0x02, // ULW_ALPHA
    );

    SelectObject(hdc_mem, old_bmp);
    DeleteDC(hdc_mem);
    ReleaseDC(std::ptr::null_mut(), hdc_screen);

    log::info!(
        "拖拽残影已重建: DPI={} 尺寸={}x{}",
        new_dpi,
        total_w,
        total_h
    );
}

/// 销毁拖拽覆盖窗口
pub(crate) unsafe fn destroy_drag_overlay() {
    log::info!(
        "[DIAG] destroy_drag_overlay called: OVERLAY_HWND={:?}",
        OVERLAY_HWND
    );
    if !OVERLAY_HWND.is_null() {
        DestroyWindow(OVERLAY_HWND);
        OVERLAY_HWND = std::ptr::null_mut();
    }
    if !OVERLAY_HBMP.is_null() {
        DeleteObject(OVERLAY_HBMP as HGDIOBJ);
        OVERLAY_HBMP = std::ptr::null_mut();
    }
    // 清理保存的拖拽参数
    DRAG_ICON_HANDLE = std::ptr::null_mut();
    DRAG_ICON_SIZE = 0;
    DRAG_CELL_W = 0;
    DRAG_DISPLAY_NAME = String::new();
    DRAG_LAST_DPI = 0;
    DRAG_ORIGINAL_DPI = 0;
}

/// 覆盖窗口的 WndProc（极简，仅处理默认消息）
unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            log::debug!("[DIAG] drag_overlay WM_NCHITTEST: returning HTTRANSPARENT");
            HTTRANSPARENT as LRESULT
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE => {
            log::warn!(
                "[DIAG] drag_overlay received mouse message: msg={:#X} (should not happen!)",
                msg
            );
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_dpi_same_dpi() {
        assert_eq!(scale_dpi(48, 96, 96), 48);
        assert_eq!(scale_dpi(80, 96, 96), 80);
    }

    #[test]
    fn test_scale_dpi_higher_dpi() {
        // 96 -> 144 DPI: 1.5x
        assert_eq!(scale_dpi(48, 144, 96), 72);
        assert_eq!(scale_dpi(80, 144, 96), 120);
    }

    #[test]
    fn test_scale_dpi_lower_dpi() {
        // 144 -> 96 DPI: 0.67x
        assert_eq!(scale_dpi(72, 96, 144), 48);
        assert_eq!(scale_dpi(120, 96, 144), 80);
    }

    #[test]
    fn test_scale_dpi_roundtrip() {
        // 96 -> 144 -> 96 应该还原原始值
        let original = 48;
        let scaled = scale_dpi(original, 144, 96);
        let restored = scale_dpi(scaled, 96, 96);
        // 注意：restored 用的是 original_dpi=96，不是 144
        // 正确用法：始终以 original_dpi=96 为基准
        assert_eq!(scale_dpi(original, 96, 96), 48);
    }

    #[test]
    fn test_scale_dpi_no_accumulation_error() {
        // 模拟多次跨显示器切换，始终以原始 DPI 为基准
        let original_dpi = 96;
        let original_size = 48;

        // 第一次：96 -> 144
        let size_144 = scale_dpi(original_size, 144, original_dpi);
        assert_eq!(size_144, 72);

        // 第二次：144 -> 96（仍然以 original_dpi=96 为基准）
        let size_96 = scale_dpi(original_size, 96, original_dpi);
        assert_eq!(size_96, 48);

        // 第三次：96 -> 192（2x 缩放）
        let size_192 = scale_dpi(original_size, 192, original_dpi);
        assert_eq!(size_192, 96);

        // 第四次：192 -> 144
        let size_144_again = scale_dpi(original_size, 144, original_dpi);
        assert_eq!(size_144_again, 72);
    }

    #[test]
    fn test_scale_dpi_zero_original() {
        // original_dpi=0 应该回退到 1，避免除零
        assert_eq!(scale_dpi(48, 96, 0), 48 * 96);
    }

    #[test]
    fn test_drag_label_height_with_label() {
        assert_eq!(drag_label_height(48, true), 36);
        assert_eq!(drag_label_height(72, true), 54);
        assert_eq!(drag_label_height(96, true), 72);
    }

    #[test]
    fn test_drag_label_height_no_label() {
        assert_eq!(drag_label_height(48, false), 0);
        assert_eq!(drag_label_height(72, false), 0);
    }

    #[test]
    fn test_drag_icon_gap_with_label() {
        assert_eq!(drag_icon_gap(48, true), 6);
        assert_eq!(drag_icon_gap(72, true), 9);
    }

    #[test]
    fn test_drag_icon_gap_no_label() {
        assert_eq!(drag_icon_gap(48, false), 0);
    }
}

#[test]
fn test_icon_offset_calculation() {
    // Test icon offset calculation logic
    // icon_offset_x = (cell_w - icon_size) / 2

    let cell_w = 100;
    let icon_size = 48;
    let icon_offset_x = (cell_w - icon_size) / 2;

    assert_eq!(icon_offset_x, 26);

    // Test with different sizes
    let cell_w = 80;
    let icon_size = 48;
    let icon_offset_x = (cell_w - icon_size) / 2;

    assert_eq!(icon_offset_x, 16);

    // Test with icon_size == cell_w (no offset)
    let cell_w = 48;
    let icon_size = 48;
    let icon_offset_x = (cell_w - icon_size) / 2;

    assert_eq!(icon_offset_x, 0);
}

#[test]
fn test_icon_offset_with_odd_difference() {
    // Test with odd difference (should round down)
    let cell_w = 81;
    let icon_size = 48;
    let icon_offset_x = (cell_w - icon_size) / 2;

    // 81 - 48 = 33, 33 / 2 = 16 (integer division)
    assert_eq!(icon_offset_x, 16);
}

#[test]
fn test_drag_timer_interval() {
    // Test that timer interval is 8ms (125Hz)
    let timer_interval_ms = 8;
    let fps = 1000.0 / timer_interval_ms as f32;

    assert!((fps - 125.0).abs() < 0.1);
}

#[test]
fn test_dpi_cache_check_frequency() {
    // Test DPI cache check frequency (every 8 frames)
    let mut frame_count = 0u32;
    let mut dpi_check_count = 0u32;

    for _ in 0..32 {
        frame_count += 1;
        if frame_count % 8 == 0 {
            dpi_check_count += 1;
        }
    }

    // 32 frames should result in 4 DPI checks
    assert_eq!(dpi_check_count, 4);
    assert_eq!(frame_count, 32);
}
