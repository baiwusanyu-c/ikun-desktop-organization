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

/// 拖拽覆盖窗口句柄（全局，同一时刻只有一个拖拽）
static mut OVERLAY_HWND: HWND = std::ptr::null_mut();
static mut OVERLAY_HBMP: HBITMAP = std::ptr::null_mut();

/// 存储拖拽创建参数，用于跨显示器时按新 DPI 重建残影位图
static mut DRAG_ICON_HANDLE: HICON = std::ptr::null_mut();
static mut DRAG_ICON_SIZE: i32 = 0;
static mut DRAG_CELL_W: i32 = 0;
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
    destroy_drag_overlay();

    if icon_handle.is_null() {
        log::debug!("创建拖拽覆盖: 无图标句柄");
        return;
    }

    // 使用分类框的实际 cell_w，确保标签换行行为与框内一致
    let overlay_w = cell_w;
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

    // 创建分层窗口（WS_EX_LAYERED + WS_EX_TRANSPARENT + WS_EX_TOOLWINDOW）
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        DRAG_OVERLAY_CLASS.as_ptr(),
        std::ptr::null(),
        WS_POPUP,
        0,
        0,
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

    let pt_dst = POINT { x: 0, y: 0 };
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

    ShowWindow(hwnd, SW_SHOWNOACTIVATE);

    OVERLAY_HWND = hwnd;
    OVERLAY_HBMP = hbmp;

    // 保存拖拽参数，供 update_drag_overlay 跨显示器重建时使用
    DRAG_ICON_HANDLE = icon_handle;
    DRAG_ICON_SIZE = icon_size;
    DRAG_CELL_W = cell_w;
    DRAG_DISPLAY_NAME = display_name.to_string();
    DRAG_LAST_DPI = dpi;
    DRAG_ORIGINAL_DPI = dpi;

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

    // 检测光标所在显示器的 DPI，变化时重建残影
    let current_dpi = crate::platform::windows::desktop_overlay::dpi_at_screen_point(pt.x, pt.y);
    if current_dpi != DRAG_LAST_DPI && !DRAG_ICON_HANDLE.is_null() {
        rebuild_drag_overlay_at_dpi(current_dpi);
    }

    // 窗口位置偏移到光标右下方（避免遮挡光标热点）
    SetWindowPos(
        OVERLAY_HWND,
        HWND_TOPMOST,
        pt.x + 8,
        pt.y + 8,
        0,
        0,
        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
    );
}

/// 按新 DPI 重建拖拽残影位图并更新窗口尺寸。
unsafe fn rebuild_drag_overlay_at_dpi(new_dpi: u32) {
    let icon_handle = DRAG_ICON_HANDLE;
    let base_icon_size = DRAG_ICON_SIZE;
    let base_cell_w = DRAG_CELL_W;
    let display_name = (*std::ptr::addr_of!(DRAG_DISPLAY_NAME)).clone();

    // 按新 DPI 缩放尺寸，始终以创建时的原始 DPI 为基准，避免累积缩放误差
    let scale = |px: i32| (px * new_dpi as i32) / DRAG_ORIGINAL_DPI.max(1) as i32;
    let icon_size = scale(base_icon_size);
    let cell_w = scale(base_cell_w);

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

    let pt_dst = POINT { x: 0, y: 0 };
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
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
