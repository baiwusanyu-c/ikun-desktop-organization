//! 分类框渲染模块 - 对标 Windows 版本

use crate::config::app_config::{AppConfig, BoxStyleConfig};
use crate::platform::macos::desktop_manager::DesktopManager;
use crate::platform::macos::icon_extract::extract_icon;
use crate::platform::DesktopManagerTrait;
use cocoa::base::{id, nil};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use foreign_types_shared::ForeignType;
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::c_void;

extern "C" {
    fn CGBitmapContextGetData(context: core_graphics::sys::CGContextRef) -> *mut c_void;
}

#[derive(Debug, Clone)]
pub struct ClassificationBox {
    pub id: String,
    pub title: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub icon_paths: Vec<String>,
    pub locked: bool,
}

impl ClassificationBox {
    pub fn new(
        id: String,
        title: String,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        locked: bool,
    ) -> Self {
        Self {
            id,
            title,
            x,
            y,
            width,
            height,
            icon_paths: Vec::new(),
            locked,
        }
    }
}

pub fn render_box(
    context: &CGContext,
    cls_box: &ClassificationBox,
    style: &BoxStyleConfig,
    scale: f64,
    bitmap_height: f64,
) {
    log::debug!(
        "渲染分类框: {} at ({}, {})",
        cls_box.title,
        cls_box.x,
        cls_box.y
    );

    // 屏幕坐标（Y=0 在顶部）转换为 CG 坐标（Y=0 在底部）
    let screen_x = cls_box.x * scale;
    let screen_y = cls_box.y * scale;
    let box_w = cls_box.width * scale;
    let box_h = cls_box.height * scale;
    let box_y = bitmap_height - screen_y - box_h;

    // 1. 圆角背景
    let bg_color = style.background_color;
    let bg_alpha = style.background_alpha as f64 / 255.0;
    context.set_rgb_fill_color(
        bg_color[0] as f64 / 255.0,
        bg_color[1] as f64 / 255.0,
        bg_color[2] as f64 / 255.0,
        bg_alpha,
    );

    let radius = style.border_radius as f64 * scale;
    draw_rounded_rect(context, screen_x, box_y, box_w, box_h, radius);

    // 2. 标题栏分隔线
    let title_h = style.title_bar_height as f64 * scale;
    let border_color = style.border_color;
    context.set_rgb_fill_color(
        border_color[0] as f64 / 255.0,
        border_color[1] as f64 / 255.0,
        border_color[2] as f64 / 255.0,
        border_color[3] as f64 / 255.0,
    );

    let sep_y = box_y + box_h - title_h;
    let sep_rect = CGRect::new(&CGPoint::new(screen_x, sep_y), &CGSize::new(box_w, scale));
    context.fill_rect(sep_rect);

    // 3. 标题文字（屏幕坐标）
    let title_screen_y = screen_y + 2.0 * scale;
    render_title(
        context,
        &cls_box.title,
        screen_x,
        title_screen_y,
        box_w,
        title_h,
        style,
        scale,
        bitmap_height,
    );

    // 4. 图标网格（屏幕坐标）
    let grid_top_screen_y = screen_y + title_h;
    render_icon_grid(
        context,
        &cls_box.icon_paths,
        screen_x,
        grid_top_screen_y,
        box_w,
        style,
        scale,
        bitmap_height,
    );

    log::debug!("分类框渲染完成: {}", cls_box.title);
}

fn draw_rounded_rect(context: &CGContext, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let k = 0.5522847498;
    let kr = k * r;

    context.begin_path();
    context.move_to_point(x + r, y);
    context.add_line_to_point(x + w - r, y);
    context.add_curve_to_point(x + w - r + kr, y, x + w, y + r - kr, x + w, y + r);
    context.add_line_to_point(x + w, y + h - r);
    context.add_curve_to_point(
        x + w,
        y + h - r + kr,
        x + w - r + kr,
        y + h,
        x + w - r,
        y + h,
    );
    context.add_line_to_point(x + r, y + h);
    context.add_curve_to_point(x + r - kr, y + h, x, y + h - r + kr, x, y + h - r);
    context.add_line_to_point(x, y + r);
    context.add_curve_to_point(x, y + r - kr, x + r - kr, y, x + r, y);
    context.close_path();
    context.fill_path();
}

