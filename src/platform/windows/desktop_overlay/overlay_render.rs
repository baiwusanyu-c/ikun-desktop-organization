//! 桌面覆盖窗口渲染
//!
//! 阶段 2 在透明覆盖窗口上绘制未进入分类框的桌面图标。渲染管线与分类框一致：
//! Direct2D/DirectWrite → 32-bit DIB → `UpdateLayeredWindow`。

use std::ffi::c_void;

use windows::Win32::Graphics::Direct2D::{
    Common::{
        D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U,
    },
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_WORD,
    DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use crate::platform::windows::ffi_imports::*;
use crate::platform::windows::ffi_types::*;

use super::overlay_window::{
    dpi_at_screen_point, get_overlay_data, overlay_icon_cell_x, overlay_icon_cell_y,
    overlay_layout_params, OverlayIconItem, OverlayWindowData,
};

/// 初始化覆盖窗口的 D2D/DirectWrite/WIC 设备。
pub(crate) fn init_overlay_d2d_devices(data: &mut OverlayWindowData) {
    if data.d2d_factory.is_none() {
        match unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
        {
            Ok(factory) => data.d2d_factory = Some(factory),
            Err(e) => {
                log::error!("创建覆盖窗口 D2D Factory 失败: {:?}", e);
                return;
            }
        }
    }

    if data.dw_factory.is_none() {
        match unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) } {
            Ok(factory) => data.dw_factory = Some(factory),
            Err(e) => {
                log::error!("创建覆盖窗口 DirectWrite Factory 失败: {:?}", e);
                return;
            }
        }
    }

    if data.wic_factory.is_none() {
        match unsafe {
            CoCreateInstance::<_, IWICImagingFactory>(
                &CLSID_WICImagingFactory,
                None,
                CLSCTX_INPROC_SERVER,
            )
        } {
            Ok(factory) => data.wic_factory = Some(factory),
            Err(e) => {
                log::error!("创建覆盖窗口 WIC Factory 失败: {:?}", e);
                return;
            }
        }
    }

    if data.d2d_target.is_none() {
        if let Some(ref factory) = data.d2d_factory {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            match unsafe { factory.CreateDCRenderTarget(&props) } {
                Ok(target) => {
                    data.d2d_target = Some(target);
                    log::info!("覆盖窗口 D2D DC RenderTarget 已创建");
                }
                Err(e) => log::error!("创建覆盖窗口 D2D DC RenderTarget 失败: {:?}", e),
            }
        }
    }
}

/// 渲染桌面覆盖窗口。
pub(crate) fn render_desktop_overlay(hwnd: HWND) {
    let data = match get_overlay_data(hwnd) {
        Some(data) => data,
        None => return,
    };

    init_overlay_d2d_devices(data);

    let (Some(ref dw_factory), Some(ref target)) = (&data.dw_factory, &data.d2d_target) else {
        log::warn!("覆盖窗口 D2D 设备未初始化，跳过渲染");
        return;
    };

    let w = data.width;
    let h = data.height;
    if w <= 0 || h <= 0 {
        return;
    }

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    let mem_dc = unsafe { CreateCompatibleDC(screen_dc) };
    if mem_dc.is_null() {
        unsafe { ReleaseDC(std::ptr::null_mut(), screen_dc) };
        return;
    }

    let bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: w,
        bi_height: -h,
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
        return;
    }

    let old_bmp = unsafe { SelectObject(mem_dc, dib as HGDIOBJ) };
    let bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    if unsafe { target.BindDC(GdiHDC(mem_dc), &bind_rect) }.is_err() {
        log::error!("覆盖窗口 BindDC 失败");
        unsafe {
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        return;
    }

    unsafe { target.BeginDraw() };
    // 动态背景透明度：有选中项时使用 alpha=1（接收点击），无选中项时使用 alpha=0（穿透到桌面）
    // 这是 WS_EX_LAYERED 窗口的特性：alpha=0 的像素会被系统视为不存在，鼠标事件直接穿透
    let bg_alpha: u8 = if data.selected_item >= 0 { 1 } else { 0 };
    unsafe { target.Clear(Some(&d2d_color(0, 0, 0, bg_alpha))) };
    draw_desktop_icons(
        target,
        dw_factory,
        &data.icons,
        data.width,
        data.dpi,
        data.selected_item,
        data.hovered_item,
        data,
    );

    let mut tag1 = 0;
    let mut tag2 = 0;
    if let Err(e) = unsafe { target.EndDraw(Some(&mut tag1), Some(&mut tag2)) } {
        log::error!("覆盖窗口 D2D EndDraw 失败: {:?}", e);
    }

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
        SelectObject(mem_dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }

    log::info!("覆盖窗口已渲染桌面图标: {} 个", data.icons.len());
}

