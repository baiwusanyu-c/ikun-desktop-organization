//! Direct2D/DirectWrite 渲染代码

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! 本模块包含使用 Direct2D 和 DirectWrite 渲染格子窗口内容的所有代码。
//! 包括设备初始化、分层窗口渲染和图标绘制。
//!
//! # 图标渲染架构
//!
//! `render_icon_grid` 是独立的图标渲染方法，通过参数驱动：
//! - `GridLayout`：布局位置（cols/cell_w/cell_h/grid_left/grid_top）
//! - `ViewParams`：视图模式参数（icon_size/show_label 等）
//!
//! 支持 4 种图标模式（ExtraLarge/Large/Medium/Small），未来扩展列表/详细信息/平铺/内容。

use crate::box_core::layout_calc::{calculate_grid, GridLayout};
use crate::box_core::view_engine::{get_view_params, ViewParams};

use super::grid_window::GridFileItem;

use std::ffi::c_void;

use windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED;
use windows::Win32::Graphics::Direct2D::{
    Common::{D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_SIZE_U},
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_WORD, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::GridWindowData;

/// 初始化 D2D/DirectWrite/WIC 设备（首次渲染时调用）
pub fn init_d2d_devices(data: &mut GridWindowData) {
    // D2D Factory
    if data.d2d_factory.is_none() {
        match unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
        {
            Ok(f) => data.d2d_factory = Some(f),
            Err(e) => {
                log::error!("创建 D2D Factory 失败: {:?}", e);
                return;
            }
        }
    }

    // DirectWrite Factory
    if data.dw_factory.is_none() {
        match unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) } {
            Ok(f) => data.dw_factory = Some(f),
            Err(e) => {
                log::error!("创建 DirectWrite Factory 失败: {:?}", e);
                return;
            }
        }
    }

    // WIC Imaging Factory
    if data.wic_factory.is_none() {
        match unsafe {
            CoCreateInstance::<_, IWICImagingFactory>(
                &CLSID_WICImagingFactory,
                None,
                CLSCTX_INPROC_SERVER,
            )
        } {
            Ok(f) => data.wic_factory = Some(f),
            Err(e) => {
                log::error!("创建 WIC Factory 失败: {:?}", e);
                return;
            }
        }
    }

    // DC Render Target（创建一次，每次渲染时 BindDC 到新 DC）
    if data.d2d_target.is_none() {
        if let Some(ref factory) = data.d2d_factory {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode:
                        windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            match unsafe { factory.CreateDCRenderTarget(&props) } {
                Ok(rt) => {
                    data.d2d_target = Some(rt);
                    log::info!("D2D DC RenderTarget 已创建");
                }
                Err(e) => {
                    log::error!("创建 D2D DC RenderTarget 失败: {:?}", e);
                }
            }
        }
    }
}