/// 渲染标题（使用 NSAttributedString + NSGraphicsContext）
fn render_title(
    context: &CGContext,
    title: &str,
    screen_x: f64,
    screen_y: f64,
    box_w: f64,
    title_h: f64,
    style: &BoxStyleConfig,
    scale: f64,
    bitmap_height: f64,
) {
    let font_size = style.title_font_size as f64 * scale;
    let title_color = style.title_font_color;
    let text_x = screen_x + 10.0 * scale;
    let text_y = screen_y + (title_h - font_size) / 2.0;

    unsafe {
        // 设置 NSGraphicsContext（flipped: false 表示 Y=0 在底部）
        let cg_ptr = context.as_ptr() as *mut c_void;
        let graphics_context: id = msg_send![
            class!(NSGraphicsContext),
            graphicsContextWithCGContext: cg_ptr flipped: false
        ];
        let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: graphics_context];

        let title_ns = NSString::alloc(nil).init_str(title);
        let attrs: id = msg_send![class!(NSMutableDictionary), dictionary];

        let font: id = msg_send![class!(NSFont), boldSystemFontOfSize: font_size];
        let color: id = msg_send![
            class!(NSColor),
            colorWithRed: title_color[0] as f64 / 255.0
            green: title_color[1] as f64 / 255.0
            blue: title_color[2] as f64 / 255.0
            alpha: title_color[3] as f64 / 255.0
        ];

        let ns_font_key = NSString::alloc(nil).init_str("NSFont");
        let ns_color_key = NSString::alloc(nil).init_str("NSColor");

        let _: () = msg_send![attrs, setObject: font forKey: ns_font_key];
        let _: () = msg_send![attrs, setObject: color forKey: ns_color_key];

        // 添加阴影
        let shadow: id = msg_send![class!(NSShadow), shadow];
        let shadow_color: id = msg_send![
            class!(NSColor),
            colorWithRed: 0.0 green: 0.0 blue: 0.0 alpha: 0.8
        ];
        let _: () = msg_send![shadow, setShadowColor: shadow_color];
        let _: () = msg_send![shadow, setShadowOffset: NSSize::new(scale, -scale)];
        let _: () = msg_send![shadow, setShadowBlurRadius: 2.0 * scale];
        let ns_shadow_key = NSString::alloc(nil).init_str("NSShadow");
        let _: () = msg_send![attrs, setObject: shadow forKey: ns_shadow_key];

        // 屏幕坐标转换为 CG 坐标（Y=0 在底部）
        let cg_y = bitmap_height - text_y - font_size;
        let title_rect = NSRect::new(
            NSPoint::new(text_x, cg_y),
            NSSize::new(box_w - 20.0 * scale, font_size + 10.0),
        );

        let _: () = msg_send![title_ns, drawInRect: title_rect withAttributes: attrs];

        let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: nil];
    }
}

fn render_icon_grid(
    context: &CGContext,
    icon_paths: &[String],
    screen_x: f64,
    grid_top_screen_y: f64,
    box_w: f64,
    style: &BoxStyleConfig,
    scale: f64,
    bitmap_height: f64,
) {
    let icon_size = style.icon_size as f64 * scale;
    let cell_spacing = style.cell_spacing as f64 * scale;
    let label_h = 36.0 * scale;

    let cols = if style.columns > 0 {
        style.columns as usize
    } else {
        let available_w = box_w - 20.0 * scale;
        let cell_w = icon_size + cell_spacing;
        ((available_w / cell_w).floor() as usize).max(1)
    };

    let cell_w = icon_size + cell_spacing;
    let cell_h = icon_size + label_h + cell_spacing;
    let grid_left = screen_x + 10.0 * scale;
    let grid_top = grid_top_screen_y + 10.0 * scale;

    for (i, path) in icon_paths.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;

        let cell_screen_x = grid_left + (col as f64) * cell_w;
        let cell_screen_y = grid_top + (row as f64) * cell_h;

        let icon_screen_x = cell_screen_x + (cell_w - icon_size) / 2.0;
        let icon_screen_y = cell_screen_y + 4.0 * scale;

        render_icon(
            context,
            path,
            icon_screen_x,
            icon_screen_y,
            icon_size,
            bitmap_height,
        );

        // 标签宽度 = 1.4 倍图标宽度（对齐 Finder：label 略微超出 icon 边界，居中对齐）
        let label_width = icon_size * 1.4;
        let label_screen_x = icon_screen_x + icon_size / 2.0 - label_width / 2.0;
        let label_screen_y = icon_screen_y + icon_size + 6.0 * scale;
        render_label(
            context,
            path,
            label_screen_x,
            label_screen_y,
            label_width,
            style,
            scale,
            bitmap_height,
        );
    }
}

