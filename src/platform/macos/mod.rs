//! macOS 平台特定实现

pub mod desktop_manager;
pub mod desktop_overlay;
pub mod ffi_imports;
pub mod icon_extract;

pub use desktop_manager::DesktopManager;
pub use desktop_overlay::{create_desktop_overlay, destroy_desktop_overlay, has_desktop_overlay};

use anyhow::Result;
use std::process::Command;

/// 隐藏 macOS 桌面图标
///
/// 使用 `defaults write com.apple.finder CreateDesktop -bool false` 隐藏所有桌面图标。
/// 这是 macOS 上隐藏桌面图标的标准方法。
pub fn hide_native_icons() -> Result<()> {
    log::info!("隐藏 macOS 桌面图标...");

    let output = Command::new("defaults")
        .args(&[
            "write",
            "com.apple.finder",
            "CreateDesktop",
            "-bool",
            "false",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!("defaults write 失败: {}", stderr);
        anyhow::bail!("defaults write 失败: {}", stderr);
    }

    // 重启 Finder 以应用更改
    let _ = Command::new("killall").arg("Finder").output();
    log::info!("Finder 已重启，桌面图标已隐藏");

    Ok(())
}

/// 恢复 macOS 桌面图标
pub fn restore_native_icons() -> Result<()> {
    log::info!("恢复 macOS 桌面图标...");

    let output = Command::new("defaults")
        .args(&[
            "write",
            "com.apple.finder",
            "CreateDesktop",
            "-bool",
            "true",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!("defaults write 失败: {}", stderr);
    }

    let _ = Command::new("killall").arg("Finder").output();
    log::info!("Finder 已重启，桌面图标已恢复");

    Ok(())
}

/// 获取屏幕分辨率
pub fn get_screen_resolution() -> (i32, i32) {
    use cocoa::appkit::NSScreen;
    use cocoa::base::nil;
    use cocoa::foundation::NSRect;
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let screen: cocoa::base::id = msg_send![class!(NSScreen), mainScreen];
        if screen == nil {
            return (1920, 1080);
        }
        let frame: NSRect = msg_send![screen, frame];
        (
            (frame.size.width as f32 * frame.size.width as f32 / frame.size.width as f32) as i32,
            frame.size.height as i32,
        )
    }
}
