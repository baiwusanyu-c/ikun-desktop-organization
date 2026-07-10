//! macOS 覆盖窗口管理
//!
//! 创建透明覆盖窗口，覆盖桌面工作区域。
//! - 窗口层级：kCGDesktopIconWindowLevel (20)
//! - 鼠标穿透：通过 NSEvent monitor 检测像素透明度，动态切换 ignoresMouseEvents
//! - 样式：NSNonactivatingPanel (128) + NSBorderlessWindowMask (0)

use crate::platform::macos::ffi_imports::*;
use anyhow::Result;
use cocoa::appkit::NSBackingStoreBuffered;
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::NSRect;
use objc::{msg_send, sel, sel_impl};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::overlay_render::render_overlay_placeholder;

pub(crate) struct OverlayWindow {
    pub(crate) window: id,
    pub(crate) screen_index: usize,
}

unsafe impl Send for OverlayWindow {}

static OVERLAY_WINDOWS: Mutex<Vec<OverlayWindow>> = Mutex::new(Vec::new());

/// 命中测试位图数据（以窗口指针为键）
/// 使用 OnceLock 延迟初始化（HashMap::new() 非 const）
static HIT_TEST_MAP: OnceLock<Mutex<HashMap<u64, (Vec<u8>, usize, usize, f64, NSRect)>>> =
    OnceLock::new();

fn hit_test_map() -> &'static Mutex<HashMap<u64, (Vec<u8>, usize, usize, f64, NSRect)>> {
    HIT_TEST_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 鼠标事件监听器引用（防止被释放）
/// 使用 SendMonitor 包装 id 以支持 Send trait
struct SendMonitor(id);
unsafe impl Send for SendMonitor {}

static MOUSE_MONITOR: Mutex<Option<SendMonitor>> = Mutex::new(None);

pub(crate) struct ScreenInfo {
    pub(crate) frame: NSRect,
    pub(crate) scale_factor: f64,
}

pub(crate) fn get_all_screens() -> Vec<ScreenInfo> {
    unsafe {
        let screens: id = msg_send![class!(NSScreen), screens];
        let count: usize = msg_send![screens, count];
        let mut screen_infos = Vec::new();
        for i in 0..count {
            let screen: id = msg_send![screens, objectAtIndex: i];
            let frame: NSRect = msg_send![screen, frame];
            let scale_factor: f64 = msg_send![screen, backingScaleFactor];
            screen_infos.push(ScreenInfo {
                frame,
                scale_factor,
            });
        }
        screen_infos
    }
}

pub fn create_desktop_overlay() -> Result<()> {
    log::info!("创建 macOS 桌面覆盖窗口...");
    let screens = get_all_screens();
    if screens.is_empty() {
        anyhow::bail!("未检测到屏幕");
    }
    log::info!("检测到 {} 个屏幕", screens.len());

    // 先销毁旧窗口
    let mut overlays = OVERLAY_WINDOWS.lock().unwrap();
    log::info!("销毁旧的覆盖窗口（{} 个）", overlays.len());
    for overlay in overlays.drain(..) {
        unsafe {
            let _: () = msg_send![overlay.window, orderOut: nil];
            let _: () = msg_send![overlay.window, close];
        }
    }

    // 清理命中测试数据
    hit_test_map().lock().unwrap().clear();

    // 创建新窗口
    for (index, screen) in screens.iter().enumerate() {
        match create_overlay_for_screen(screen, index) {
            Ok(window) => {
                log::info!("屏幕 {} 覆盖窗口创建成功", index);
                overlays.push(OverlayWindow {
                    window,
                    screen_index: index,
                });
            }
            Err(e) => {
                log::error!("屏幕 {} 覆盖窗口创建失败: {}", index, e);
            }
        }
    }

    // 设置鼠标事件监听器（用于动态切换 ignoresMouseEvents）
    if !overlays.is_empty() {
        setup_mouse_monitor();
    }

    Ok(())
}