/// 渲染图标（使用 NSImage 内置绘制）
fn render_icon(
    context: &CGContext,
    path: &str,
    screen_x: f64,
    screen_y: f64,
    size: f64,
    bitmap_height: f64,
) {
    log::info!("开始渲染图标: {}", path);
    let ns_icon = match extract_icon(std::path::Path::new(path)) {
        Some(icon) => icon,
        None => {
            log::error!("无法提取图标: {}", path);
            return;
        }
    };
    log::info!("成功提取图标: {}", path);

    unsafe {
        // 设置 NSGraphicsContext
        let cg_ptr = context.as_ptr() as *mut c_void;
        let graphics_context: id = msg_send![
            class!(NSGraphicsContext),
            graphicsContextWithCGContext: cg_ptr flipped: false
        ];
        let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: graphics_context];

        // 屏幕坐标转换为 CG 坐标
        let cg_y = bitmap_height - screen_y - size;
        let dest_rect = NSRect::new(NSPoint::new(screen_x, cg_y), NSSize::new(size, size));

        // 获取图标的完整尺寸
        let icon_size: NSSize = msg_send![ns_icon, size];
        let from_rect = NSRect::new(NSPoint::new(0.0, 0.0), icon_size);

        // 使用 NSCompositeSourceOver 绘制图标（保持 alpha 通道）
        let _: () = msg_send![
            ns_icon,
            drawInRect: dest_rect
            fromRect: from_rect
            operation: 2u64  // NSCompositeSourceOver
            fraction: 1.0
        ];

        let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: nil];
    }
}

/// 标签文本分割算法（Finder 风格）
///
/// 策略：
/// - 第一行：最大前缀（无省略号），尽量填满 width
/// - 第二行：剩余内容，超出则中间截断（保留 extension）
///
/// # Arguments
/// * `text` - 原始文本
/// * `width` - 可用宽度（像素）
/// * `measure_fn` - 测量函数：给定文本返回宽度（像素）
///
/// # Returns
/// 返回 1-2 行的 Vec<String>
pub fn split_label_lines<F>(text: &str, width: f64, measure_fn: F) -> Vec<String>
where
    F: Fn(&str) -> f64,
{
    let chars: Vec<char> = text.chars().collect();
    let total_chars = chars.len();

    // 测量整段文字宽度
    let full_width = measure_fn(text);

    if full_width <= width {
        // 单行即可放下
        return vec![text.to_string()];
    }

    // 找第一行的最大前缀（不超过 width）
    let mut line1_end = 0;
    for i in 1..=total_chars {
        let test_str: String = chars[..i].iter().collect();
        let test_width = measure_fn(&test_str);

        if test_width > width {
            line1_end = i - 1;
            break;
        }
    }
    if line1_end == 0 {
        line1_end = total_chars / 2;
    }

    let line1: String = chars[..line1_end].iter().collect();

    // 第二行：从 line1_end 到结尾
    let remaining: String = chars[line1_end..].iter().collect();
    let remaining_width = measure_fn(&remaining);

    let line2 = if remaining_width <= width {
        // 第二行能放下，不需要截断
        remaining
    } else {
        // 第二行超出，使用中间截断（保留 extension）
        let rem_chars: Vec<char> = remaining.chars().collect();
        let rem_count = rem_chars.len();

        // 找最大的 prefix+suffix 组合，使 prefix+"…"+suffix <= width
        let mut best_keep = 3;
        for keep in 3..rem_count {
            let half = keep / 2;
            let prefix: String = rem_chars[..half].iter().collect();
            let suffix: String = rem_chars[rem_count - half..].iter().collect();
            let test_str = format!("{}…{}", prefix, suffix);
            let test_width = measure_fn(&test_str);

            if test_width > width {
                break;
            }
            best_keep = keep;
        }

        let half = best_keep / 2;
        let prefix: String = rem_chars[..half].iter().collect();
        let suffix: String = rem_chars[rem_count - half..].iter().collect();
        format!("{}…{}", prefix, suffix)
    };

    vec![line1, line2]
}