/// 渲染分层窗口内容（Direct2D/DirectWrite → DIB → UpdateLayeredWindow）
///
/// 背景 rgba(1,1,1,0.15) = alpha 38/255 ≈ 15% 不透明
/// 标题文字和图标以完全不透明（alpha=255）绘制
///
/// 重要：使用 GetWindowRect（而非 GetClientRect）获取尺寸。
/// 对 WS_POPUP|WS_THICKFRAME 分层窗口，UpdateLayeredWindow 的 psize 参数
/// 会直接设置窗口尺寸。
pub fn render_layered_window(hwnd: HWND) {
    // 递归保护：防止 UpdateLayeredWindow 触发 WM_SIZE → 再次进入本函数
    use std::sync::atomic::{AtomicBool, Ordering};
    static RENDERING: AtomicBool = AtomicBool::new(false);
    if RENDERING.swap(true, Ordering::SeqCst) {
        return; // 已在渲染中，跳过
    }

    let data = unsafe {
        match get_data(hwnd) {
            Some(d) => d,
            None => {
                RENDERING.store(false, Ordering::SeqCst);
                return;
            }
        }
    };

    // 确保 D2D/DW/WIC 设备已初始化
    init_d2d_devices(data);

    let (Some(ref _d2d_factory), Some(ref dw_factory), Some(ref wic_factory), Some(ref target)) = (
        &data.d2d_factory,
        &data.dw_factory,
        &data.wic_factory,
        &data.d2d_target,
    ) else {
        log::warn!("D2D 设备未初始化，跳过渲染");
        RENDERING.store(false, Ordering::SeqCst);
        return;
    };

    // 使用 GetWindowRect 获取完整窗口尺寸
    let mut wr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut wr) };
    let w = wr.right - wr.left;
    let h = wr.bottom - wr.top;
    if w <= 0 || h <= 0 {
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    let ww = w as f32;
    let hh = h as f32;

    // 创建 32 位 ARGB DIB Section（D2D 渲染后端）
    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    let mem_dc = unsafe { CreateCompatibleDC(screen_dc) };

    let bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: w,
        bi_height: -h, // 自上而下
        bi_planes: 1,
        bi_bit_count: 32,
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };

    let mut bits: *mut Pixel32 = std::ptr::null_mut();
    let dib = unsafe {
        CreateDIBSection(
            mem_dc,
            &bi as *const BITMAPINFOHEADER as *const c_void,
            DIB_RGB_COLORS,
            &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
            std::ptr::null_mut(),
            0,
        )
    };
    if dib.is_null() || bits.is_null() {
        unsafe {
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    let old_bmp = unsafe { SelectObject(mem_dc, dib as HGDIOBJ) };

    // 绑定 D2D render target 到 memory DC
    let bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    if unsafe { target.BindDC(GdiHDC(mem_dc), &bind_rect) }.is_err() {
        log::error!("BindDC 失败");
        unsafe {
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    // ===== D2D 渲染开始 =====
    unsafe { target.BeginDraw() };

    // 清空为全透明（premultiplied alpha: 所有通道为 0）
    let transparent = D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    unsafe { target.Clear(Some(&transparent)) };

    // ===== 获取 DPI =====
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let _dpi = if dpi > 0 { dpi } else { 96 }; // DPI 预留供后续 DPI 感知渲染使用

    // ===== 绘制圆角背景（整体，四角圆角）=====
    let bg_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(1, 1, 1, 38), None) };
    if let Ok(ref brush) = bg_brush {
        let corner_radius = 16.0_f32;
        let rounded_rect = D2D1_ROUNDED_RECT {
            rect: d2d_rect(0.0, 0.0, ww, hh),
            radiusX: corner_radius,
            radiusY: corner_radius,
        };
        unsafe { target.FillRoundedRectangle(&rounded_rect, brush) };
    }

    // ===== 绘制标题栏与主体的分隔线 =====
    let title_h = data.title_h;
    let sep_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(100, 100, 100, 100), None) };
    if let Ok(ref brush) = sep_brush {
        let sep_rect = d2d_rect(0.0, (title_h - 1) as f32, ww, title_h as f32);
        unsafe { target.FillRectangle(&sep_rect, brush) };
    }

    // ===== 获取 DPI =====
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let _dpi = if dpi > 0 { dpi } else { 96 }; // DPI 预留供后续 DPI 感知渲染使用

    // ===== 创建文字画刷（标题专用，图标标签画刷在 render_icon_grid 内创建）=====
    let white_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 255), None) };
    let shadow_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(0, 0, 0, 200), None) };

    // ===== 标题文字（DirectWrite 渲染）=====
    let title_font_size = scale_for_dpi(15, dpi) as f32;
    let title_format = unsafe {
        let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
        let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
        dw_factory.CreateTextFormat::<_, _, _>(
            windows::core::PCWSTR(font_name.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            title_font_size,
            windows::core::PCWSTR(locale.as_ptr()),
        )
    };

    if let Ok(ref tf) = title_format {
        let _ = unsafe { tf.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP) };

        // ===== 正常模式：绘制标题文字 =====
        let title_text: Vec<u16> = data.title.encode_utf16().collect();

        // 锁定按钮区域（标题栏右侧）
        let (btn_x, btn_y, btn_size) = super::grid_layout::lock_btn_rect(w, title_h, dpi);

        // 创建 TextLayout（右侧为锁定按钮预留空间）
        let text_max_w = (btn_x as f32 - 10.0).max(40.0);
        let layout = unsafe {
            target_inner_create_text_layout(
                dw_factory,
                &title_text,
                tf,
                text_max_w,
                title_h as f32,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
            )
        };

        if let Some(ref layout) = layout {
            // 阴影描边
            let shadow_offset = scale_for_dpi(1, dpi) as f32;
            if let Ok(ref sb) = shadow_brush {
                for &(dx, dy) in &[
                    (shadow_offset, 0.0),
                    (-shadow_offset, 0.0),
                    (0.0, shadow_offset),
                    (0.0, -shadow_offset),
                ] {
                    let origin = D2D_POINT_2F {
                        x: 10.0 + dx,
                        y: dy,
                    };
                    unsafe {
                        target.DrawTextLayout(origin, layout, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
            // 主文字（白色）
            if let Ok(ref wb) = white_brush {
                let origin = D2D_POINT_2F { x: 10.0, y: 0.0 };
                unsafe {
                    target.DrawTextLayout(origin, layout, wb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                }
            }
        }

        // ===== 绘制锁定按钮（标题栏右侧）=====
        let lock_icon = if data.locked { "🔒" } else { "🔓" };
        let lock_font_size = scale_for_dpi(14, dpi) as f32;
        let lock_format = unsafe {
            let font_name: Vec<u16> = "Segoe UI Emoji\0".encode_utf16().collect();
            let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
            dw_factory.CreateTextFormat::<_, _, _>(
                windows::core::PCWSTR(font_name.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                lock_font_size,
                windows::core::PCWSTR(locale.as_ptr()),
            )
        };

        if let Ok(ref lf) = lock_format {
            let _ = unsafe { lf.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) };
            let _ = unsafe { lf.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };

            let lock_text: Vec<u16> = lock_icon.encode_utf16().collect();
            let lock_layout = unsafe {
                target_inner_create_text_layout(
                    dw_factory,
                    &lock_text,
                    lf,
                    btn_size as f32,
                    btn_size as f32,
                    DWRITE_TEXT_ALIGNMENT_CENTER,
                    DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                )
            };

            if let Some(ref ll) = lock_layout {
                // 锁定按钮 hover 背景高亮
                if data.lock_hovered {
                    let hover_brush = unsafe {
                        target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 40), None)
                    };
                    if let Ok(ref hb) = hover_brush {
                        let pad = scale_for_dpi(3, dpi) as f32;
                        let hover_rect = D2D1_ROUNDED_RECT {
                            rect: d2d_rect(
                                (btn_x - pad as i32) as f32,
                                (btn_y - pad as i32) as f32,
                                (btn_x + btn_size + pad as i32) as f32,
                                (btn_y + btn_size + pad as i32) as f32,
                            ),
                            radiusX: 4.0,
                            radiusY: 4.0,
                        };
                        unsafe { target.FillRoundedRectangle(&hover_rect, hb) };
                    }
                }

                // 锁定按钮阴影
                let shadow_offset = scale_for_dpi(1, dpi) as f32;
                if let Ok(ref sb) = shadow_brush {
                    for &(sdx, sdy) in &[
                        (shadow_offset, 0.0),
                        (-shadow_offset, 0.0),
                        (0.0, shadow_offset),
                        (0.0, -shadow_offset),
                    ] {
                        let origin = D2D_POINT_2F {
                            x: btn_x as f32 + sdx,
                            y: btn_y as f32 + sdy,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, ll, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                        }
                    }
                }
                // 锁定按钮主文字
                if let Ok(ref wb) = white_brush {
                    let origin = D2D_POINT_2F {
                        x: btn_x as f32,
                        y: btn_y as f32,
                    };
                    unsafe {
                        target.DrawTextLayout(origin, ll, wb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
        }
    }

    // ===== 绘制"更多"按钮（⋮，锁定按钮右侧）=====
    let (mbtn_x, mbtn_y, mbtn_size) = super::grid_layout::more_btn_rect(w, title_h, dpi);
    let more_icon = "\u{22EE}"; // ⋮ 三个竖点
    let more_font_size = scale_for_dpi(16, dpi) as f32;
    let more_format = unsafe {
        let font_name: Vec<u16> = "Segoe UI Symbol\0".encode_utf16().collect();
        let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
        dw_factory.CreateTextFormat::<_, _, _>(
            windows::core::PCWSTR(font_name.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            more_font_size,
            windows::core::PCWSTR(locale.as_ptr()),
        )
    };

    if let Ok(ref mf) = more_format {
        let _ = unsafe { mf.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) };
        let _ = unsafe { mf.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };

        let more_text: Vec<u16> = more_icon.encode_utf16().collect();
        let more_layout = unsafe {
            target_inner_create_text_layout(
                dw_factory,
                &more_text,
                mf,
                mbtn_size as f32,
                mbtn_size as f32,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
            )
        };

        if let Some(ref ml) = more_layout {
            // hover 背景高亮
            if data.more_hovered || data.menu.open {
                let hover_brush =
                    unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 40), None) };
                if let Ok(ref hb) = hover_brush {
                    let pad = scale_for_dpi(3, dpi) as f32;
                    let hover_rect = D2D1_ROUNDED_RECT {
                        rect: d2d_rect(
                            (mbtn_x - pad as i32) as f32,
                            (mbtn_y - pad as i32) as f32,
                            (mbtn_x + mbtn_size + pad as i32) as f32,
                            (mbtn_y + mbtn_size + pad as i32) as f32,
                        ),
                        radiusX: 4.0,
                        radiusY: 4.0,
                    };
                    unsafe { target.FillRoundedRectangle(&hover_rect, hb) };
                }
            }

            // 阴影
            let shadow_offset = scale_for_dpi(1, dpi) as f32;
            if let Ok(ref sb) = shadow_brush {
                for &(sdx, sdy) in &[
                    (shadow_offset, 0.0),
                    (-shadow_offset, 0.0),
                    (0.0, shadow_offset),
                    (0.0, -shadow_offset),
                ] {
                    let origin = D2D_POINT_2F {
                        x: mbtn_x as f32 + sdx,
                        y: mbtn_y as f32 + sdy,
                    };
                    unsafe {
                        target.DrawTextLayout(origin, ml, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
            // 主文字
            if let Ok(ref wb) = white_brush {
                let origin = D2D_POINT_2F {
                    x: mbtn_x as f32,
                    y: mbtn_y as f32,
                };
                unsafe {
                    target.DrawTextLayout(origin, ml, wb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                }
            }
        }
    }

    // ===== 绘制图标网格（独立方法，参数化设计）=====
    // 通过 view_engine 获取视图参数，layout_calc 计算网格布局
    let view_params = get_view_params(&data.view_mode, dpi);
    let grid_layout = calculate_grid(w, title_h, &view_params, dpi);

    render_icon_grid(
        target,
        dw_factory,
        wic_factory,
        &data.items,
        &grid_layout,
        &view_params,
        data.selected_item,
        data.hovered_item,
        dpi,
        data.scroll_offset,
        h,
    );

    // ===== 绘制边缘拖拽指示条（未锁定时鼠标悬停边缘显示）=====
    if !data.locked && data.hover_edge > 0 {
        let edge_brush =
            unsafe { target.CreateSolidColorBrush(&d2d_color(120, 180, 255, 100), None) };
        if let Ok(ref eb) = edge_brush {
            let border_w = scale_for_dpi(3, dpi) as f32;
            let edge_rect = match data.hover_edge {
                1 => d2d_rect(0.0, 0.0, border_w, hh),     // 左边缘
                2 => d2d_rect(ww - border_w, 0.0, ww, hh), // 右边缘
                3 => d2d_rect(0.0, 0.0, ww, border_w),     // 上边缘
                4 => d2d_rect(0.0, hh - border_w, ww, hh), // 下边缘
                _ => d2d_rect(0.0, 0.0, 0.0, 0.0),
            };
            unsafe { target.FillRectangle(&edge_rect, eb) };
        }
    }

    // ===== D2D 渲染结束 =====
    let mut tag1: u64 = 0;
    let mut tag2: u64 = 0;
    if let Err(e) = unsafe { target.EndDraw(Some(&mut tag1), Some(&mut tag2)) } {
        log::error!("D2D EndDraw 失败: {:?}", e);
    }

    // ===== 调用 UpdateLayeredWindow 提交渲染结果 =====
    let size = POINT { x: w, y: h };
    let src_pt = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 255,
        alpha_format: AC_SRC_ALPHA,
    };

    unsafe {
        UpdateLayeredWindow(
            hwnd,
            screen_dc,
            std::ptr::null(),
            &size,
            mem_dc,
            &src_pt,
            0,
            &blend,
            LWA_ALPHA,
        );
    }

    // 清理 GDI 资源（D2D target 保留复用）
    unsafe {
        SelectObject(mem_dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }

    RENDERING.store(false, Ordering::SeqCst);
}

/// 渲染图标网格（独立方法，参数化设计）
///
/// 通过 `GridLayout` 确定位置（cols/cell_w/cell_h/grid_left/grid_top），
/// 通过 `ViewParams` 控制视图模式（icon_size/show_label 等）。
///
/// # 支持的视图模式
///
/// | 模式 | icon_size | 说明 |
/// |------|-----------|------|
/// | ExtraLarge | 96px | 超大图标 |
/// | Large | 64px | 大图标 |
/// | Medium | 48px | 中等图标（默认） |
/// | Small | 32px | 小图标 |
///
/// # 自动换行逻辑
///
/// `GridLayout` 已处理窄窗口边界：当窗口宽度不足以容纳一个完整 cell 时，
/// `cell_w` 被 clamp 到可用宽度，`cols` 强制为 1，保证图标始终可见。
///
/// # 参数
///
/// - `target`：D2D render target（已 BindDC）
/// - `dw_factory`：DirectWrite 工厂（用于创建文字格式）
/// - `wic_factory`：WIC 工厂（用于图标位图转换，保留扩展）
/// - `items`：要渲染的文件项列表
/// - `layout`：网格布局（由 `layout_calc::calculate_grid` 计算）
/// - `view_params`：视图参数（由 `view_engine::get_view_params` 获取）
/// - `selected_item`：当前选中项索引（-1 = 无选中）
/// - `hovered_item`：当前 hover 项索引（-1 = 无 hover）
/// - `dpi`：窗口 DPI（用于缩放字体和间距）
pub fn render_icon_grid(
    target: &ID2D1DCRenderTarget,
    dw_factory: &IDWriteFactory,
    _wic_factory: &IWICImagingFactory,
    items: &[GridFileItem],
    layout: &GridLayout,
    view_params: &ViewParams,
    selected_item: i32,
    hovered_item: i32,
    dpi: u32,
    scroll_offset: i32,
    window_height: i32,
) {
    let icon_size = view_params.icon_size;
    let cell_w = layout.cell_w;
    let cell_h = layout.cell_h;
    let cols = layout.cols;
    let grid_left = layout.grid_left;
    let grid_top = layout.grid_top;
    let icon_label_h = scale_for_dpi(36, dpi); // 2 行文字高度（支持换行）

    // 创建文字画刷（阴影 + 标签）
    let shadow_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(0, 0, 0, 200), None) };
    let label_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(250, 250, 255, 255), None) };

    // 图标标签字体（根据视图模式动态调整）
    let label_font_size = scale_for_dpi(13, dpi) as f32;
    let label_format = unsafe {
        let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
        let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
        dw_factory.CreateTextFormat::<_, _, _>(
            windows::core::PCWSTR(font_name.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            label_font_size,
            windows::core::PCWSTR(locale.as_ptr()),
        )
    };

    if let Ok(ref lf) = label_format {
        let _ = unsafe { lf.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) };
        let _ = unsafe { lf.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };
        let _ = unsafe { lf.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP) };
    }

    // 省略号裁剪标志（line-clamp：最多 2 行 + "..."）
    let ellipsis_sign = if let Ok(ref lf) = label_format {
        unsafe { dw_factory.CreateEllipsisTrimmingSign(lf).ok() }
    } else {
        None
    };

    // 选中项高亮画刷
    let highlight_brush =
        unsafe { target.CreateSolidColorBrush(&d2d_color(60, 80, 120, 120), None) };
    // Hover 高亮画刷（比选中色浅）
    let hover_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(70, 90, 130, 70), None) };

    for (i, item) in items.iter().enumerate() {
        let (cell_x, cell_y) = if item.grid_col >= 0 && item.grid_row >= 0 {
            // Per-item grid position (user-placed via drag)
            (
                grid_left + item.grid_col * cell_w,
                grid_top + item.grid_row * cell_h,
            )
        } else {
            // Auto-layout: index-based position
            let col = (i as i32) % cols;
            let row = (i as i32) / cols;
            (grid_left + col * cell_w, grid_top + row * cell_h)
        };
        // Apply scroll offset
        let cell_y = cell_y - scroll_offset;
        let is_selected = i as i32 == selected_item;
        let is_hovered = i as i32 == hovered_item;

        // Hover 背景（比选中浅，仅非选中项显示）
        if is_hovered && !is_selected {
            if let Ok(ref hb) = hover_brush {
                let hl_rect = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(
                        cell_x as f32,
                        cell_y as f32,
                        (cell_x + cell_w) as f32,
                        (cell_y + cell_h) as f32,
                    ),
                    radiusX: scale_for_dpi(6, dpi) as f32,
                    radiusY: scale_for_dpi(6, dpi) as f32,
                };
                unsafe { target.FillRoundedRectangle(&hl_rect, hb) };
            }
        }

        // 选中项高亮背景
        if is_selected {
            if let Ok(ref hb) = highlight_brush {
                let hl_rect = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(
                        cell_x as f32,
                        cell_y as f32,
                        (cell_x + cell_w) as f32,
                        (cell_y + cell_h) as f32,
                    ),
                    radiusX: scale_for_dpi(6, dpi) as f32,
                    radiusY: scale_for_dpi(6, dpi) as f32,
                };
                unsafe { target.FillRoundedRectangle(&hl_rect, hb) };
            }
        }

        // 绘制图标（HICON → GetDIBits → D2D Bitmap → DrawBitmap）
        let icon_x = cell_x + (cell_w - icon_size) / 2;
        let icon_y = cell_y + 4;
        if !item.icon_handle.is_null() {
            draw_icon_d2d(
                target,
                Some(_wic_factory),
                item.icon_handle,
                icon_x,
                icon_y,
                icon_size,
            );
        }

        // 绘制图标标签文字（DirectWrite）
        if view_params.show_label {
            if let Ok(ref lf) = label_format {
                let label: Vec<u16> = item.display_name.encode_utf16().collect();
                let label_x = cell_x + 2;
                let label_y = icon_y + icon_size + scale_for_dpi(6, dpi);
                let label_w = (cell_w - 4) as f32;

                // 选中项：足够高度显示完整名称；未选中：2 行裁剪
                let (label_h, para_align) = if is_selected {
                    let max_h = scale_for_dpi(36 * 3, dpi);
                    (max_h as f32, DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
                } else {
                    (icon_label_h as f32, DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
                };

                let text_layout = unsafe {
                    target_inner_create_text_layout(
                        dw_factory,
                        &label,
                        lf,
                        label_w,
                        label_h,
                        DWRITE_TEXT_ALIGNMENT_CENTER,
                        para_align,
                    )
                };

                if let Some(ref tl) = text_layout {
                    // 未选中：应用 line-clamp（2 行 + 省略号）
                    if !is_selected {
                        if let Some(ref sign) = ellipsis_sign {
                            let trimming = DWRITE_TRIMMING {
                                granularity: DWRITE_TRIMMING_GRANULARITY_WORD,
                                delimiter: 0,
                                delimiterCount: 0,
                            };
                            let _ = unsafe { tl.SetTrimming(&trimming, sign) };
                        }
                    }

                    // 阴影描边
                    let lsh = scale_for_dpi(1, dpi) as f32;
                    if let Ok(ref sb) = shadow_brush {
                        for &(dx, dy) in &[(lsh, 0.0), (-lsh, 0.0), (0.0, lsh), (0.0, -lsh)] {
                            let origin = D2D_POINT_2F {
                                x: label_x as f32 + dx,
                                y: label_y as f32 + dy,
                            };
                            unsafe {
                                target.DrawTextLayout(origin, tl, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                            }
                        }
                    }
                    // 主文字（浅灰白色）
                    if let Ok(ref lb) = label_brush {
                        let origin = D2D_POINT_2F {
                            x: label_x as f32,
                            y: label_y as f32,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, tl, lb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                        }
                    }
                }
            }
        }
    }

    // ===== Draw scrollbar if content overflows =====
    let total_rows = (items.len() as i32 + cols - 1) / cols;
    let total_content_h = total_rows * cell_h;
    let visible_h = window_height - grid_top;
    if total_content_h > visible_h && visible_h > 0 {
        // Calculate scrollbar thumb size and position
        let scrollbar_w = scale_for_dpi(6, dpi) as f32;
        let min_thumb_h = scale_for_dpi(30, dpi) as f32;
        let thumb_h = (visible_h as f32 * visible_h as f32 / total_content_h as f32).max(min_thumb_h);
        let max_scroll = total_content_h - visible_h;
        let scroll_ratio = if max_scroll > 0 { scroll_offset as f32 / max_scroll as f32 } else { 0.0 };
        let track_h = visible_h as f32 - thumb_h;
        let thumb_y = grid_top as f32 + scroll_ratio * track_h;

        // Draw scrollbar track (subtle background)
        let track_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 15), None) };
        if let Ok(ref tb) = track_brush {
            let track_rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(
                    (layout.grid_left + cols * cell_w) as f32 + scale_for_dpi(2, dpi) as f32,
                    grid_top as f32,
                    (layout.grid_left + cols * cell_w) as f32 + scale_for_dpi(2, dpi) as f32 + scrollbar_w,
                    window_height as f32,
                ),
                radiusX: scrollbar_w / 2.0,
                radiusY: scrollbar_w / 2.0,
            };
            unsafe { target.FillRoundedRectangle(&track_rect, tb) };
        }

        // Draw scrollbar thumb
        let thumb_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 80), None) };
        if let Ok(ref hb) = thumb_brush {
            let thumb_rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(
                    (layout.grid_left + cols * cell_w) as f32 + scale_for_dpi(2, dpi) as f32,
                    thumb_y,
                    (layout.grid_left + cols * cell_w) as f32 + scale_for_dpi(2, dpi) as f32 + scrollbar_w,
                    thumb_y + thumb_h,
                ),
                radiusX: scrollbar_w / 2.0,
                radiusY: scrollbar_w / 2.0,
            };
            unsafe { target.FillRoundedRectangle(&thumb_rect, hb) };
        }
    }
}