/// 为指定屏幕创建覆盖窗口
fn create_overlay_for_screen(screen: &ScreenInfo, index: usize) -> Result<id> {
    unsafe {
        let window: id = msg_send![class!(NSPanel), alloc];

        // NSNonactivatingPanel (128): 面板不会成为 key window，不会抢焦点
        // NSBorderlessWindowMask (0): 无边框
        let style_mask: u64 = 128; // NSNonactivatingPanel

        let window: id = msg_send![
            window,
            initWithContentRect: screen.frame
            styleMask: style_mask
            backing: NSBackingStoreBuffered
            defer: NO
        ];
        if window == nil {
            anyhow::bail!("创建 NSWindow 失败");
        }

        // 窗口属性
        let _: () = msg_send![window, setOpaque: NO];
        let clear_color: id = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![window, setBackgroundColor: clear_color];
        let _: () = msg_send![window, setHasShadow: NO];

        // 窗口层级：kCGDesktopIconWindowLevel = 20
        // 与 Finder 桌面图标同层，覆盖在桌面壁纸之上
        let level = NS_WINDOW_LEVEL_DESKTOP_ICON;
        let _: () = msg_send![window, setLevel: level];

        // 初始状态：忽略鼠标事件（全透明时）
        // 鼠标监听器会根据像素透明度动态切换此属性
        let _: () = msg_send![window, setIgnoresMouseEvents: YES];

        let actual_level: i64 = msg_send![window, level];
        log::info!(
            "屏幕 {} 覆盖窗口层级: 设置={}, 实际={}",
            index,
            level,
            actual_level
        );
        log::info!(
            "屏幕 {} 覆盖窗口 frame: ({}, {}, {}x{})",
            index,
            screen.frame.origin.x,
            screen.frame.origin.y,
            screen.frame.size.width,
            screen.frame.size.height
        );

        // 渲染覆盖内容（分类框 + 图标）
        let (pixel_data, ns_image) = render_overlay_placeholder(window, screen, index);

        if ns_image != nil {
            // 创建 NSImageView 作为内容视图
            let image_view: id = msg_send![class!(NSImageView), alloc];
            let image_view: id = msg_send![image_view, initWithFrame: NSRect::new(
                cocoa::foundation::NSPoint::new(0.0, 0.0),
                cocoa::foundation::NSSize::new(screen.frame.size.width, screen.frame.size.height),
            )];
            let _: () = msg_send![image_view, setImage: ns_image];
            let _: () = msg_send![window, setContentView: image_view];

            // 存储像素数据到命中测试映射
            let window_key = window as u64;
            let bitmap_width = (screen.frame.size.width * screen.scale_factor) as usize;
            let bitmap_height = (screen.frame.size.height * screen.scale_factor) as usize;
            hit_test_map().lock().unwrap().insert(
                window_key,
                (
                    pixel_data,
                    bitmap_width,
                    bitmap_height,
                    screen.scale_factor,
                    screen.frame,
                ),
            );
        }

        let _: () = msg_send![window, orderFrontRegardless];
        Ok(window)
    }
}