/// 渲染标签（对齐 Finder 桌面图标标签）
///
/// 使用 NSLineBreakByCharWrapping 让文字自然换行，居中对齐。
/// 最多 2 行（超出自动裁切）。
fn render_label(
    context: &CGContext,
    path: &str,
    screen_x: f64,
    screen_y: f64,
    width: f64,
    style: &BoxStyleConfig,
    scale: f64,
    bitmap_height: f64,
) {
    let display_name = unsafe {
        let path_ns = NSString::alloc(nil).init_str(path);
        let file_manager: id = msg_send![class!(NSFileManager), defaultManager];
        let display_name_ns: id = msg_send![file_manager, displayNameAtPath: path_ns];

        let utf8: *const std::os::raw::c_char = msg_send![display_name_ns, UTF8String];
        if utf8.is_null() {
            String::from("Unknown")
        } else {
            std::ffi::CStr::from_ptr(utf8)
                .to_string_lossy()
                .into_owned()
        }
    };

    let font_size = style.label_font_size as f64 * scale;
    let label_color = style.label_font_color;
    let line_height = font_size * 1.3;

    log::info!(
        "render_label: '{}' x={}, y={}, w={}, fs={}",
        display_name,
        screen_x,
        screen_y,
        width,
        font_size
    );

    unsafe {
        // 1. NSGraphicsContext
        let cg_ptr = context.as_ptr() as *mut c_void;
        let graphics_context: id = msg_send![
            class!(NSGraphicsContext),
            graphicsContextWithCGContext: cg_ptr flipped: false
        ];
        let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: graphics_context];

        // 2. 字体和颜色
        let font: id = msg_send![class!(NSFont), systemFontOfSize: font_size];
        let color: id = msg_send![
            class!(NSColor),
            colorWithRed: label_color[0] as f64 / 255.0
            green: label_color[1] as f64 / 255.0
            blue: label_color[2] as f64 / 255.0
            alpha: label_color[3] as f64 / 255.0
        ];

        // 3. 属性字典
        let attrs: id = msg_send![class!(NSMutableDictionary), dictionary];
        let _: () =
            msg_send![attrs, setObject: font forKey: NSString::alloc(nil).init_str("NSFont")];
        let _: () =
            msg_send![attrs, setObject: color forKey: NSString::alloc(nil).init_str("NSColor")];

        // 4. 分割文字（Finder 风格，使用纯函数）
        let lines = split_label_lines(&display_name, width, |text| {
            let ns = NSString::alloc(nil).init_str(text);
            let size: cocoa::foundation::NSSize = msg_send![ns, sizeWithAttributes: attrs];
            size.width
        });

        // 5. 逐行绘制（居中对齐）
        log::info!("render_label: split into {} lines", lines.len());
        for (line_idx, line_text) in lines.iter().enumerate() {
            let line_str = NSString::alloc(nil).init_str(line_text);
            let size: cocoa::foundation::NSSize = msg_send![line_str, sizeWithAttributes: attrs];

            let line_x = screen_x + (width - size.width) / 2.0;
            let line_screen_y = screen_y + (line_idx as f64) * line_height;
            let line_cg_y = bitmap_height - line_screen_y - line_height;

            log::info!(
                "  line {}: '{}' w={:.1}, x={:.1}",
                line_idx,
                line_text,
                size.width,
                line_x
            );

            let point = NSPoint::new(line_x, line_cg_y);
            let _: () = msg_send![line_str, drawAtPoint: point withAttributes: attrs];
        }

        let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: nil];
    }
}