fn draw_desktop_icons(
    target: &ID2D1DCRenderTarget,
    dw_factory: &IDWriteFactory,
    icons: &[OverlayIconItem],
    width: i32,
    dpi: u32,
    selected_item: i32,
    hovered_item: i32,
    data: &OverlayWindowData,
) {
    // 网格定位仍使用窗口级 DPI（保持网格一致性）
    let grid_layout = overlay_layout_params(width, dpi);

    let shadow_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(0, 0, 0, 220), None) };
    let label_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(250, 250, 255, 255), None) };
    let hover_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(70, 90, 130, 70), None) };
    let selected_brush =
        unsafe { target.CreateSolidColorBrush(&d2d_color(60, 80, 120, 120), None) };

    let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
    let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();

    // per-DPI 缓存：避免同一 DPI 的图标重复创建 DirectWrite 资源
    let mut cached_label_dpi: u32 = 0;
    let mut cached_label_format: Option<windows::Win32::Graphics::DirectWrite::IDWriteTextFormat> =
        None;
    let mut cached_ellipsis_sign: Option<
        windows::Win32::Graphics::DirectWrite::IDWriteInlineObject,
    > = None;

    for (index, icon) in icons.iter().enumerate() {
        let cell_x = overlay_icon_cell_x(data, index, &grid_layout);
        let cell_y = overlay_icon_cell_y(data, index, &grid_layout);

        // 获取该图标所在显示器的 DPI，实现跨显示器自适应缩放
        let icon_screen_x = data.x + cell_x + grid_layout.cell_w / 2;
        let icon_screen_y = data.y + cell_y + grid_layout.cell_h / 2;
        let icon_dpi = dpi_at_screen_point(icon_screen_x, icon_screen_y);

        // 使用 per-icon DPI 计算所有尺寸：图标、单元格、文字、高亮框
        let icon_layout = overlay_layout_params(width, icon_dpi);
        let icon_size = icon_layout.icon_size;
        let icon_cell_w = icon_layout.cell_w;
        let icon_cell_h = icon_layout.cell_h;

        let icon_x = cell_x + (icon_cell_w - icon_size) / 2;
        let icon_y = cell_y + scale_for_dpi(4, icon_dpi);

        let is_selected = index as i32 == selected_item;
        let is_hovered = index as i32 == hovered_item;
        if is_selected || is_hovered {
            let rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(
                    cell_x as f32 + scale_for_dpi(2, icon_dpi) as f32,
                    cell_y as f32 + scale_for_dpi(2, icon_dpi) as f32,
                    (cell_x + icon_cell_w - scale_for_dpi(2, icon_dpi)) as f32,
                    (cell_y + icon_cell_h - scale_for_dpi(2, icon_dpi)) as f32,
                ),
                radiusX: scale_for_dpi(6, icon_dpi) as f32,
                radiusY: scale_for_dpi(6, icon_dpi) as f32,
            };
            if is_selected {
                if let Ok(ref brush) = selected_brush {
                    unsafe { target.FillRoundedRectangle(&rect, brush) };
                }
            } else if let Ok(ref brush) = hover_brush {
                unsafe { target.FillRoundedRectangle(&rect, brush) };
            }
        }

        // 拖拽覆盖替换高亮
        if index as i32 == data.drag_over_item && !is_selected {
            let rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(
                    cell_x as f32 + scale_for_dpi(2, icon_dpi) as f32,
                    cell_y as f32 + scale_for_dpi(2, icon_dpi) as f32,
                    (cell_x + icon_cell_w - scale_for_dpi(2, icon_dpi)) as f32,
                    (cell_y + icon_cell_h - scale_for_dpi(2, icon_dpi)) as f32,
                ),
                radiusX: scale_for_dpi(6, icon_dpi) as f32,
                radiusY: scale_for_dpi(6, icon_dpi) as f32,
            };
            if let Ok(ref brush) = hover_brush {
                unsafe { target.FillRoundedRectangle(&rect, brush) };
            }
        }

        if !icon.icon_handle.is_null() {
            draw_icon_d2d(target, icon.icon_handle, icon_x, icon_y, icon_size);
        }

        // 文字标签：使用 per-icon DPI，按 DPI 缓存 label_format 避免重复创建
        if cached_label_dpi != icon_dpi {
            cached_label_format = unsafe {
                dw_factory
                    .CreateTextFormat::<_, _, _>(
                        windows::core::PCWSTR(font_name.as_ptr()),
                        None,
                        DWRITE_FONT_WEIGHT_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        scale_for_dpi(13, icon_dpi) as f32,
                        windows::core::PCWSTR(locale.as_ptr()),
                    )
                    .ok()
            };
            if let Some(ref format) = cached_label_format {
                let _ = unsafe { format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) };
                let _ = unsafe { format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR) };
                let _ = unsafe { format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP) };
            }
            cached_ellipsis_sign = cached_label_format
                .as_ref()
                .and_then(|format| unsafe { dw_factory.CreateEllipsisTrimmingSign(format).ok() });
            cached_label_dpi = icon_dpi;
        }
        if index as i32 != data.renaming_item_index {
        if let Some(ref format) = cached_label_format {
            let label_h = scale_for_dpi(36, icon_dpi);
            let label: Vec<u16> = icon.display_name.encode_utf16().collect();
            let text_layout = unsafe {
                dw_factory
                    .CreateTextLayout(&label, format, (icon_cell_w - 4) as f32, label_h as f32)
                    .ok()
            };
            if let Some(ref text_layout) = text_layout {
                if let Some(ref sign) = cached_ellipsis_sign {
                    let trimming = DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_WORD,
                        delimiter: 0,
                        delimiterCount: 0,
                    };
                    let _ = unsafe { text_layout.SetTrimming(&trimming, sign) };
                }

                let label_x = cell_x + 2;
                let label_y = icon_y + icon_size + scale_for_dpi(6, icon_dpi);
                let shadow_offset = scale_for_dpi(1, icon_dpi) as f32;
                if let Ok(ref brush) = shadow_brush {
                    for &(dx, dy) in &[
                        (shadow_offset, 0.0),
                        (-shadow_offset, 0.0),
                        (0.0, shadow_offset),
                        (0.0, -shadow_offset),
                    ] {
                        unsafe {
                            target.DrawTextLayout(
                                D2D_POINT_2F {
                                    x: label_x as f32 + dx,
                                    y: label_y as f32 + dy,
                                },
                                text_layout,
                                brush,
                                D2D1_DRAW_TEXT_OPTIONS_NONE,
                            );
                        }
                    }
                }
                if let Ok(ref brush) = label_brush {
                    unsafe {
                        target.DrawTextLayout(
                            D2D_POINT_2F {
                                x: label_x as f32,
                                y: label_y as f32,
                            },
                            text_layout,
                            brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }
            }
        }
        }
    }

    // ===== 拖拽插入指示器（垂直分隔线）=====
    if data.drag_over_insert_col >= 0 && data.drag_over_insert_row >= 0 {
        let grid_layout = overlay_layout_params(width, dpi);
        let sep_col = data.drag_over_insert_col;

        // 分隔线 X/Y 都使用网格计算位置（而非 overlay_icon_cell_x/y 的保存位置），
        // 跟随光标的网格列/行位置移动，高度为单个格子高度 (cell_h)。
        let separator_x = (grid_layout.padding + sep_col * grid_layout.cell_w) as f32;
        let sep_row = data.drag_over_insert_row;
        let separator_top = (grid_layout.padding + sep_row * grid_layout.cell_h) as f32;
        let separator_bottom = separator_top + grid_layout.cell_h as f32;
        let separator_w = scale_for_dpi(1, dpi) as f32;

        let insert_brush =
            unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 255), None) };
        if let Ok(ref ib) = insert_brush {
            let line_rect = d2d_rect(
                separator_x - separator_w / 2.0,
                separator_top + scale_for_dpi(4, dpi) as f32,
                separator_x + separator_w / 2.0,
                separator_bottom - scale_for_dpi(4, dpi) as f32,
            );
            unsafe { target.FillRectangle(&line_rect, ib) };
        }
    }
}

