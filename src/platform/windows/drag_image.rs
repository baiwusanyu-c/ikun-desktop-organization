//! 自定义拖拽覆盖窗口（替代 OLE IDragSourceHelper）
//!
//! 创建一个小型 WS_EX_LAYERED 窗口跟随鼠标光标，
//! 显示被拖拽图标 + 名称的半透明图像。与原生桌面拖拽体验一致。
//!
//! # 渲染方案
//!
//! 使用 `DrawIconEx` 在透明 DIB Section 上渲染图标（保留正确 alpha），
//! 再用 GDI `TextOutW` 绘制名称标签，最后对文字区域做 alpha 后处理
//! （GDI 不写 alpha 通道，需手动修正）。
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
/// - `display_name`: 图标下方的标签文字（空字符串则不显示标签）
pub(crate) unsafe fn create_drag_overlay(
    icon_handle: HICON,
    icon_size: i32,
    display_name: &str,
) {
    destroy_drag_overlay();

    if icon_handle.is_null() {
        log::debug!("创建拖拽覆盖: 无图标句柄");
        return;
    }

    // 计算总尺寸：图标 + 标签区域
    let label_h = if display_name.is_empty() { 0 } else { icon_size / 2 };
    let total_w = icon_size;
    let total_h = icon_size + label_h;

    // 创建复合位图（图标 + 标签，32-bit ARGB straight alpha）
    let hbmp = create_drag_bitmap(icon_handle, icon_size, display_name, total_w, total_h);
    if hbmp.is_null() {
        log::warn!("创建拖拽覆盖: 位图创建失败");
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
        0, 0, total_w, total_h,
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
    let sz = SIZE { cx: total_w, cy: total_h };
    let pt_src = POINT { x: 0, y: 0 };

    // 70% 不透明度（半透明效果，与原生桌面拖拽一致）
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

    ShowWindow(hwnd, SW_SHOWNOACTIVATE);

    OVERLAY_HWND = hwnd;
    OVERLAY_HBMP = hbmp;
    log::info!("拖拽覆盖窗口已创建 ({}x{}, 70% 不透明)", total_w, total_h);
}

/// 更新拖拽覆盖窗口位置（跟随光标）
pub(crate) unsafe fn update_drag_overlay() {
    if OVERLAY_HWND.is_null() {
        return;
    }

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);

    // 窗口位置偏移到光标右下方（避免遮挡光标热点）
    SetWindowPos(
        OVERLAY_HWND,
        std::ptr::null_mut(),
        pt.x + 8, pt.y + 8,
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
// 复合位图创建（DrawIconEx + GDI 文字 + alpha 后处理）
// =============================================================================

/// 创建拖拽复合位图（图标 + 标签文字）
///
/// 使用 `DrawIconEx` 在透明 DIB Section 上渲染图标，
/// 然后用 GDI 绘制名称标签，最后对文字区域做 alpha 后处理。
///
/// `DrawIconEx` 在预填充透明黑 (0,0,0,0) 的 DIB 上绘制 32-bit ARGB 图标时，
/// 能正确保留 alpha 通道（透明区域保持 alpha=0，不透明区域 alpha 正确）。
unsafe fn create_drag_bitmap(
    icon_handle: HICON,
    icon_size: i32,
    display_name: &str,
    total_w: i32,
    total_h: i32,
) -> HBITMAP {
    let hdc_screen = GetDC(std::ptr::null_mut());
    if hdc_screen.is_null() {
        return std::ptr::null_mut();
    }

    let hdc_mem = CreateCompatibleDC(hdc_screen);
    if hdc_mem.is_null() {
        ReleaseDC(std::ptr::null_mut(), hdc_screen);
        return std::ptr::null_mut();
    }

    // 创建 32-bit top-down DIB Section（预填充透明黑）
    let bmi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: total_w,
        bi_height: -total_h, // top-down
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

    // 1. DrawIconEx 渲染图标到透明 DIB
    //    DI_NORMAL = 0x0003 (DI_MASK | DI_IMAGE)
    //    在预填充 (0,0,0,0) 的 DIB 上，DrawIconEx 正确保留 alpha 通道
    DrawIconEx(
        hdc_mem, 0, 0, icon_handle,
        icon_size, icon_size,
        0, std::ptr::null_mut(),
        0x0003, // DI_NORMAL
    );

    // 2. 绘制名称标签（如果有）
    if !display_name.is_empty() && !bits.is_null() {
        let label_y = icon_size + 2;
        let label_h = total_h - label_y;

        // 创建字体（微软雅黑，ClearType）
        let font_size = -(icon_size * 12 / 48); // 按比例缩放（负值 = 字符高度）
        let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
        let font = CreateFontW(
            font_size, 0, 0, 0,
            FW_NORMAL,
            0, 0, 0, // 非斜体、非下划线、非删除线
            DEFAULT_CHARSET,
            0, 0, // OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS
            CLEARTYPE_QUALITY,
            FF_DONTCARE,
            font_name.as_ptr(),
        );

        if !font.is_null() {
            let old_font = SelectObject(hdc_mem, font);
            SetBkMode(hdc_mem, TRANSPARENT_BK_MODE);
            // 白色文字（桌面拖拽标签标准颜色）
            SetTextColor(hdc_mem, 0x00FFFFFF); // RGB(255,255,255)

            let text_wide: Vec<u16> = display_name.encode_utf16().collect();

            // 测量文字宽度，居中绘制
            let mut text_size = SIZE { cx: 0, cy: 0 };
            GetTextExtentPoint32W(
                hdc_mem,
                text_wide.as_ptr(),
                text_wide.len() as i32,
                &mut text_size,
            );

            let mut text_x = (total_w - text_size.cx) / 2;
            if text_x < 0 { text_x = 0; }
            let mut text_y = label_y + (label_h - text_size.cy) / 2;
            if text_y < label_y { text_y = label_y; }

            TextOutW(
                hdc_mem,
                text_x, text_y,
                text_wide.as_ptr(),
                text_wide.len() as i32,
            );

            SelectObject(hdc_mem, old_font);
            DeleteObject(font);

            // 3. Alpha 后处理：修正文字区域的 alpha 通道
            //    GDI 文字渲染可能不写 alpha 通道，导致文字像素 alpha=0（不可见）
            //    对于白色文字，使用 max(R,G,B) 作为 alpha 值
            fix_text_alpha(
                bits as *mut u32,
                total_w, total_h,
                label_y, label_h,
            );
        }
    }

    SelectObject(hdc_mem, old_bmp);
    DeleteDC(hdc_mem);
    ReleaseDC(std::ptr::null_mut(), hdc_screen);

    hbmp
}

/// 修正文字区域的 alpha 通道
///
/// GDI 在 32-bit DIB 上绘制文字时，可能不正确设置 alpha 通道。
/// 对于白色文字，使用 `max(R, G, B)` 作为 alpha 值：
/// - 纯白像素 (255,255,255) → alpha=255（完全不透明）
/// - 抗锯齿过渡像素 (128,128,128) → alpha=128（半透明）
/// - 透明背景 (0,0,0) → alpha=0（完全透明）
unsafe fn fix_text_alpha(
    pixels: *mut u32,
    width: i32,
    height: i32,
    region_y: i32,
    region_h: i32,
) {
    let start_y = if region_y < 0 { 0 } else { region_y };
    let end_y = if region_y + region_h > height { height } else { region_y + region_h };

    for y in start_y..end_y {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let pixel = *pixels.add(idx);
            let b = (pixel & 0xFF) as u8;
            let g = ((pixel >> 8) & 0xFF) as u8;
            let r = ((pixel >> 16) & 0xFF) as u8;
            let max_rgb = r.max(g).max(b);
            // 只修正文字像素（非零 RGB），不修改图标区域
            if max_rgb > 0 {
                let new_pixel = (pixel & 0x00FFFFFF) | ((max_rgb as u32) << 24);
                *pixels.add(idx) = new_pixel;
            }
        }
    }
}
