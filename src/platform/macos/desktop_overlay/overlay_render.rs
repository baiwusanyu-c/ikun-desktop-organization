//! 覆盖窗口渲染

use crate::config::AppConfig;
use crate::platform::macos::desktop_overlay::box_render::{get_boxes_from_config, render_box};
use crate::platform::macos::desktop_overlay::overlay_window::ScreenInfo;
use cocoa::base::id;
use cocoa::foundation::{NSPoint, NSRect, NSSize};
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use foreign_types_shared::ForeignType;
use objc::{class, msg_send, sel, sel_impl};

pub(crate) fn render_overlay_placeholder(
    window: id,
    screen_info: &ScreenInfo,
    screen_index: usize,
) {
    log::info!("渲染桌面分类框和图标 (屏幕 {})", screen_index);

    let config = AppConfig::load().unwrap_or_default();
    let style = &config.box_style;
    let boxes = get_boxes_from_config(&config, screen_index);

    if boxes.is_empty() {
        log::info!("屏幕 {} 没有分类框，跳过渲染", screen_index);
        render_empty(window, screen_info);
        return;
    }

    log::info!("开始渲染 {} 个分类框 (屏幕 {})", boxes.len(), screen_index);

    unsafe {
        let screen_width = screen_info.frame.size.width;
        let screen_height = screen_info.frame.size.height;
        let scale = screen_info.scale_factor;

        let bitmap_width = (screen_width * scale) as usize;
        let bitmap_height = (screen_height * scale) as usize;

        // 创建位图上下文（Y=0 在底部，标准 CG 坐标系）
        let color_space = CGColorSpace::create_device_rgb();
        let context = CGContext::create_bitmap_context(
            None,
            bitmap_width,
            bitmap_height,
            8,
            bitmap_width * 4,
            &color_space,
            0x2000 | 0x02, // kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big
        );

        // 清空为全透明
        context.set_rgb_fill_color(0.0, 0.0, 0.0, 0.0);
        context.fill_rect(CGRect::new(
            &CGPoint::new(0.0, 0.0),
            &CGSize::new(bitmap_width as f64, bitmap_height as f64),
        ));

        // 渲染每个分类框
        for item in &boxes {
            render_box(&context, item, style, scale, bitmap_height as f64);
        }

        // 创建 CGImage
        let cg_image = match context.create_image() {
            Some(img) => img,
            None => {
                log::error!("创建 CGImage 失败");
                return;
            }
        };

        // 创建 NSImage 和 NSImageView
        let ns_image: id = msg_send![class!(NSImage), alloc];
        let ns_image: id = msg_send![ns_image, initWithCGImage: cg_image.as_ptr()
                                                       size: NSSize::new(screen_width, screen_height)];

        let image_view: id = msg_send![class!(NSImageView), alloc];
        let image_view: id = msg_send![image_view, initWithFrame: NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(screen_width, screen_height)
        )];

        let _: () = msg_send![image_view, setImage: ns_image];
        let _: () = msg_send![window, setContentView: image_view];

        log::info!("分类框渲染完成 (屏幕 {})", screen_index);
    }
}

fn render_empty(window: id, screen_info: &ScreenInfo) {
    unsafe {
        let screen_width = screen_info.frame.size.width;
        let screen_height = screen_info.frame.size.height;

        let view: id = msg_send![class!(NSView), alloc];
        let view: id = msg_send![view, initWithFrame: NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(screen_width, screen_height)
        )];

        let _: () = msg_send![window, setContentView: view];
    }
}