/// 使用 Direct2D 绘制图标（HICON → GetDIBits → premultiplied D2D Bitmap → DrawBitmap）
///
/// 绕过 WIC，直接从 HICON 的颜色位图读取原始 BGRA 像素，
/// 手动转换为 premultiplied alpha 后创建 D2D bitmap，确保边缘透明过渡正确。
fn draw_icon_d2d(
    target: &ID2D1DCRenderTarget,
    _wic_factory: Option<&IWICImagingFactory>,
    hicon: HICON,
    x: i32,
    y: i32,
    size: i32,
) {
    // 1. 从 HICON 提取颜色位图
    let mut icon_info = ICONINFO {
        f_icon: 0,
        x_hotspot: 0,
        y_hotspot: 0,
        hbm_mask: std::ptr::null_mut(),
        hbm_color: std::ptr::null_mut(),
    };
    if unsafe { GetIconInfo(hicon, &mut icon_info) } == 0 {
        return;
    }

    // 必须有颜色位图（32-bit ARGB）；monochrome mask 无法正确渲染
    if icon_info.hbm_color.is_null() {
        if !icon_info.hbm_mask.is_null() {
            unsafe { DeleteObject(icon_info.hbm_mask as HGDIOBJ) };
        }
        return;
    }

    // 2. 用 GetDIBits 读取原始 BGRA 像素
    //    第一次查询：bi_bit_count 必须为 0，让系统返回实际格式
    let mut bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: 0,
        bi_height: 0,
        bi_planes: 0,
        bi_bit_count: 0, // 关键：查询时必须为 0
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };

    // 第一次调用获取位图尺寸和格式
    if unsafe {
        GetDIBits(
            screen_dc,
            icon_info.hbm_color,
            0,
            0,
            std::ptr::null_mut(),
            &mut bi as *mut BITMAPINFOHEADER as *mut c_void,
            DIB_RGB_COLORS,
        )
    } == 0
    {
        log::warn!("GetDIBits 查询失败");
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    let bmp_w = bi.bi_width;
    let bmp_h = bi.bi_height.abs(); // 可能是负值（top-down DIB）

    if bmp_w <= 0 || bmp_h <= 0 || bi.bi_bit_count != 32 {
        log::debug!(
            "图标位图格式不支持: {}x{} {}bpp",
            bmp_w,
            bmp_h,
            bi.bi_bit_count
        );
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    // 分配像素缓冲区
    let pixel_count = (bmp_w as usize) * (bmp_h as usize);
    let mut pixels = vec![
        Pixel32 {
            b: 0,
            g: 0,
            r: 0,
            a: 0
        };
        pixel_count
    ];

    // 设置 bi_height 为负值以请求 top-down 输出，确保 bi_bit_count = 32
    bi.bi_height = -bmp_h;
    bi.bi_bit_count = 32;
    bi.bi_compression = 0;

    // 第二次调用读取像素数据
    if unsafe {
        GetDIBits(
            screen_dc,
            icon_info.hbm_color,
            0,
            bmp_h as u32,
            pixels.as_mut_ptr() as *mut c_void,
            &mut bi as *mut BITMAPINFOHEADER as *mut c_void,
            DIB_RGB_COLORS,
        )
    } == 0
    {
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    unsafe { ReleaseDC(std::ptr::null_mut(), screen_dc) };

    // 3. 将 straight alpha (BGRA) 转换为 premultiplied alpha
    //    HBITMAP 像素是 straight alpha: RGB 是原始颜色，A 是透明度
    //    D2D 需要 premultiplied: RGB' = RGB * A/255
    for p in pixels.iter_mut() {
        let a = p.a as u16;
        p.b = ((p.b as u16 * a) / 255) as u8;
        p.g = ((p.g as u16 * a) / 255) as u8;
        p.r = ((p.r as u16 * a) / 255) as u8;
    }

    // 4. 创建 D2D bitmap（显式 premultiplied alpha 格式）
    let bitmap_props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };

    let d2d_bitmap = unsafe {
        target.CreateBitmap(
            D2D_SIZE_U {
                width: bmp_w as u32,
                height: bmp_h as u32,
            },
            Some(pixels.as_ptr() as *const c_void),
            (bmp_w as u32) * 4, // pitch = width * bytes_per_pixel
            &bitmap_props,
        )
    };

    if let Ok(ref bmp) = d2d_bitmap {
        let dest_rect = d2d_rect(x as f32, y as f32, (x + size) as f32, (y + size) as f32);
        unsafe {
            target.DrawBitmap(
                bmp,
                Some(&dest_rect),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }
    }

    // 5. 清理 GDI 位图资源
    if !icon_info.hbm_color.is_null() {
        unsafe { DeleteObject(icon_info.hbm_color as HGDIOBJ) };
    }
    if !icon_info.hbm_mask.is_null() {
        unsafe { DeleteObject(icon_info.hbm_mask as HGDIOBJ) };
    }
}

/// 从 HWND 获取 GridWindowData 引用（d2d_render 内部使用）
unsafe fn get_data<'a>(hwnd: HWND) -> Option<&'a mut GridWindowData> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

// =============================================================================
// 拖拽覆盖位图渲染（D2D + DirectWrite，高质量）
// =============================================================================

/// 为拖拽覆盖窗口渲染复合位图（图标 + 标签文字）
///
/// 使用 Direct2D + DirectWrite 渲染，与分类框内图标标签质量一致。
/// 返回 32-bit ARGB HBITMAP（premultiplied alpha），
/// 可直接用于 `UpdateLayeredWindow`。
///
/// # 参数
/// - `icon_handle`: 图标句柄（128×128 高清图标）
/// - `icon_size`: 图标渲染尺寸（DPI 自适应）
/// - `display_name`: 标签文字
/// - `total_w`: 位图总宽度
/// - `total_h`: 位图总高度（图标 + gap + 标签区域）
/// - `gap`: 图标与标签之间的间距（DPI 自适应，6px at 96 DPI）
/// - `label_h`: 标签区域高度（2 行文字高度，36px at 96 DPI）
/// - `dpi`: 当前 DPI（用于字体缩放）
pub fn render_drag_overlay_bitmap(
    icon_handle: HICON,
    icon_size: i32,
    display_name: &str,
    total_w: i32,
    total_h: i32,
    gap: i32,
    label_h: i32,
    dpi: u32,
) -> HBITMAP {
    unsafe {
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return std::ptr::null_mut();
        }

        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return std::ptr::null_mut();
        }

        // 创建 32-bit top-down DIB Section
        let bi = BITMAPINFOHEADER {
            bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            bi_width: total_w,
            bi_height: -total_h,
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
        let dib = CreateDIBSection(
            mem_dc,
            &bi as *const BITMAPINFOHEADER as *const c_void,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );

        if dib.is_null() || bits.is_null() {
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return std::ptr::null_mut();
        }

        let old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);

        // 创建临时 D2D DC Render Target
        let d2d_factory =
            match D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("拖拽覆盖: D2D Factory 创建失败: {:?}", e);
                    SelectObject(mem_dc, old_bmp);
                    DeleteObject(dib as HGDIOBJ);
                    DeleteDC(mem_dc);
                    ReleaseDC(std::ptr::null_mut(), screen_dc);
                    return std::ptr::null_mut();
                }
            };

        let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };

        let target = match d2d_factory.CreateDCRenderTarget(&rt_props) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("拖拽覆盖: DC RenderTarget 创建失败: {:?}", e);
                SelectObject(mem_dc, old_bmp);
                DeleteObject(dib as HGDIOBJ);
                DeleteDC(mem_dc);
                ReleaseDC(std::ptr::null_mut(), screen_dc);
                return std::ptr::null_mut();
            }
        };

        // 绑定 D2D render target 到 memory DC
        let bind_rect = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: total_w,
            bottom: total_h,
        };
        if target.BindDC(GdiHDC(mem_dc), &bind_rect).is_err() {
            log::warn!("拖拽覆盖: BindDC 失败");
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return std::ptr::null_mut();
        }

        // ===== D2D 渲染开始 =====
        target.BeginDraw();

        // 清空为全透明
        target.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        // 绘制图标（居中于覆盖窗口宽度内）
        if !icon_handle.is_null() {
            let icon_x = (total_w - icon_size) / 2; // 水平居中
            draw_icon_d2d(&target, None, icon_handle, icon_x, 0, icon_size);
        }

        // 绘制标签文字（DirectWrite，与分类框一致）
        if !display_name.is_empty() {
            if let Ok(dw_factory) =
                DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)
            {
                let label_font_size = scale_for_dpi(13, dpi) as f32;
                let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
                let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();

                if let Ok(text_format) = dw_factory.CreateTextFormat::<_, _, _>(
                    windows::core::PCWSTR(font_name.as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    label_font_size,
                    windows::core::PCWSTR(locale.as_ptr()),
                ) {
                    let _ = text_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                    let _ = text_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
                    let _ = text_format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);

                    // 标签位置：图标下方 + 间距，两侧各 2px padding
                    let label_x = 2.0_f32;
                    let label_y = (icon_size + gap) as f32;
                    let label_w = (total_w - 4) as f32;

                    let text_wide: Vec<u16> = display_name.encode_utf16().collect();
                    if let Some(layout) = target_inner_create_text_layout(
                        &dw_factory,
                        &text_wide,
                        &text_format,
                        label_w,
                        label_h as f32,
                        DWRITE_TEXT_ALIGNMENT_CENTER,
                        DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
                    ) {
                        // 应用 word-level 省略号裁剪（与分类框未选中项一致）
                        if let Ok(ellipsis) = dw_factory.CreateEllipsisTrimmingSign(&text_format) {
                            let trimming = DWRITE_TRIMMING {
                                granularity: DWRITE_TRIMMING_GRANULARITY_WORD,
                                delimiter: 0,
                                delimiterCount: 0,
                            };
                            let _ = layout.SetTrimming(&trimming, &ellipsis);
                        }

                        if let Ok(brush) =
                            target.CreateSolidColorBrush(&d2d_color(250, 250, 255, 255), None)
                        {
                            // 4 方向阴影描边（与分类框一致）
                            let shadow_offset = scale_for_dpi(1, dpi) as f32;
                            if let Ok(shadow) =
                                target.CreateSolidColorBrush(&d2d_color(0, 0, 0, 200), None)
                            {
                                for &(dx, dy) in &[
                                    (shadow_offset, 0.0),
                                    (-shadow_offset, 0.0),
                                    (0.0, shadow_offset),
                                    (0.0, -shadow_offset),
                                ] {
                                    target.DrawTextLayout(
                                        D2D_POINT_2F {
                                            x: label_x + dx,
                                            y: label_y + dy,
                                        },
                                        &layout,
                                        &shadow,
                                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                                    );
                                }
                            }
                            // 主文字（浅灰白色）
                            target.DrawTextLayout(
                                D2D_POINT_2F {
                                    x: label_x,
                                    y: label_y,
                                },
                                &layout,
                                &brush,
                                D2D1_DRAW_TEXT_OPTIONS_NONE,
                            );
                        }
                    }
                }
            }
        }

        let _ = target.EndDraw(None, None);

        // 清理
        SelectObject(mem_dc, old_bmp);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);

        dib
    }
}
