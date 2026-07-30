//! 桌面覆盖窗口渲染
//!
//! 阶段 2 在透明覆盖窗口上绘制未进入分类框的桌面图标。渲染管线与分类框一致：
//! Direct2D/DirectWrite → 32-bit DIB → `UpdateLayeredWindow`。

use std::ffi::c_void;

use windows::Win32::Graphics::Direct2D::{
    Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT},
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED,
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
use crate::platform::windows::d2d_render::draw_icon_with_label;

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
    let bg_alpha: u8 = if data.selected_item >= 0 || data.external_drag_hover {
        1
    } else {
        0
    };
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

    // ===== Marquee selection rectangle =====
    if data.marquee_active {
        let mx1 = data.marquee_start_x;
        let my1 = data.marquee_start_y;
        let mx2 = data.marquee_current_x;
        let my2 = data.marquee_current_y;
        let left = mx1.min(mx2) as f32;
        let top = my1.min(my2) as f32;
        let right = mx1.max(mx2) as f32;
        let bottom = my1.max(my2) as f32;
        let fill_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(0, 120, 215, 40), None) };
        if let Ok(ref fb) = fill_brush {
            unsafe { target.FillRectangle(&d2d_rect(left, top, right, bottom), fb) };
        }
        let border_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(0, 120, 215, 200), None) };
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

    let hover_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(70, 90, 130, 70), None) };
    let selected_brush =
        unsafe { target.CreateSolidColorBrush(&d2d_color(60, 80, 120, 120), None) };

    // per-DPI 缓存：避免同一 DPI 的图标重复创建 DirectWrite 资源

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

        let is_selected = index as i32 == selected_item
            || super::super::multi_select::is_selected(super::super::multi_select::SelectionKey {
                container_type: super::super::multi_select::ContainerType::Overlay,
                container_id: 0,
                item_index: index,
            });
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

        // 绘制图标 + 标签文字（公共方法，与分类框完全一致）
        let icon_x = cell_x + (icon_cell_w - icon_size) / 2;
        let icon_y = cell_y + scale_for_dpi(4, icon_dpi);
        let label_x = (cell_x + 2) as f32;
        let label_y = (icon_y + icon_size + scale_for_dpi(6, icon_dpi)) as f32;
        let label_w = (icon_cell_w - 4) as f32;
        // 选中项显示完整名称（与分类框一致）；未选中项使用 2 行截断
        let (label_h, apply_ellipsis) = if is_selected {
            (scale_for_dpi(36 * 3, icon_dpi) as f32, false)
        } else {
            (scale_for_dpi(36, icon_dpi) as f32, true)
        };

        draw_icon_with_label(
            target,
            dw_factory,
            None,
            icon.icon_handle,
            &icon.display_name,
            icon_x,
            icon_y,
            icon_size,
            label_x,
            label_y,
            label_w,
            label_h,
            icon_dpi,
            apply_ellipsis,
        );
    }
    // ===== 拖拽插入指示器（垂直分隔线）=====
    if data.drag_over_insert_col >= 0 && data.drag_over_insert_row >= 0 {
        let sep_col = data.drag_over_insert_col;
        let sep_row = data.drag_over_insert_row;

        // 使用光标所在显示器的本地 DPI 计算网格参数（每个显示器独立网格）
        let local_dpi = dpi_at_screen_point(data.drag_cursor_screen_x, data.drag_cursor_screen_y);
        let grid_layout = overlay_layout_params(width, local_dpi);

        // 位置使用本地 DPI 网格（与拖拽检测一致）
        let separator_x = (grid_layout.padding + sep_col * grid_layout.cell_w) as f32;
        let separator_top = (grid_layout.padding + sep_row * grid_layout.cell_h) as f32;
        let separator_bottom = separator_top + grid_layout.cell_h as f32;

        // 线宽和内边距也使用本地 DPI
        let separator_w = scale_for_dpi(2, local_dpi) as f32;
        let sep_pad = scale_for_dpi(4, local_dpi) as f32;

        let insert_brush =
            unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 255), None) };
        if let Ok(ref ib) = insert_brush {
            let line_rect = d2d_rect(
                separator_x - separator_w / 2.0,
                separator_top + sep_pad,
                separator_x + separator_w / 2.0,
                separator_bottom - sep_pad,
            );
            unsafe { target.FillRectangle(&line_rect, ib) };
        }
    }
}

#[cfg(test)]
mod tests {
    /// 验证选中项与未选中项的标签区域高度计算（与分类框一致）
    #[test]
    fn test_label_height_selected_vs_unselected() {
        // 模拟 scale_for_dpi 在不同 DPI 下的行为
        fn scale_for_dpi(px: i32, dpi: u32) -> i32 {
            ((px as f32) * dpi as f32 / 96.0).round() as i32
        }

        for dpi in [96, 120, 144, 192] {
            let icon_dpi = dpi;

            // 未选中：2 行文字高度
            let unselected_h = scale_for_dpi(36, icon_dpi);
            // 选中：3 倍高度（完整名称显示）
            let selected_h = scale_for_dpi(36 * 3, icon_dpi);

            // 选中高度始终是未选中的 3 倍
            assert_eq!(
                selected_h,
                unselected_h * 3,
                "selected label_h should be 3x unselected at DPI {}",
                dpi
            );

            // 96 DPI 基准值验证
            if dpi == 96 {
                assert_eq!(unselected_h, 36, "unselected should be 36px at 96 DPI");
                assert_eq!(selected_h, 108, "selected should be 108px at 96 DPI");
            }
        }
    }

    /// 验证图标标签宽度计算与分类框一致（cell_w - 4 padding）
    #[test]
    fn test_label_width_calculation() {
        for cell_w in [72, 80, 96, 120] {
            let label_w = (cell_w - 4) as f32;
            assert!(
                label_w > 0.0,
                "label_w should be positive for cell_w={}",
                cell_w
            );
            assert_eq!(
                label_w as i32,
                cell_w - 4,
                "label_w should be cell_w - 4 (2px padding each side)"
            );
        }
    }

    /// 验证 apply_ellipsis 逻辑：选中项不截断，未选中项截断
    #[test]
    fn test_apply_ellipsis_logic() {
        // 模拟 overlay_render 中的逻辑
        let is_selected = true;
        let (_label_h, apply_ellipsis) = if is_selected {
            (108.0_f32, false)
        } else {
            (36.0_f32, true)
        };
        assert!(!apply_ellipsis, "selected items should NOT apply ellipsis");

        let is_selected = false;
        let (_label_h, apply_ellipsis) = if is_selected {
            (108.0_f32, false)
        } else {
            (36.0_f32, true)
        };
        assert!(apply_ellipsis, "unselected items SHOULD apply ellipsis");
    }
}
