//! macOS 平台桌面管理实现
//!
//! 使用 macOS API 读取桌面图标信息

use crate::platform::types::DesktopIcon;
use crate::platform::DesktopManagerTrait;
use anyhow::Result;
use std::path::PathBuf;

/// macOS 桌面管理器
pub struct DesktopManager;

impl Default for DesktopManager {
    fn default() -> Self {
        Self
    }
}

impl DesktopManagerTrait for DesktopManager {
    fn new() -> Result<Self> {
        log::info!("macOS DesktopManager 初始化");
        Ok(Self)
    }

    fn get_desktop_icons(&self) -> Vec<DesktopIcon> {
        // TODO: 实现实际的 macOS 桌面图标读取
        log::info!("获取 macOS 桌面图标...");

        // 返回模拟数据（用于 MVP 测试）
        vec![
            DesktopIcon {
                id: uuid::Uuid::new_v4(),
                display_name: "Safari".to_string(),
                path: PathBuf::from("/Users/Desktop/Safari.app"),
                target_path: Some(PathBuf::from("/Applications/Safari.app")),
                icon_path: None,
                position: (100, 100),
                is_shortcut: false,
            },
            DesktopIcon {
                id: uuid::Uuid::new_v4(),
                display_name: "Xcode".to_string(),
                path: PathBuf::from("/Users/Desktop/Xcode.app"),
                target_path: Some(PathBuf::from("/Applications/Xcode.app")),
                icon_path: None,
                position: (200, 100),
                is_shortcut: false,
            },
        ]
    }

    fn watch_desktop(&self) -> Result<()> {
        // TODO: 实现桌面变化监听
        log::info!("开始监听 macOS 桌面变化...");
        Ok(())
    }
}
