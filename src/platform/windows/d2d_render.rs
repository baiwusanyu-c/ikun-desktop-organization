//! Direct2D/DirectWrite 渲染代码
//!
//! 本模块包含使用 Direct2D 和 DirectWrite 渲染格子窗口内容的所有代码。
//! 包括设备初始化、分层窗口渲染和图标绘制。

use std::ffi::c_void;

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
    let dpi = if dpi > 0 { dpi } else { 96 };

    // ===== 创建文字画刷（复用于标题和标签）=====
    let white_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 255), None) };
    let shadow_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(0, 0, 0, 200), None) };
    let label_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(250, 250, 255, 255), None) };

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
        let title_text: Vec<u16> = format!("📁 {} ({})", data.title, data.items.len())
            .encode_utf16()
            .collect();

        // 创建 TextLayout
        let layout = unsafe {
            target_inner_create_text_layout(
                dw_factory,
                &title_text,
                tf,
                (w - 20) as f32,
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
    }

    // ===== 绘制图标网格 =====
    let icon_size = data.icon_size;
    let cell_w = data.cell_w;
    let cell_h = data.cell_h;
    let cols = data.cols;
    let grid_left = (w - cols * cell_w) / 2;
    let grid_top = title_h + scale_for_dpi(8, dpi);
    let icon_label_h = scale_for_dpi(36, dpi); // 2 行文字高度（支持换行）

    // 图标标签字体
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

    // 设置标签文字对齐方式
    if let Ok(ref lf) = label_format {
        let _ = unsafe { lf.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) };
        let _ = unsafe { lf.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };
        let _ = unsafe { lf.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP) };
    }

    // 创建省略号裁剪标志（line-clamp 效果：最多 2 行 + "..."）
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

    for (i, item) in data.items.iter().enumerate() {
        let col = (i as i32) % cols;
        let row = (i as i32) / cols;

        let cell_x = grid_left + col * cell_w;
        let cell_y = grid_top + row * cell_h;
        let is_selected = i as i32 == data.selected_item;
        let is_hovered = i as i32 == data.hovered_item;

        // Hover 背景（比选中浅，仅非选中项显示）
        if is_hovered && !is_selected {
            if let Ok(ref hb) = hover_brush {
                let hl_rect = d2d_rect(
                    cell_x as f32,
                    cell_y as f32,
                    (cell_x + cell_w) as f32,
                    (cell_y + cell_h) as f32,
                );
                unsafe { target.FillRectangle(&hl_rect, hb) };
            }
        }

        // 选中项高亮背景
        if is_selected {
            if let Ok(ref hb) = highlight_brush {
                let hl_rect = d2d_rect(
                    cell_x as f32,
                    cell_y as f32,
                    (cell_x + cell_w) as f32,
                    (cell_y + cell_h) as f32,
                );
                unsafe { target.FillRectangle(&hl_rect, hb) };
            }
        }

        // 绘制图标（HICON → GetDIBits → D2D Bitmap → DrawBitmap）
        let icon_x = cell_x + (cell_w - icon_size) / 2;
        let icon_y = cell_y + 4;
        if !item.icon_handle.is_null() {
            draw_icon_d2d(
                target,
                wic_factory,
                item.icon_handle,
                icon_x,
                icon_y,
                icon_size,
            );
        }

        // 绘制图标标签文字（DirectWrite）
        if let Ok(ref lf) = label_format {
            let label: Vec<u16> = item.display_name.encode_utf16().collect();
            let label_x = cell_x + 2;
            let label_y = icon_y + icon_size + scale_for_dpi(6, dpi);
            let label_w = (cell_w - 4) as f32;

            // 选中项：足够高度显示完整名称；未选中：2 行裁剪
            // 两种状态都使用 NEAR（顶部对齐），确保单行文本与图标间距一致
            let (label_h, para_align) = if is_selected {
                let max_h = scale_for_dpi(36 * 3, dpi);
                (max_h as f32, DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
            } else {
                (icon_label_h as f32, DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
            };

            let layout = unsafe {
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

            if let Some(ref layout) = layout {
                // 未选中：应用 line-clamp（2 行 + 省略号）
                if !is_selected {
                    if let Some(ref sign) = ellipsis_sign {
                        let trimming = DWRITE_TRIMMING {
                            granularity: DWRITE_TRIMMING_GRANULARITY_WORD,
                            delimiter: 0,
                            delimiterCount: 0,
                        };
                        let _ = unsafe { layout.SetTrimming(&trimming, sign) };
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
                            target.DrawTextLayout(origin, layout, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
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
                        target.DrawTextLayout(origin, layout, lb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
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

/// 使用 Direct2D 绘制图标（HICON → GetDIBits → premultiplied D2D Bitmap → DrawBitmap）
///
/// 绕过 WIC，直接从 HICON 的颜色位图读取原始 BGRA 像素，
/// 手动转换为 premultiplied alpha 后创建 D2D bitmap，确保边缘透明过渡正确。
fn draw_icon_d2d(
    target: &ID2D1DCRenderTarget,
    _wic_factory: &IWICImagingFactory,
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