/// 设置鼠标事件监听器
///
/// 使用 NSEvent.addLocalMonitorForEventsMatchingMask:handler: 监听鼠标移动事件。
/// 当鼠标位于覆盖窗口之上时，检查鼠标下方像素的透明度：
/// - 透明像素 → 设置 ignoresMouseEvents = YES（鼠标事件穿透到桌面）
/// - 不透明像素 → 设置 ignoresMouseEvents = NO（拦截鼠标事件）
fn setup_mouse_monitor() {
    use block::ConcreteBlock;

    log::info!("设置鼠标事件监听器（动态命中测试）");

    // NSEventMaskMouseMoved = 1 << 5 = 32
    let event_mask: u64 = 1 << 5;

    // 创建事件处理块
    let handler_block = ConcreteBlock::new(|event: id| -> id {
        unsafe {
            let mouse_loc: cocoa::foundation::NSPoint = msg_send![class!(NSEvent), mouseLocation];

            let overlays = OVERLAY_WINDOWS.lock().unwrap();
            for overlay in overlays.iter() {
                let window = overlay.window;
                let frame: NSRect = msg_send![window, frame];

                // 检查鼠标是否在窗口范围内
                if mouse_loc.x >= frame.origin.x
                    && mouse_loc.x < frame.origin.x + frame.size.width
                    && mouse_loc.y >= frame.origin.y
                    && mouse_loc.y < frame.origin.y + frame.size.height
                {
                    let is_opaque = is_pixel_opaque(window, mouse_loc, frame);
                    let current_ignores: cocoa::base::BOOL = msg_send![window, ignoresMouseEvents];

                    if is_opaque && current_ignores != NO {
                        let _: () = msg_send![window, setIgnoresMouseEvents: NO];
                    } else if !is_opaque && current_ignores != YES {
                        let _: () = msg_send![window, setIgnoresMouseEvents: YES];
                    }
                }
            }
        }
        event // 放行事件，不做拦截
    });

    let handler = handler_block.copy();

    unsafe {
        let monitor: id = msg_send![
            class!(NSEvent),
            addLocalMonitorForEventsMatchingMask: event_mask
            handler: &*handler
        ];

        // 保存引用防止被释放
        *MOUSE_MONITOR.lock().unwrap() = Some(SendMonitor(monitor));
    }

    log::info!("鼠标事件监听器已设置");
}

/// 检查指定屏幕坐标处的像素是否不透明
fn is_pixel_opaque(
    window: id,
    screen_point: cocoa::foundation::NSPoint,
    window_frame: NSRect,
) -> bool {
    let window_key = window as u64;
    let map = hit_test_map().lock().unwrap();

    if let Some((pixel_data, bitmap_width, bitmap_height, scale, _frame)) = map.get(&window_key) {
        // 屏幕坐标 → 窗口坐标（Y=0 在底部）
        let win_x = screen_point.x - window_frame.origin.x;
        let win_y = screen_point.y - window_frame.origin.y;

        // 窗口坐标 → 位图像素坐标（乘以缩放因子）
        let pixel_x = (win_x * scale) as usize;
        let pixel_y = (win_y * scale) as usize;

        if pixel_x < *bitmap_width && pixel_y < *bitmap_height {
            // RGBA 格式：索引 = (y * width + x) * 4 + 3（alpha 通道）
            let index = (pixel_y * bitmap_width + pixel_x) * 4 + 3;
            if index < pixel_data.len() {
                let alpha = pixel_data[index];
                return alpha > 10; // 阈值：alpha > 10 视为不透明
            }
        }
    }

    false // 默认：透明（穿透）
}

pub fn destroy_desktop_overlay() {
    log::info!("销毁 macOS 覆盖窗口...");

    // 移除鼠标监听器
    if let Some(SendMonitor(monitor)) = MOUSE_MONITOR.lock().unwrap().take() {
        unsafe {
            let _: () = msg_send![class!(NSEvent), removeMonitor: monitor];
        }
    }

    let mut overlays = OVERLAY_WINDOWS.lock().unwrap();
    for overlay in overlays.drain(..) {
        unsafe {
            let _: () = msg_send![overlay.window, orderOut: nil];
            let _: () = msg_send![overlay.window, close];
        }
    }

    // 清理命中测试数据
    hit_test_map().lock().unwrap().clear();
}

pub fn has_desktop_overlay() -> bool {
    let overlays = OVERLAY_WINDOWS.lock().unwrap();
    !overlays.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_windows_initially_empty() {
        let overlays = OVERLAY_WINDOWS.lock().unwrap();
        assert!(overlays.is_empty());
    }

    #[test]
    fn test_has_desktop_overlay_default_false() {
        assert!(!has_desktop_overlay());
    }
}
