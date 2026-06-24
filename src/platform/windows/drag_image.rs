//! 自定义拖拽覆盖窗口（替代 OLE IDragSourceHelper）
//!
//! 创建一个小型 WS_EX_LAYERED 窗口跟随鼠标光标，
//! 显示被拖拽图标的半透明图像。与原生桌面拖拽体验一致。
//!
//! 不依赖 OLE 拖拽图像（IDragSourceHelper），
//! 避免了 Shell 拒绝 drop 时拖拽图像不显示的问题。

use std::ffi::c_void;

use super::ffi_imports::*;
use super::ffi_types::*;

/// 拖拽覆盖窗口类名
static DRAG_OVERLAY_CLASS: &[u16] = &[
    73, 107, 117, 110, 68, 114, 97, 103, 79, 118, 101, 114, 108, 97, 121, 0,
]; // "IkunDragOverlay"

/// 拖拽覆盖窗口句柄（全局，同一时刻只有一个拖拽）
static mut OVERLAY_HWND: HWND = std::ptr::null_mut();
static mut OVERLAY_HBMP: HBITMAP = std::ptr::null_mut();

/// 创建拖拽覆盖窗口
///
/// 创建一个小窗口显示被拖拽图标的半透明图像。
/// 窗口跟随光标移动（通过 `update_drag_overlay` 更新位置）。
pub(crate) unsafe fn create_drag_overlay(icon_handle: HICON) {
    destroy_drag_overlay(); // 确保清理旧窗口

    if icon_handle.is_null() {
        log::debug!("创建拖拽覆盖: 无图标句柄");
        return;
    }

    // 将图标转换为 32-bit ARGB 位图
    let hbmp = icon_to_bitmap(icon_handle, 48);
    if hbmp.is_null() {
        log::warn!("创建拖拽覆盖: HICON → HBITMAP 失败");
        return;
    }

    let icon_size = 48;

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
    // WS_EX_TRANSPARENT: 鼠标事件穿透到底层窗口
    // WS_EX_TOOLWINDOW: 不在任务栏显示
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        DRAG_OVERLAY_CLASS.as_ptr(),
        std::ptr::null(),
        WS_POPUP,
        0, 0, icon_size, icon_size,
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
    let sz = SIZE { cx: icon_size, cy: icon_size };
    let pt_src = POINT { x: 0, y: 0 };

    // 70% 不透明度（半透明效果）
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 180, // 0-255, 180 ≈ 70%
        alpha_format: AC_SRC_ALPHA,
    };

    UpdateLayeredWindow(
        hwnd, hdc_screen,
        &pt_dst, &sz as *const SIZE as *const POINT,
        hdc_mem, &pt_src,
        0, &blend, 0x02, // ULW_ALPHA
    );

    SelectObject(hdc_mem, old_bmp);
    DeleteDC(hdc_mem);
    ReleaseDC(std::ptr::null_mut(), hdc_screen);

    // 显示窗口
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);

    OVERLAY_HWND = hwnd;
    OVERLAY_HBMP = hbmp;
    log::info!("拖拽覆盖窗口已创建 ({}x{}, 70% 不透明)", icon_size, icon_size);
}

/// 更新拖拽覆盖窗口位置（跟随光标）
pub(crate) unsafe fn update_drag_overlay() {
    if OVERLAY_HWND.is_null() {
        return;
    }

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);

    // 窗口位置偏移到光标右下方（避免遮挡光标热点）
    let offset_x = pt.x + 8;
    let offset_y = pt.y + 8;

    SetWindowPos(
        OVERLAY_HWND,
        std::ptr::null_mut(),
        offset_x, offset_y,
        0, 0,
        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
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
}

/// 覆盖窗口的 WndProc（极简，仅处理默认消息）
unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// =============================================================================
// HICON → HBITMAP 转换
// =============================================================================

/// 将 HICON 转换为指定尺寸的 32-bit ARGB HBITMAP
///
/// 使用 DrawIconEx 渲染 RGB + GetIconInfo 修复 alpha 通道。
unsafe fn icon_to_bitmap(icon_handle: HICON, icon_size: i32) -> HBITMAP {
    let hdc_screen = GetDC(std::ptr::null_mut());
    if hdc_screen.is_null() { return std::ptr::null_mut(); }

    let hdc_mem = CreateCompatibleDC(hdc_screen);
    if hdc_mem.is_null() {
        ReleaseDC(std::ptr::null_mut(), hdc_screen);
        return std::ptr::null_mut();
    }

    let bmi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: icon_size,
        bi_height: -icon_size,
        bi_planes: 1,
        bi_bit_count: 32,
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let hbmp = CreateDIBSection(
        hdc_mem,
        &bmi as *const BITMAPINFOHEADER as *const c_void,
        DIB_RGB_COLORS,
        &mut bits,
        std::ptr::null_mut(),
        0,
    );

    if hbmp.is_null() || bits.is_null() {
        DeleteDC(hdc_mem);
        ReleaseDC(std::ptr::null_mut(), hdc_screen);
        return std::ptr::null_mut();
    }

    let old_bmp = SelectObject(hdc_mem, hbmp as HGDIOBJ);

    // 1. DrawIconEx 渲染 RGB
    DrawIconEx(hdc_mem, 0, 0, icon_handle, icon_size, icon_size, 0, std::ptr::null_mut(), 0x0003);

    // 2. 获取 mask bitmap 修复 alpha
    let mut icon_info = std::mem::zeroed::<ICONINFO>();
    if GetIconInfo(icon_handle, &mut icon_info) != 0 && !icon_info.hbm_mask.is_null() {
        let pixel_count = (icon_size * icon_size) as usize;
        let mut mask_bits = vec![0u32; pixel_count];

        let mut mask_bmi = BITMAPINFOHEADER {
            bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            bi_width: icon_size,
            bi_height: -icon_size,
            bi_planes: 1,
            bi_bit_count: 32,
            bi_compression: 0,
            bi_size_image: 0,
            bi_x_pels_per_meter: 0,
            bi_y_pels_per_meter: 0,
            bi_clr_used: 0,
            bi_clr_important: 0,
        };

        let got = GetDIBits(
            hdc_mem, icon_info.hbm_mask,
            0, icon_size as u32,
            mask_bits.as_mut_ptr() as *mut c_void,
            &mut mask_bmi as *mut BITMAPINFOHEADER as *mut c_void,
            DIB_RGB_COLORS,
        );

        if got > 0 {
            let pixels = bits as *mut u32;
            for i in 0..pixel_count {
                if mask_bits[i] & 0x00FFFFFF == 0 {
                    *pixels.add(i) = *pixels.add(i) | 0xFF000000;
                } else {
                    *pixels.add(i) = 0;
                }
            }
        }

        DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        if !icon_info.hbm_color.is_null() {
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
        }
    }

    SelectObject(hdc_mem, old_bmp);
    DeleteDC(hdc_mem);
    ReleaseDC(std::ptr::null_mut(), hdc_screen);

    hbmp
}