pub fn get_boxes_from_config(config: &AppConfig, screen_index: usize) -> Vec<ClassificationBox> {
    if screen_index != 0 {
        log::debug!("跳过副屏分类框渲染");
        return Vec::new();
    }

    let box_layouts = &config.layout.boxes;

    if box_layouts.is_empty() {
        log::info!("配置文件中没有分类框，使用默认测试数据");
        return get_default_test_boxes();
    }

    let mut result = Vec::new();

    for box_data in box_layouts {
        let mut cls_box = ClassificationBox::new(
            box_data.id.to_string(),
            box_data.name.clone(),
            box_data.position[0] as f64,
            box_data.position[1] as f64,
            box_data.size[0] as f64,
            box_data.size[1] as f64,
            box_data.locked,
        );

        let desktop_manager = DesktopManager::new().unwrap_or_default();
        let icons = desktop_manager.get_desktop_icons();

        for icon in icons.iter() {
            if let Some(filename) = icon.path.file_name().and_then(|n| n.to_str()) {
                if box_data
                    .icon_names
                    .iter()
                    .any(|name| filename.contains(name))
                {
                    cls_box
                        .icon_paths
                        .push(icon.path.to_string_lossy().to_string());
                }
            }
        }

        result.push(cls_box);
    }

    log::info!("从配置文件加载了 {} 个分类框", result.len());
    result
}

