//! macOS 覆盖窗口渲染

use crate::platform::macos::ffi_imports::*;
use cocoa::base::id;
use cocoa::foundation::{NSPoint, NSRect, NSSize};
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use objc::{msg_send, sel, sel_impl};

use super::overlay_window::ScreenInfo;

const K_CG_BITMAP_INFO: u32 = 2 | 0x2000;

/// 渲染覆盖窗口内容
pub(crate) fn render_overlay_placeholder(window: id, screen_info: &ScreenInfo) {
    unsafe {
        let width = screen_info.frame.size.width;
        let height = screen_info.frame.size.height;
        let scale = screen_info.scale_factor;
        let pixel_w = (width * scale) as usize;
        let pixel_h = (height * scale) as usize;

        log::info!("渲染覆盖窗口占位符: {}x{} @ {:.1}x", width, height, scale);

        // 创建位图上下文
        let color_space = CGColorSpace::create_device_rgb();
        let bytes_per_row = pixel_w * 4;
        let ctx = CGContext::create_bitmap_context(
            None,
            pixel_w,
            pixel_h,
            8,
            bytes_per_row,
            &color_space,
            K_CG_BITMAP_INFO,
        );

        // 绘制半透明蓝色背景
        ctx.set_rgb_fill_color(0.2, 0.5, 0.8, 0.15);
        let rect = CGRect::new(
            &CGPoint::new(0.0, 0.0),
            &CGSize::new(pixel_w as f64, pixel_h as f64),
        );
        ctx.fill_rect(rect);

        // 绘制边框
        ctx.set_rgb_stroke_color(0.0, 0.4, 0.8, 0.3);
        ctx.set_line_width(2.0 * scale);
        let border_rect = CGRect::new(
            &CGPoint::new(1.0, 1.0),
            &CGSize::new(pixel_w as f64 - 2.0, pixel_h as f64 - 2.0),
        );
        ctx.stroke_rect(border_rect);

        // 创建 NSImage
        let image = ctx.create_image().expect("Failed to create image");
        let ns_image: id = msg_send![class!(NSImage), alloc];
        let size = NSSize::new(width, height);
        let ns_image: id = msg_send![ns_image, initWithCGImage:image.as_ptr() size:size];

        // 创建 NSImageView
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);
        let image_view: id = msg_send![class!(NSImageView), alloc];
        let image_view: id = msg_send![image_view, initWithFrame:frame];
        let _: () = msg_send![image_view, setImage:ns_image];
        let _: () = msg_send![image_view, setImageScaling:0]; // NSImageScaleNone

        // 设置为窗口的内容视图
        let _: () = msg_send![window, setContentView:image_view];

        log::info!("覆盖窗口占位符渲染完成");
    }
}
