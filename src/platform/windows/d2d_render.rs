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
//! HICON 转换为 D2D 位图时会裁剪外围透明画布，并在目标图标区域内保持
//! 内容宽高比居中；图标区域尺寸仍由 `ViewParams`/`GridLayout` 决定。
//!
//! 支持 4 种图标模式（ExtraLarge/Large/Medium/Small），未来扩展列表/详细信息/平铺/内容。

use crate::box_core::layout_calc::{calculate_grid, GridLayout};
use crate::box_core::view_engine::{get_view_params, ViewParams};

use super::grid_window::GridFileItem;
use super::icon_bitmap::{icon_destination_rect, visible_pixel_bounds};

use std::collections::HashMap;
use std::ffi::c_void;

use windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED;
use windows::Win32::Graphics::Direct2D::{
    Common::{D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_SIZE_U},
    ID2D1Bitmap, ID2D1DCRenderTarget, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;
use windows::Win32::Graphics::Imaging::IWICImagingFactory;
use windows_numerics::{Matrix3x2, Vector2};

use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::GridWindowData;
use super::render_devices::TextFormatSpec;

/// 初始化窗口的 D2D DC 渲染目标（首次渲染时调用）。
///
/// Direct2D / DirectWrite / WIC 三个工厂已收敛到 render_devices 按渲染线程共享：
/// 每个 ID2D1Factory 内部都持有一个独立 D3D11 设备，按窗口各建一套会让
/// 5 个分类框额外占用约 200MB 私有内存。
///
/// DC 渲染目标仍按窗口持有：它需要与各窗口自己的 DIB 绑定，
/// 共享同一个 target 会在并发 BindDC 时互相踩画面。
pub fn init_d2d_devices(data: &mut GridWindowData) {
    if data.d2d_target.is_some() {
        return;
    }
    if let Some(target) = super::render_devices::create_dc_render_target(0.0) {
        data.d2d_target = Some(target);
        log::info!("D2D DC RenderTarget 已创建（共享 D2D 工厂）");
    }
}

/// 丢弃图标位图缓存中已不再被任何条目引用的 HICON。
///
/// 缓存以 `HICON` 地址为键，而图标句柄在条目被删除 / 替换时就会
/// `DestroyIcon`。旧实现从不清理，于是：
/// - 缓存单调增长，长期运行后每个分类框都背着一堆无主位图；
/// - 句柄地址被系统回收复用后会命中过期条目，画出错误图标。
///
/// 键空间实际上只有「可见图标数 × 图标尺寸种类」（尺寸同一时刻只有一种），
/// 因此只在超出预算时才做一次全量剪枝，正常帧零开销。
fn prune_icon_bitmap_cache(data: &mut GridWindowData) {
    // 预留 4 倍余量：切换视图模式会短暂共存新旧两种图标尺寸
    let budget = data.items.len() * 4 + 16;
    if data.icon_bitmap_cache.len() <= budget {
        return;
    }
    let live: Vec<usize> = data
        .items
        .iter()
        .map(|item| item.icon_handle as usize)
        .collect();
    let before = data.icon_bitmap_cache.len();
    data.icon_bitmap_cache
        .retain(|(hicon, _), _| live.contains(hicon));
    log::debug!(
        "图标位图缓存剪枝: {} → {} 条",
        before,
        data.icon_bitmap_cache.len()
    );
}

/// 释放分类框复用的 DIB 渲染后端。
///
/// 先把 D2D DC 渲染目标从旧 DC 解绑，再还原默认位图并删除 GDI 对象，
/// 避免 D2D 继续向已删除的 DC 提交画面。窗口销毁和尺寸变化时调用。
pub(crate) fn release_grid_dib(data: &mut GridWindowData) {
    if data.dib_dc.is_null() {
        return;
    }
    if let Some(ref target) = data.d2d_target {
        let empty = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let _ = unsafe { target.BindDC(GdiHDC(std::ptr::null_mut()), &empty) };
    }
    unsafe {
        if !data.dib_old_bmp.is_null() {
            SelectObject(data.dib_dc, data.dib_old_bmp);
        }
        if !data.dib_section.is_null() {
            DeleteObject(data.dib_section as HGDIOBJ);
        }
        DeleteDC(data.dib_dc);
    }
    data.dib_section = std::ptr::null_mut();
    data.dib_dc = std::ptr::null_mut();
    data.dib_old_bmp = std::ptr::null_mut();
    data.dib_w = 0;
    data.dib_h = 0;
}

/// 确保分类框持有一个与 width × height 匹配的 DIB 渲染后端。
///
/// 尺寸一致时直接复用（绝大多数帧命中这条路径）；尺寸变化时先释放旧后端再重建。
fn ensure_grid_dib(data: &mut GridWindowData, width: i32, height: i32) -> bool {
    if !data.dib_dc.is_null() && data.dib_w == width && data.dib_h == height {
        return true;
    }
    release_grid_dib(data);
    if width <= 0 || height <= 0 {
        return false;
    }

    unsafe {
        // 屏幕 DC 仅用于创建兼容 DC，创建完成后立即归还
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return false;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        if mem_dc.is_null() {
            return false;
        }

        let bi = BITMAPINFOHEADER {
            bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            bi_width: width,
            bi_height: -height, // 自上而下
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
        let dib = CreateDIBSection(
            mem_dc,
            &bi as *const BITMAPINFOHEADER as *const c_void,
            DIB_RGB_COLORS,
            &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
            std::ptr::null_mut(),
            0,
        );
        if dib.is_null() || bits.is_null() {
            DeleteDC(mem_dc);
            return false;
        }

        data.dib_old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);
        data.dib_section = dib;
        data.dib_dc = mem_dc;
        data.dib_w = width;
        data.dib_h = height;
        true
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
    static LAYERED_SUBMIT_FAILED: AtomicBool = AtomicBool::new(false);
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

    // 确保窗口的 DC 渲染目标已初始化（D2D/DW/WIC 工厂按渲染线程共享）
    init_d2d_devices(data);
    // 记一笔「已渲染」，下一个 200ms tick 据此判断能不能跳帧
    super::grid_window::note_grid_frame_rendered(data);
    prune_icon_bitmap_cache(data);

    let (Some(dw_factory), Some(wic_factory), Some(target)) = (
        super::render_devices::dw_factory(),
        super::render_devices::wic_factory(),
        data.d2d_target.clone(),
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
    let outer_w = wr.right - wr.left;
    let outer_h = wr.bottom - wr.top;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    // 留白取窗口当前生效值，不能每帧重新按 DPI 推算：
    // 玻璃开关刚切换、窗口外框还没收缩完时，重新推算会让本帧
    // 按新留白在旧外框上作画，表现为内容整体错位一帧。
    let shadow_padding = data.render_padding;
    let w = outer_w - shadow_padding * 2;
    let h = outer_h - shadow_padding * 2;
    if w <= 0 || h <= 0 || outer_w <= 0 || outer_h <= 0 {
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    let ww = w as f32;
    let hh = h as f32;

    // ===== 复用的 32 位 ARGB DIB Section（D2D 渲染后端）=====
    //
    // 旧实现每帧 CreateDIBSection + DeleteObject：分类框定时器 200ms 一次、
    // 5 个框即每秒上百 MB 的 GDI 分配/释放，是主进程内存持续爬升的主因之一。
    // 现在 DIB 与内存 DC 存放在 GridWindowData 中，仅尺寸变化或窗口销毁时重建。
    if !ensure_grid_dib(data, outer_w, outer_h) {
        log::error!("分类框 DIB 渲染后端创建失败: {outer_w}x{outer_h}");
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }
    let mem_dc = data.dib_dc;

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    if screen_dc.is_null() {
        log::error!("分类框渲染获取屏幕 DC 失败");
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    // 绑定 D2D render target 到 memory DC
    let bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: outer_w,
        bottom: outer_h,
    };
    if unsafe { target.BindDC(GdiHDC(mem_dc), &bind_rect) }.is_err() {
        log::error!("BindDC 失败");
        release_grid_dib(data);
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    // ===== D2D 渲染开始 =====
    unsafe { target.BeginDraw() };

    let identity = Matrix3x2 {
        M11: 1.0,
        M22: 1.0,
        ..Default::default()
    };
    unsafe { target.SetTransform(&identity) };

    // 清空为全透明（premultiplied alpha: 所有通道为 0）
    let transparent = D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    unsafe { target.Clear(Some(&transparent)) };

    // ===== 绘制液态玻璃背景；GPU 捕获不可用时回退原半透明背景 =====
    let box_style = super::liquid_glass::get_box_style_config();
    let glass_config = box_style.liquid_glass.clone();
    let dpi_scale = dpi as f32 / 96.0;
    // border_radius 为逻辑像素值；render_background 内部会自行乘以 dpi_scale
    let corner_radius = box_style.border_radius;
    // D2D 坐标系为物理像素，回退路径需手动缩放以与液态玻璃路径保持一致
    let fallback_radius = box_style.border_radius * dpi_scale;
    let glass_drawn = if glass_config.enabled {
        draw_liquid_glass_background(
            data,
            &target,
            wr.left,
            wr.top,
            w,
            h,
            shadow_padding,
            corner_radius,
            dpi_scale,
            &glass_config,
        )
    } else {
        false
    };
    if !glass_drawn {
        let [red, green, blue] = box_style.background_color;
        let bg_brush = unsafe {
            target.CreateSolidColorBrush(
                &d2d_color(red, green, blue, box_style.background_alpha),
                None,
            )
        };
        if let Ok(ref brush) = bg_brush {
            let rounded_rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(
                    shadow_padding as f32,
                    shadow_padding as f32,
                    shadow_padding as f32 + ww,
                    shadow_padding as f32 + hh,
                ),
                radiusX: fallback_radius,
                radiusY: fallback_radius,
            };
            unsafe { target.FillRoundedRectangle(&rounded_rect, brush) };
        }
    }

    // 后续标题、图标与交互反馈继续使用原有内容坐标系；仅在提交位图时
    // 平移到透明外投影留白之内。
    let content_transform = Matrix3x2 {
        M11: 1.0,
        M22: 1.0,
        M31: shadow_padding as f32,
        M32: shadow_padding as f32,
        ..Default::default()
    };
    unsafe { target.SetTransform(&content_transform) };

    // ===== 绘制标题栏与主体的分隔线 =====
    let title_h = data.title_h;
    let sep_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(100, 100, 100, 100), None) };
    if let Ok(ref brush) = sep_brush {
        let sep_rect = d2d_rect(0.0, (title_h - 1) as f32, ww, title_h as f32);
        unsafe { target.FillRectangle(&sep_rect, brush) };
    }

    // ===== 创建文字画刷（标题专用，图标标签画刷在 render_icon_grid 内创建）=====
    let white_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 255), None) };
    let shadow_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(0, 0, 0, 200), None) };

    // ===== 标题文字（DirectWrite 渲染）=====
    let title_font_size = scale_for_dpi(15, dpi) as f32;
    // 标题格式：雅黑 UI + 粗体 + 不换行；对齐沿用 DirectWrite 默认的 LEADING / NEAR
    let title_spec = TextFormatSpec::new(
        "Microsoft YaHei UI",
        title_font_size,
        DWRITE_FONT_WEIGHT_BOLD,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_TEXT_ALIGNMENT_LEADING,
        DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
        DWRITE_WORD_WRAPPING_NO_WRAP,
    );
    let title_format = super::render_devices::text_format(&title_spec);

    if let Some(ref tf) = title_format {
        // 锁定按钮区域（标题栏右侧）
        let (btn_x, btn_y, btn_size) = super::grid_layout::lock_btn_rect(w, title_h, dpi);

        let title_text: Vec<u16> = data.title.encode_utf16().collect();
        let text_max_w = (btn_x as f32 - 10.0).max(40.0);
        let layout = unsafe {
            target_inner_create_text_layout(
                &dw_factory,
                &title_text,
                tf,
                text_max_w,
                title_h as f32,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
            )
        };

        if let Some(ref layout) = layout {
            let shadow_offset = scale_for_dpi(1, dpi) as f32;
            if let Ok(ref sb) = shadow_brush {
                for &(dx, dy) in &[
                    (shadow_offset, 0.0),
                    (-shadow_offset, 0.0),
                    (0.0, shadow_offset),
                    (0.0, -shadow_offset),
                ] {
                    let origin = Vector2 {
                        X: 10.0 + dx,
                        Y: dy,
                    };
                    unsafe {
                        target.DrawTextLayout(origin, layout, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
            if let Ok(ref wb) = white_brush {
                let origin = Vector2 { X: 10.0, Y: 0.0 };
                unsafe {
                    target.DrawTextLayout(origin, layout, wb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                }
            }
        }

        // ===== 绘制锁定按钮（标题栏右侧）=====
        let lock_icon = if data.locked { "🔒" } else { "🔓" };
        let lock_font_size = scale_for_dpi(14, dpi) as f32;
        let lock_spec = TextFormatSpec::new(
            "Segoe UI Emoji",
            lock_font_size,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
            DWRITE_WORD_WRAPPING_WRAP,
        );
        let lock_format = super::render_devices::text_format(&lock_spec);

        if let Some(ref lf) = lock_format {
            let lock_text: Vec<u16> = lock_icon.encode_utf16().collect();
            let lock_layout = unsafe {
                target_inner_create_text_layout(
                    &dw_factory,
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
                        let origin = Vector2 {
                            X: btn_x as f32 + sdx,
                            Y: btn_y as f32 + sdy,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, ll, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                        }
                    }
                }
                // 锁定按钮主文字
                if let Ok(ref wb) = white_brush {
                    let origin = Vector2 {
                        X: btn_x as f32,
                        Y: btn_y as f32,
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
    let more_spec = TextFormatSpec::new(
        "Segoe UI Symbol",
        more_font_size,
        DWRITE_FONT_WEIGHT_BOLD,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_TEXT_ALIGNMENT_CENTER,
        DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
        DWRITE_WORD_WRAPPING_WRAP,
    );
    let more_format = super::render_devices::text_format(&more_spec);

    if let Some(ref mf) = more_format {
        let more_text: Vec<u16> = more_icon.encode_utf16().collect();
        let more_layout = unsafe {
            target_inner_create_text_layout(
                &dw_factory,
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
                    let origin = Vector2 {
                        X: mbtn_x as f32 + sdx,
                        Y: mbtn_y as f32 + sdy,
                    };
                    unsafe {
                        target.DrawTextLayout(origin, ml, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
            // 主文字
            if let Ok(ref wb) = white_brush {
                let origin = Vector2 {
                    X: mbtn_x as f32,
                    Y: mbtn_y as f32,
                };
                unsafe {
                    target.DrawTextLayout(origin, ml, wb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                }
            }
        }
    }

    // ===== 绘制图标网格（独立方法，参数化设计）=====
    // 通过 view_engine 获取视图参数，layout_calc 计算网格布局
    let mut view_params = get_view_params(&data.view_mode, dpi);
    // 纯图标模式：隐藏图标下方的文本标签（仅显示图标）
    if box_style.icon_only_mode {
        view_params.show_label = false;
    }
    let grid_layout = calculate_grid(w, title_h, &view_params, dpi);

    // 裁剪区域：标题栏下方到窗口底部，防止滚动时图标溢出到标题区域
    let clip_rect = d2d_rect(0.0, title_h as f32, w as f32, h as f32);
    unsafe { target.PushAxisAlignedClip(&clip_rect, D2D1_ANTIALIAS_MODE_ALIASED) };

    // 构建多选参数
    let multi_select_ct = Some(super::multi_select::ContainerType::GridBox);
    let multi_select_id = super::multi_select::container_id_from_uuid(data.config_id);
    let marquee = if data.marquee_active {
        Some((
            data.marquee_start_x,
            data.marquee_start_y,
            data.marquee_current_x,
            data.marquee_current_y,
        ))
    } else {
        None
    };

    render_icon_grid(
        &target,
        &dw_factory,
        &wic_factory,
        &data.items,
        &grid_layout,
        &view_params,
        data.selected_item,
        data.hovered_item,
        dpi,
        data.scroll_offset,
        h,
        data.drag_over_item,
        data.drag_over_insert_col,
        data.drag_over_insert_row,
        data.renaming_item_index,
        &mut data.icon_bitmap_cache,
        multi_select_ct,
        multi_select_id,
        marquee,
    );

    unsafe { target.PopAxisAlignedClip() };

    // ===== 绘制边缘拖拽指示条（未锁定时鼠标悬停边缘显示）=====
    // 边缘条避开圆角区域，避免直角突出于圆角背景。
    if !data.locked && data.hover_edge > 0 {
        let edge_brush =
            unsafe { target.CreateSolidColorBrush(&d2d_color(120, 180, 255, 100), None) };
        if let Ok(ref eb) = edge_brush {
            let border_w = scale_for_dpi(3, dpi) as f32;
            let corner_r = fallback_radius;
            let (l, t, r, b) = compute_edge_rect(data.hover_edge, ww, hh, border_w, corner_r);
            unsafe { target.FillRectangle(&d2d_rect(l, t, r, b), eb) };
        }
    }

    // ===== D2D 渲染结束 =====
    unsafe { target.SetTransform(&identity) };
    let mut tag1: u64 = 0;
    let mut tag2: u64 = 0;
    if let Err(e) = unsafe { target.EndDraw(Some(&mut tag1), Some(&mut tag2)) } {
        log::error!("D2D EndDraw 失败: {:?}", e);
    }

    // ===== 调用 UpdateLayeredWindow 提交渲染结果 =====
    let size = POINT {
        x: outer_w,
        y: outer_h,
    };
    let src_pt = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 255,
        alpha_format: AC_SRC_ALPHA,
    };

    let submit_result = unsafe {
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
        )
    };
    if submit_result == 0 {
        let error_code = unsafe { GetLastError() };
        if !LAYERED_SUBMIT_FAILED.swap(true, Ordering::Relaxed) {
            log::error!(
                "UpdateLayeredWindow 提交分类框画面失败: hwnd={:?}, size={}x{}, error={}",
                hwnd,
                outer_w,
                outer_h,
                error_code
            );
        }
    } else if LAYERED_SUBMIT_FAILED.swap(false, Ordering::Relaxed) {
        log::info!("UpdateLayeredWindow 提交分类框画面已恢复: hwnd={:?}", hwnd);
    }

    // DIB 与内存 DC 保留给下一帧复用，仅在尺寸变化或窗口销毁时释放
    unsafe {
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }

    RENDERING.store(false, Ordering::SeqCst);
}
/// 把液态玻璃表面画到分层窗口上。
///
/// `data.glass_pixels` 是跨帧复用的 GPU 回读缓冲，避免每帧新分配
/// 一块 1~5MB 的 Vec。
///
/// D2D 位图则**必须**每帧重建，不能跨帧缓存：分类框用的是
/// `ID2D1DCRenderTarget`，每帧都重新 `BindDC` 到 DIB；从它派生并长期
/// 持有的位图会钉住上一次绑定的后端表面，让 D2D 无法释放。
/// 实测把位图缓存下来后主进程私有内存反而涨得更快，因此
/// 这里保留「每帧创建、帧末释放」的原有语义。
#[allow(clippy::too_many_arguments)]
fn draw_liquid_glass_background(
    data: &mut GridWindowData,
    target: &ID2D1DCRenderTarget,
    screen_x: i32,
    screen_y: i32,
    width: i32,
    height: i32,
    padding: i32,
    corner_radius: f32,
    dpi_scale: f32,
    glass_config: &crate::config::LiquidGlassConfig,
) -> bool {
    if !super::liquid_glass::render_background(
        screen_x,
        screen_y,
        width,
        height,
        padding,
        corner_radius,
        dpi_scale,
        glass_config,
        &mut data.glass_pixels,
    ) {
        return false;
    }
    // 回读缓冲必须刚好是一张完整表面，否则 pitch 会与位图对不上
    if data.glass_pixels.len() != ((width + padding * 2) * (height + padding * 2)) as usize {
        log::warn!("液态玻璃回读尺寸不匹配: {}", data.glass_pixels.len());
        return false;
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
                width: (width + padding * 2) as u32,
                height: (height + padding * 2) as u32,
            },
            Some(data.glass_pixels.as_ptr().cast::<c_void>()),
            (width + padding * 2) as u32 * std::mem::size_of::<Pixel32>() as u32,
            &bitmap_props,
        )
    };
    let Ok(bitmap) = bitmap else {
        return false;
    };

    unsafe {
        target.DrawBitmap(
            &bitmap,
            Some(&d2d_rect(
                0.0,
                0.0,
                (width + padding * 2) as f32,
                (height + padding * 2) as f32,
            )),
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
    }
    true
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
    drag_over_item: i32,
    drag_over_insert_col: i32,
    drag_over_insert_row: i32,
    renaming_item_index: i32,
    icon_bitmap_cache: &mut HashMap<(usize, i32), ID2D1Bitmap>,
    multi_select_container_type: Option<super::multi_select::ContainerType>,
    multi_select_container_id: u64,
    marquee: Option<(i32, i32, i32, i32)>,
) {
    let icon_size = view_params.icon_size;
    let cell_w = layout.cell_w;
    let cell_h = layout.cell_h;
    let cols = layout.cols;
    let grid_left = layout.grid_left;
    let grid_top = layout.grid_top;
    let icon_label_h = scale_for_dpi(36, dpi); // 2 行文字高度（支持换行）

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
        let is_selected = i as i32 == selected_item
            || multi_select_container_type.is_some_and(|ct| {
                super::multi_select::is_selected(super::multi_select::SelectionKey {
                    container_type: ct,
                    container_id: multi_select_container_id,
                    item_index: i,
                })
            });
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

        // 拖拽覆盖替换高亮（与 hover 背景一致，显示在拖拽项覆盖的目标项上）
        if i as i32 == drag_over_item && !is_selected {
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

        // 绘制图标 + 标签文字（公共方法，与覆盖区域完全一致）
        let icon_x = cell_x + (cell_w - icon_size) / 2;
        let icon_y = cell_y + 4;
        let label_x = (cell_x + 2) as f32;
        let label_y = (icon_y + icon_size + scale_for_dpi(6, dpi)) as f32;
        let label_w = (cell_w - 4) as f32;
        let label_h = if is_selected {
            scale_for_dpi(36 * 3, dpi) as f32
        } else {
            icon_label_h as f32
        };

        if view_params.show_label && i as i32 != renaming_item_index {
            draw_icon_with_label_cached(
                target,
                dw_factory,
                Some(_wic_factory),
                item.icon_handle,
                &item.display_name,
                icon_x,
                icon_y,
                icon_size,
                label_x,
                label_y,
                label_w,
                label_h,
                dpi,
                !is_selected, // 未选中时应用省略号截断
                Some(&mut *icon_bitmap_cache),
            );
        } else if !item.icon_handle.is_null() {
            // 不显示标签时仅绘制图标
            draw_icon_d2d(
                target,
                Some(_wic_factory),
                item.icon_handle,
                icon_x,
                icon_y,
                icon_size,
                Some(&mut *icon_bitmap_cache),
            );
        }
    }

    // ===== 拖拽插入指示器（垂直分隔线）=====
    if drag_over_insert_col >= 0 && drag_over_insert_row >= 0 {
        let separator_x = (grid_left + drag_over_insert_col * cell_w) as f32;
        let separator_top =
            (grid_top + drag_over_insert_row * cell_h) as f32 - scroll_offset as f32;
        let separator_bottom = separator_top + cell_h as f32;
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

    // ===== Draw scrollbar if content overflows =====
    let total_rows = (items.len() as i32 + cols - 1) / cols;
    let total_content_h = total_rows * cell_h;
    let visible_h = window_height - grid_top;
    if total_content_h > visible_h && visible_h > 0 {
        // Calculate scrollbar thumb size and position
        let scrollbar_w = scale_for_dpi(6, dpi) as f32;
        let min_thumb_h = scale_for_dpi(30, dpi) as f32;
        let thumb_h =
            (visible_h as f32 * visible_h as f32 / total_content_h as f32).max(min_thumb_h);
        let max_scroll = total_content_h - visible_h;
        let scroll_ratio = if max_scroll > 0 {
            scroll_offset as f32 / max_scroll as f32
        } else {
            0.0
        };
        let track_h = visible_h as f32 - thumb_h;
        let thumb_y = grid_top as f32 + scroll_ratio * track_h;

        // Draw scrollbar track (subtle background)
        let track_brush =
            unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 15), None) };
        if let Ok(ref tb) = track_brush {
            let track_rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(
                    (layout.grid_left + cols * cell_w) as f32 + scale_for_dpi(2, dpi) as f32,
                    grid_top as f32,
                    (layout.grid_left + cols * cell_w) as f32
                        + scale_for_dpi(2, dpi) as f32
                        + scrollbar_w,
                    window_height as f32,
                ),
                radiusX: scrollbar_w / 2.0,
                radiusY: scrollbar_w / 2.0,
            };
            unsafe { target.FillRoundedRectangle(&track_rect, tb) };
        }

        // Draw scrollbar thumb
        let thumb_brush =
            unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 80), None) };
        if let Ok(ref hb) = thumb_brush {
            let thumb_rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(
                    (layout.grid_left + cols * cell_w) as f32 + scale_for_dpi(2, dpi) as f32,
                    thumb_y,
                    (layout.grid_left + cols * cell_w) as f32
                        + scale_for_dpi(2, dpi) as f32
                        + scrollbar_w,
                    thumb_y + thumb_h,
                ),
                radiusX: scrollbar_w / 2.0,
                radiusY: scrollbar_w / 2.0,
            };
            unsafe { target.FillRoundedRectangle(&thumb_rect, hb) };
        }
    }

    // ===== Marquee selection rectangle =====
    if let Some((mx1, my1, mx2, my2)) = marquee {
        let left = mx1.min(mx2) as f32;
        let top = (my1.min(my2) - scroll_offset) as f32;
        let right = mx1.max(mx2) as f32;
        let bottom = (my1.max(my2) - scroll_offset) as f32;
        // Semi-transparent blue fill
        let fill_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(0, 120, 215, 40), None) };
        if let Ok(ref fb) = fill_brush {
            unsafe { target.FillRectangle(&d2d_rect(left, top, right, bottom), fb) };
        }
        // Blue border (1px)
        let border_brush =
            unsafe { target.CreateSolidColorBrush(&d2d_color(0, 120, 215, 200), None) };
        if let Ok(ref bb) = border_brush {
            let bw = 1.0_f32;
            unsafe {
                target.FillRectangle(&d2d_rect(left, top, right, top + bw), bb);
                target.FillRectangle(&d2d_rect(left, bottom - bw, right, bottom), bb);
                target.FillRectangle(&d2d_rect(left, top, left + bw, bottom), bb);
                target.FillRectangle(&d2d_rect(right - bw, top, right, bottom), bb);
            }
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
    icon_bitmap_cache: Option<&mut HashMap<(usize, i32), ID2D1Bitmap>>,
) {
    let cache_key = (hicon as usize, size);
    if let Some(bitmap) = icon_bitmap_cache
        .as_deref()
        .and_then(|cache| cache.get(&cache_key))
    {
        draw_icon_bitmap(target, bitmap, x, y, size);
        return;
    }

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

    // Remove transparent canvas padding before scaling the icon into its cell.
    let source_w = bmp_w as usize;
    let source_h = bmp_h as usize;
    let (mut pixels, bitmap_w, bitmap_h) = match visible_pixel_bounds(&pixels, source_w, source_h) {
        Some((left, top, width, height)) if width != source_w || height != source_h => {
            let mut cropped = Vec::with_capacity(width * height);
            for row in top..top + height {
                let start = row * source_w + left;
                cropped.extend_from_slice(&pixels[start..start + width]);
            }
            (cropped, width, height)
        }
        _ => (pixels, source_w, source_h),
    };

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
                width: bitmap_w as u32,
                height: bitmap_h as u32,
            },
            Some(pixels.as_ptr() as *const c_void),
            (bitmap_w as u32) * 4, // pitch = width * bytes_per_pixel
            &bitmap_props,
        )
    };

    if let Ok(bmp) = d2d_bitmap {
        draw_icon_bitmap(target, &bmp, x, y, size);
        if let Some(cache) = icon_bitmap_cache {
            cache.insert(cache_key, bmp);
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

fn draw_icon_bitmap(target: &ID2D1DCRenderTarget, bitmap: &ID2D1Bitmap, x: i32, y: i32, size: i32) {
    let bitmap_size = unsafe { bitmap.GetPixelSize() };
    let (left, top, right, bottom) =
        icon_destination_rect(bitmap_size.width, bitmap_size.height, x, y, size);
    let dest_rect = d2d_rect(left, top, right, bottom);
    unsafe {
        target.DrawBitmap(
            bitmap,
            Some(&dest_rect),
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
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

/// 绘制图标及其标签文字（分类框与覆盖区域共用）
///
/// 统一处理：图标绘制（`draw_icon_d2d`）+ DirectWrite 文本渲染（含截断、阴影）。
/// 确保分类框和覆盖区域的图标渲染、文本、大小完全一致。
///
/// # 参数
/// - `target`: D2D render target
/// - `dw_factory`: DirectWrite factory
/// - `wic_factory`: WIC factory（可选，用于图标绘制）
/// - `icon_handle`: 图标句柄（HICON）
/// - `display_name`: 标签文字
/// - `icon_x`, `icon_y`: 图标左上角坐标
/// - `icon_size`: 图标渲染尺寸（正方形）
/// - `label_x`, `label_y`: 标签区域左上角坐标
/// - `label_w`, `label_h`: 标签区域尺寸
/// - `dpi`: 当前 DPI（用于字体缩放和阴影偏移）
/// - `apply_ellipsis`: 是否应用字符级省略号截断（未选中项 = true，选中项 = false）
pub fn draw_icon_with_label(
    target: &ID2D1DCRenderTarget,
    dw_factory: &IDWriteFactory,
    wic_factory: Option<&IWICImagingFactory>,
    icon_handle: HICON,
    display_name: &str,
    icon_x: i32,
    icon_y: i32,
    icon_size: i32,
    label_x: f32,
    label_y: f32,
    label_w: f32,
    label_h: f32,
    dpi: u32,
    apply_ellipsis: bool,
) {
    draw_icon_with_label_cached(
        target,
        dw_factory,
        wic_factory,
        icon_handle,
        display_name,
        icon_x,
        icon_y,
        icon_size,
        label_x,
        label_y,
        label_w,
        label_h,
        dpi,
        apply_ellipsis,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_icon_with_label_cached(
    target: &ID2D1DCRenderTarget,
    dw_factory: &IDWriteFactory,
    wic_factory: Option<&IWICImagingFactory>,
    icon_handle: HICON,
    display_name: &str,
    icon_x: i32,
    icon_y: i32,
    icon_size: i32,
    label_x: f32,
    label_y: f32,
    label_w: f32,
    label_h: f32,
    dpi: u32,
    apply_ellipsis: bool,
    icon_bitmap_cache: Option<&mut HashMap<(usize, i32), ID2D1Bitmap>>,
) {
    unsafe {
        // 1. 绘制图标
        if !icon_handle.is_null() {
            draw_icon_d2d(
                target,
                wic_factory,
                icon_handle,
                icon_x,
                icon_y,
                icon_size,
                icon_bitmap_cache,
            );
        }

        // 2. 绘制标签文字
        if display_name.is_empty() {
            return;
        }

        let label_font_size = scale_for_dpi(13, dpi) as f32;
        // 图标标签格式全进程缓存：每个分类框每帧每个图标都会用到，
        // 旧实现每次重新 CreateTextFormat，是分类框定时器最主要的对象churn来源。
        let label_spec = TextFormatSpec::new(
            "Microsoft YaHei UI",
            label_font_size,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            DWRITE_WORD_WRAPPING_WRAP,
        );
        let Some(text_format) = super::render_devices::text_format(&label_spec) else {
            return;
        };

        let text_wide: Vec<u16> = display_name.encode_utf16().collect();
        let text_layout = {
            target_inner_create_text_layout(
                dw_factory,
                &text_wide,
                &text_format,
                label_w,
                label_h,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            )
        };

        if let Some(ref layout) = text_layout {
            // 应用字符级省略号裁剪
            if apply_ellipsis {
                if let Some(ellipsis) = super::render_devices::ellipsis_sign(&label_spec) {
                    let trimming = DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    };
                    let _ = layout.SetTrimming(&trimming, &ellipsis);
                }
            }

            // 4 方向阴影描边
            let shadow_offset = scale_for_dpi(1, dpi) as f32;
            if let Ok(shadow) = target.CreateSolidColorBrush(&d2d_color(0, 0, 0, 200), None) {
                for &(dx, dy) in &[
                    (shadow_offset, 0.0),
                    (-shadow_offset, 0.0),
                    (0.0, shadow_offset),
                    (0.0, -shadow_offset),
                ] {
                    target.DrawTextLayout(
                        Vector2 {
                            X: label_x + dx,
                            Y: label_y + dy,
                        },
                        layout,
                        &shadow,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }

            // 主文字（浅灰白色）
            if let Ok(brush) = target.CreateSolidColorBrush(&d2d_color(250, 250, 255, 255), None) {
                target.DrawTextLayout(
                    Vector2 {
                        X: label_x,
                        Y: label_y,
                    },
                    layout,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
    } // unsafe
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

        // D2D 工厂与 DC 渲染目标改用全进程共享工厂（见 render_devices）。
        // 旧实现每次重建拖拽位图都新建一个 ID2D1Factory，等价于新建一个 D3D11 设备。
        let Some(target) = super::render_devices::create_dc_render_target(96.0) else {
            log::warn!("拖拽覆盖: 共享 DC RenderTarget 创建失败");
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return std::ptr::null_mut();
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

        // 绘制图标 + 标签文字（公共方法，与分类框完全一致）
        let icon_x = (total_w - icon_size) / 2;
        let label_x = 2.0_f32;
        let label_y = (icon_size + gap) as f32;
        let label_w = (total_w - 4) as f32;

        // DirectWrite 工厂改为全进程共享，避免每次重建拖拽位图都新建一个工厂
        let Some(dw_factory) = super::render_devices::dw_factory() else {
            log::warn!("拖拽覆盖: 共享 DirectWrite Factory 不可用");
            let _ = target.EndDraw(None, None);
            SelectObject(mem_dc, old_bmp);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return dib;
        };

        draw_icon_with_label(
            &target,
            &dw_factory,
            None,
            icon_handle,
            display_name,
            icon_x,
            0,
            icon_size,
            label_x,
            label_y,
            label_w,
            label_h as f32,
            dpi,
            true, // 覆盖区域始终应用省略号截断
        );

        let _ = target.EndDraw(None, None);

        // 清理
        SelectObject(mem_dc, old_bmp);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);

        dib
    }
}

/// 计算边缘拖拽指示条的矩形区域
///
/// 根据 hover_edge 值计算边缘条的矩形，边缘条避开圆角区域以避免视觉冲突。
///
/// # Arguments
/// * `hover_edge` - 边缘标识：0=无, 1=左, 2=右, 3=上, 4=下
/// * `window_width` - 窗口宽度
/// * `window_height` - 窗口高度  
/// * `border_width` - 边缘条宽度
/// * `corner_radius` - 圆角半径
///
/// # Returns
/// (left, top, right, bottom) 格式的矩形坐标
pub fn compute_edge_rect(
    hover_edge: i32,
    window_width: f32,
    window_height: f32,
    border_width: f32,
    corner_radius: f32,
) -> (f32, f32, f32, f32) {
    match hover_edge {
        1 => (
            0.0,
            corner_radius,
            border_width,
            window_height - corner_radius,
        ), // 左边缘
        2 => (
            window_width - border_width,
            corner_radius,
            window_width,
            window_height - corner_radius,
        ), // 右边缘
        3 => (
            corner_radius,
            0.0,
            window_width - corner_radius,
            border_width,
        ), // 上边缘
        4 => (
            corner_radius,
            window_height - border_width,
            window_width - corner_radius,
            window_height,
        ), // 下边缘
        _ => (0.0, 0.0, 0.0, 0.0), // 无边缘
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== compute_edge_rect 单元测试 =====
    // 测试边缘拖拽指示条的矩形计算，验证边缘条避开圆角区域。

    #[test]
    fn test_edge_rect_left_edge() {
        let (l, t, r, b) = compute_edge_rect(1, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 0.0);
        assert_eq!(t, 16.0); // 避开上方圆角
        assert_eq!(r, 3.0);
        assert_eq!(b, 584.0); // 避开下方圆角 (600 - 16)
    }

    #[test]
    fn test_edge_rect_right_edge() {
        let (l, t, r, b) = compute_edge_rect(2, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 397.0); // 400 - 3
        assert_eq!(t, 16.0); // 避开上方圆角
        assert_eq!(r, 400.0);
        assert_eq!(b, 584.0); // 避开下方圆角
    }

    #[test]
    fn test_edge_rect_top_edge() {
        let (l, t, r, b) = compute_edge_rect(3, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 16.0); // 避开左侧圆角
        assert_eq!(t, 0.0);
        assert_eq!(r, 384.0); // 避开右侧圆角 (400 - 16)
        assert_eq!(b, 3.0);
    }

    #[test]
    fn test_edge_rect_bottom_edge() {
        let (l, t, r, b) = compute_edge_rect(4, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 16.0); // 避开左侧圆角
        assert_eq!(t, 597.0); // 600 - 3
        assert_eq!(r, 384.0); // 避开右侧圆角
        assert_eq!(b, 600.0);
    }

    #[test]
    fn test_edge_rect_no_edge() {
        let (l, t, r, b) = compute_edge_rect(0, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 0.0);
        assert_eq!(t, 0.0);
        assert_eq!(r, 0.0);
        assert_eq!(b, 0.0);
    }

    #[test]
    fn test_edge_rect_invalid_edge() {
        let (l, t, r, b) = compute_edge_rect(5, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 0.0);
        assert_eq!(t, 0.0);
        assert_eq!(r, 0.0);
        assert_eq!(b, 0.0);
    }

    #[test]
    fn test_edge_rect_negative_edge() {
        let (l, t, r, b) = compute_edge_rect(-1, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 0.0);
        assert_eq!(t, 0.0);
        assert_eq!(r, 0.0);
        assert_eq!(b, 0.0);
    }

    /// 验证边缘条不超出窗口边界
    #[test]
    fn test_edge_rect_within_bounds() {
        for edge in 1..=4 {
            let (l, t, r, b) = compute_edge_rect(edge, 400.0, 600.0, 3.0, 16.0);
            assert!(l >= 0.0, "left edge {} should be >= 0", l);
            assert!(t >= 0.0, "top edge {} should be >= 0", t);
            assert!(r <= 400.0, "right edge {} should be <= 400", r);
            assert!(b <= 600.0, "bottom edge {} should be <= 600", b);
        }
    }

    /// 验证边缘条避开圆角区域（边缘条起点/终点不应在圆角区域内）
    #[test]
    fn test_edge_rect_avoids_corners() {
        let corner_r = 16.0_f32;
        let ww = 400.0_f32;
        let hh = 600.0_f32;
        let bw = 3.0_f32;

        // 左边缘：top >= corner_r, bottom <= hh - corner_r
        let (_l, t, _r, b) = compute_edge_rect(1, ww, hh, bw, corner_r);
        assert!(t >= corner_r, "left edge top should be >= corner_r");
        assert!(
            b <= hh - corner_r,
            "left edge bottom should be <= hh - corner_r"
        );

        // 右边缘：top >= corner_r, bottom <= hh - corner_r
        let (_l, t, _r, b) = compute_edge_rect(2, ww, hh, bw, corner_r);
        assert!(t >= corner_r, "right edge top should be >= corner_r");
        assert!(
            b <= hh - corner_r,
            "right edge bottom should be <= hh - corner_r"
        );

        // 上边缘：left >= corner_r, right <= ww - corner_r
        let (l, _t, r, _b) = compute_edge_rect(3, ww, hh, bw, corner_r);
        assert!(l >= corner_r, "top edge left should be >= corner_r");
        assert!(
            r <= ww - corner_r,
            "top edge right should be <= ww - corner_r"
        );

        // 下边缘：left >= corner_r, right <= ww - corner_r
        let (l, _t, r, _b) = compute_edge_rect(4, ww, hh, bw, corner_r);
        assert!(l >= corner_r, "bottom edge left should be >= corner_r");
        assert!(
            r <= ww - corner_r,
            "bottom edge right should be <= ww - corner_r"
        );
    }

    /// 验证不同 DPI 缩放下的边缘条宽度计算（scale_for_dpi 逻辑验证）
    #[test]
    fn test_edge_rect_with_dpi_scaled_border() {
        // 模拟 150% DPI：scale_for_dpi(3, 144) = 3 * 144 / 96 = 4
        let border_w_150 = 4.0_f32;
        let (l, _t, r, _b) = compute_edge_rect(1, 400.0, 600.0, border_w_150, 16.0);
        assert_eq!(r - l, 4.0, "border width should match DPI-scaled value");

        // 模拟 200% DPI：scale_for_dpi(3, 192) = 3 * 192 / 96 = 6
        let border_w_200 = 6.0_f32;
        let (l, _t, r, _b) = compute_edge_rect(2, 400.0, 600.0, border_w_200, 16.0);
        assert_eq!(r - l, 6.0, "border width should match DPI-scaled value");
    }

    /// 验证小窗口场景（窗口尺寸接近圆角直径）
    #[test]
    fn test_edge_rect_small_window() {
        // 窗口 40x40，圆角 16：边缘条只有 40-16*2=8 像素可用
        let (_l, t, _r, b) = compute_edge_rect(1, 40.0, 40.0, 3.0, 16.0);
        assert!(
            t < b,
            "left edge should have positive height: t={}, b={}",
            t,
            b
        );
        assert_eq!(b - t, 8.0, "edge height should be 40 - 16*2 = 8");
    }
}
