//! 覆盖窗口渲染

use crate::config::AppConfig;
use crate::platform::macos::desktop_overlay::box_render::{get_boxes_from_config, render_box};
use crate::platform::macos::desktop_overlay::overlay_window::ScreenInfo;
use cocoa::base::id;
use cocoa::foundation::NSSize;
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use foreign_types_shared::ForeignType;
use objc::{class, msg_send, sel, sel_impl};

/// 渲染覆盖窗口内容。
///
/// 返回 `(pixel_data, ns_image)`：
/// - `pixel_data`: RGBA 像素数据（用于命中测试），Y=0 在底部
/// - `ns_image`: NSImage 对象（用于显示），调用者负责设置到内容视图
///
/// 如果渲染失败或没有分类框，返回 `(Vec::new(), nil)`。
pub(crate) fn render_overlay_placeholder(
    _window: id,
    screen_info: &ScreenInfo,
    screen_index: usize,
) -> (Vec<u8>, id) {
    log::info!("渲染桌面分类框和图标 (屏幕 {})", screen_index);

    let config = AppConfig::load().unwrap_or_default();
    let style = &config.box_style;
    let boxes = get_boxes_from_config(&config, screen_index);

    if boxes.is_empty() {
        log::info!("屏幕 {} 没有分类框，跳过渲染", screen_index);
        return (Vec::new(), cocoa::base::nil);
    }

    log::info!(
        "开始渲染 {} 个分类框 (屏幕 {})",
        boxes.len(),
        screen_index
    );

    let screen_width = screen_info.frame.size.width;
    let screen_height = screen_info.frame.size.height;
    let scale = screen_info.scale_factor;

    let bitmap_width = (screen_width * scale) as usize;
    let bitmap_height = (screen_height * scale) as usize;

    // 创建位图上下文（Y=0 在底部，标准 CG 坐标系）
    let color_space = CGColorSpace::create_device_rgb();
    let mut context = CGContext::create_bitmap_context(
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

    // 提取像素数据（用于命中测试）
    let data_ptr = context.data().as_ptr();
    let pixel_data_len = bitmap_width * bitmap_height * 4;
    let pixel_data = unsafe { std::slice::from_raw_parts(data_ptr, pixel_data_len).to_vec() };

    // 创建 CGImage → NSImage
    let cg_image = match context.create_image() {
        Some(img) => img,
        None => {
            log::error!("创建 CGImage 失败");
            return (Vec::new(), cocoa::base::nil);
        }
    };

    let ns_image = unsafe {
        let img: id = msg_send![class!(NSImage), alloc];
        let img: id = msg_send![img, initWithCGImage: cg_image.as_ptr()
                                              size: NSSize::new(screen_width, screen_height)];
        img
    };

    log::info!(
        "分类框渲染完成 (屏幕 {}), 像素数据 {} bytes",
        screen_index,
        pixel_data.len()
    );
    (pixel_data, ns_image)
}
