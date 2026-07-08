//! macOS 覆盖窗口管理

use crate::platform::macos::ffi_imports::*;
use anyhow::Result;
use cocoa::appkit::NSBackingStoreBuffered;
use cocoa::appkit::NSWindowStyleMask;
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::NSRect;
use objc::{msg_send, sel, sel_impl};
use std::sync::Mutex;

use super::overlay_render::render_overlay_placeholder;

pub(crate) struct OverlayWindow {
    pub(crate) window: id,
    pub(crate) screen_index: usize,
}

unsafe impl Send for OverlayWindow {}

static OVERLAY_WINDOWS: Mutex<Vec<OverlayWindow>> = Mutex::new(Vec::new());

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
            screen_infos.push(ScreenInfo { frame, scale_factor });
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
    
    // 创建新窗口
    for (index, screen) in screens.iter().enumerate() {
        match create_overlay_for_screen(screen, index) {
            Ok(window) => {
                log::info!("屏幕 {} 覆盖窗口创建成功", index);
                overlays.push(OverlayWindow { window, screen_index: index });
            }
            Err(e) => {
                log::error!("屏幕 {} 覆盖窗口创建失败: {}", index, e);
            }
        }
    }
    Ok(())
}

fn create_overlay_for_screen(screen: &ScreenInfo, index: usize) -> Result<id> {
    unsafe {
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![
            window,
            initWithContentRect:screen.frame
            styleMask:NSWindowStyleMask::NSBorderlessWindowMask
            backing:NSBackingStoreBuffered
            defer:NO
        ];
        if window == nil {
            anyhow::bail!("创建 NSWindow 失败");
        }

        let _: () = msg_send![window, setOpaque: NO];
        let clear_color: id = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![window, setBackgroundColor: clear_color];
        let _: () = msg_send![window, setHasShadow: NO];
        let _: () = msg_send![window, setIgnoresMouseEvents: YES];

        // 尝试使用 -1 层级（刚好低于普通窗口）
        // kCGDesktopWindowLevel (-2147483603) 在某些情况下可能被 macOS 限制
        let level = -1i64;
        let _: () = msg_send![window, setLevel: level];

        let actual_level: i64 = msg_send![window, level];
        log::info!("屏幕 {} 覆盖窗口层级: 设置={}, 实际={}", index, level, actual_level);
        log::info!("屏幕 {} 覆盖窗口 frame: ({}, {}, {}x{})", index, screen.frame.origin.x, screen.frame.origin.y, screen.frame.size.width, screen.frame.size.height);

        render_overlay_placeholder(window, screen);
        let _: () = msg_send![window, orderFrontRegardless];
        Ok(window)
    }
}

pub fn destroy_desktop_overlay() {
    log::info!("销毁 macOS 覆盖窗口...");
    let mut overlays = OVERLAY_WINDOWS.lock().unwrap();
    for overlay in overlays.drain(..) {
        unsafe { 
            let _: () = msg_send![overlay.window, orderOut: nil];
            let _: () = msg_send![overlay.window, close]; 
        }
    }
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