fn get_default_test_boxes() -> Vec<ClassificationBox> {
    let desktop_manager = DesktopManager::new().unwrap_or_default();
    let icons = desktop_manager.get_desktop_icons();

    let mut box1 = ClassificationBox::new(
        "test-1".to_string(),
        "文档".to_string(),
        100.0,
        100.0,
        680.0,
        800.0,
        false,
    );

    let mut box2 = ClassificationBox::new(
        "test-2".to_string(),
        "工具".to_string(),
        800.0,
        100.0,
        680.0,
        800.0,
        false,
    );

    for (i, icon) in icons.iter().enumerate() {
        let path_str = icon.path.to_string_lossy().to_string();
        if i % 2 == 0 {
            box1.icon_paths.push(path_str);
        } else {
            box2.icon_paths.push(path_str);
        }
    }

    vec![box1, box2]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模拟等宽字体测量：每个字符 10px
    fn mock_measure_monospace(text: &str) -> f64 {
        text.chars().count() as f64 * 10.0
    }

    /// 模拟测量：ASCII 字符 8px，非 ASCII（中文等）16px，省略号 12px
    fn mock_measure_mixed(text: &str) -> f64 {
        text.chars()
            .map(|c| {
                if c == '…' {
                    12.0
                } else if c.is_ascii() {
                    8.0
                } else {
                    16.0
                }
            })
            .sum()
    }

    // =========================================================================
    // split_label_lines 单元测试
    // =========================================================================

    #[test]
    fn test_short_text_single_line() {
        let result = split_label_lines("hello", 100.0, mock_measure_monospace);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hello");
    }

    #[test]
    fn test_exact_width_single_line() {
        // "hello" = 5 chars * 10px = 50px, width = 50
        let result = split_label_lines("hello", 50.0, mock_measure_monospace);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hello");
    }

    #[test]
    fn test_two_lines_no_truncation() {
        // "abcdefghij" = 10 chars * 10px = 100px, width = 60
        // line1: first 6 chars = "abcdef" (60px)
        // line2: remaining "ghij" (40px) <= 60, no truncation
        let result = split_label_lines("abcdefghij", 60.0, mock_measure_monospace);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "abcdef");
        assert_eq!(result[1], "ghij");
    }

    #[test]
    fn test_two_lines_with_middle_truncation() {
        // "abcdefghijklmnopqrstuvwxyz" = 26 chars * 10px = 260px, width = 80
        // line1: first 8 chars = "abcdefgh" (80px)
        // line2: remaining "ijklmnopqrstuvwxyz" (18 chars * 10px = 180px) > 80
        //   → middle truncation: keep best_keep chars, prefix…suffix
        let result = split_label_lines("abcdefghijklmnopqrstuvwxyz", 80.0, mock_measure_monospace);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "abcdefgh");
        // line2 should contain "…" (middle truncation)
        assert!(
            result[1].contains('…'),
            "line2 should have middle truncation: {}",
            result[1]
        );
        // line2 should start with some prefix of "ijklmnopqrstuvwxyz"
        assert!(
            result[1].starts_with('i'),
            "line2 should start with 'i': {}",
            result[1]
        );
        // line2 should end with some suffix
        assert!(
            result[1].ends_with("yz"),
            "line2 should end with 'yz': {}",
            result[1]
        );
    }

    #[test]
    fn test_chinese_text_single_line() {
        // "股市" = 2 chars * 16px = 32px, width = 100
        let result = split_label_lines("股市", 100.0, mock_measure_mixed);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "股市");
    }

    #[test]
    fn test_chinese_text_two_lines() {
        // "PandaClaw 架构分析" mixed width
        // P(8)+a(8)+n(8)+d(8)+a(8)+C(8)+l(8)+a(8)+w(8)+space(8)+架(16)+构(16)+分(16)+析(16) = 80+64 = 144px
        // width = 100
        let result = split_label_lines("PandaClaw 架构分析", 100.0, mock_measure_mixed);
        assert_eq!(result.len(), 2);
        // line1 should be a prefix that fits in 100px
        let line1_width = mock_measure_mixed(&result[0]);
        assert!(
            line1_width <= 100.0,
            "line1 width {} should be <= 100",
            line1_width
        );
    }

    #[test]
    fn test_long_filename_finder_style() {
        // Simulate: alert-message-notification-统计报告与实现计划.md
        let text = "alert-message-notification-统计报告与实现计划.md";
        let width = 179.0; // icon_size * 1.4

        let result = split_label_lines(text, width, mock_measure_mixed);
        assert_eq!(result.len(), 2, "should split into 2 lines");

        // line1: should be a prefix without "…"
        assert!(
            !result[0].contains('…'),
            "line1 should NOT have ellipsis: {}",
            result[0]
        );
        let line1_width = mock_measure_mixed(&result[0]);
        assert!(
            line1_width <= width,
            "line1 width {} should be <= {}",
            line1_width,
            width
        );

        // line2: should contain "…" if it needs truncation
        let remaining: String = text.chars().skip(result[0].chars().count()).collect();
        let remaining_width = mock_measure_mixed(&remaining);
        if remaining_width > width {
            assert!(
                result[1].contains('…'),
                "line2 should have middle truncation: {}",
                result[1]
            );
            // Should preserve extension (.md)
            assert!(
                result[1].ends_with(".md"),
                "line2 should end with .md: {}",
                result[1]
            );
        }
    }

    #[test]
    fn test_first_line_no_ellipsis() {
        // Verify first line never contains ellipsis (Finder style)
        let texts = vec![
            "xmind2md.py",
            "Windows 11",
            "PENDING_FEATURES.md",
            "alert-message-notification-统计报告.md",
        ];

        for text in texts {
            let result = split_label_lines(text, 128.0, mock_measure_mixed);
            assert!(
                !result[0].contains('…'),
                "First line should never have ellipsis: '{}' for input '{}'",
                result[0],
                text
            );
        }
    }

    #[test]
    fn test_extension_preserved_in_truncation() {
        // Long filename with .md extension
        let text = "very-long-filename-with-many-dashes-and-chinese-字符-统计报告与实现计划.md";
        let result = split_label_lines(text, 100.0, mock_measure_mixed);

        if result.len() == 2 {
            let line2_width = mock_measure_mixed(&result[1]);
            if line2_width > 100.0 || result[1].contains('…') {
                // If truncated, should preserve extension
                assert!(
                    result[1].ends_with(".md"),
                    "Truncated line2 should preserve .md extension: '{}'",
                    result[1]
                );
            }
        }
    }

    #[test]
    fn test_empty_string() {
        let result = split_label_lines("", 100.0, mock_measure_monospace);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "");
    }

    #[test]
    fn test_single_char() {
        let result = split_label_lines("a", 100.0, mock_measure_monospace);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "a");
    }

    #[test]
    fn test_very_narrow_width() {
        // Width smaller than a single character
        let result = split_label_lines("hello", 5.0, mock_measure_monospace);
        assert_eq!(result.len(), 2);
        // Should still produce 2 lines without panicking
    }

    #[test]
    fn test_line2_width_within_bounds() {
        // Verify that line2 width never exceeds the given width
        let texts = vec![
            "abcdefghijklmnopqrstuvwxyz",
            "alert-message-notification-统计报告与实现计划.md",
            "PENDING_FEATURES.md",
            "very-long-filename-that-needs-truncation.txt",
        ];

        for text in texts {
            let result = split_label_lines(text, 128.0, mock_measure_mixed);
            for (i, line) in result.iter().enumerate() {
                let w = mock_measure_mixed(line);
                // Allow small tolerance for the "…" character
                assert!(
                    w <= 128.0 + 1.0,
                    "Line {} width {} exceeds 128 for '{}': '{}'",
                    i,
                    w,
                    text,
                    line
                );
            }
        }
    }
}