fn draw_icon_d2d(target: &ID2D1DCRenderTarget, hicon: HICON, x: i32, y: i32, size: i32) {
    let mut icon_info = ICONINFO {
        f_icon: 0,
        x_hotspot: 0,
        y_hotspot: 0,
        hbm_mask: std::ptr::null_mut(),
        hbm_color: std::ptr::null_mut(),
    };
    if unsafe { GetIconInfo(hicon, &mut icon_info) } == 0 || icon_info.hbm_color.is_null() {
        if !icon_info.hbm_mask.is_null() {
            unsafe { DeleteObject(icon_info.hbm_mask as HGDIOBJ) };
        }
        return;
    }

    let mut bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: 0,
        bi_height: 0,
        bi_planes: 0,
        bi_bit_count: 0,
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
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
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    let bmp_w = bi.bi_width;
    let bmp_h = bi.bi_height.abs();
    if bmp_w <= 0 || bmp_h <= 0 || bi.bi_bit_count != 32 {
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    let mut pixels = vec![
        Pixel32 {
            b: 0,
            g: 0,
            r: 0,
            a: 0
        };
        (bmp_w * bmp_h) as usize
    ];
    bi.bi_height = -bmp_h;
    bi.bi_bit_count = 32;
    bi.bi_compression = 0;

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

    for pixel in pixels.iter_mut() {
        let alpha = pixel.a as u16;
        pixel.b = ((pixel.b as u16 * alpha) / 255) as u8;
        pixel.g = ((pixel.g as u16 * alpha) / 255) as u8;
        pixel.r = ((pixel.r as u16 * alpha) / 255) as u8;
    }

    let bitmap_props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    let bitmap = unsafe {
        target.CreateBitmap(
            D2D_SIZE_U {
                width: bmp_w as u32,
                height: bmp_h as u32,
            },
            Some(pixels.as_ptr() as *const c_void),
            (bmp_w as u32) * 4,
            &bitmap_props,
        )
    };
    if let Ok(ref bitmap) = bitmap {
        let rect = d2d_rect(x as f32, y as f32, (x + size) as f32, (y + size) as f32);
        unsafe {
            target.DrawBitmap(
                bitmap,
                Some(&rect),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }
    }

    unsafe {
        DeleteObject(icon_info.hbm_color as HGDIOBJ);
        DeleteObject(icon_info.hbm_mask as HGDIOBJ);
    }
}

fn scale_for_dpi(value: i32, dpi: u32) -> i32 {
    ((value as f32) * dpi as f32 / 96.0).round() as i32
}

fn d2d_color(
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
    windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: a as f32 / 255.0,
    }
}

fn d2d_rect(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}
