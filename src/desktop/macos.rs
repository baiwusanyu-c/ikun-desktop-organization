//! macOS 平台桌面管理实现
//!
//! 使用 macOS API 读取桌面图标信息

use crate::desktop::{DesktopIcon, DesktopManagerTrait};
use anyhow::Result;
use std::path::PathBuf;

/// macOS 桌面管理器
pub struct DesktopManager {
    /// 桌面路径
    desktop_path: PathBuf,
}

impl Default for DesktopManager {
    fn default() -> Self {
        Self {
            desktop_path: dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("~/Desktop")),
        }
    }
}

impl DesktopManagerTrait for DesktopManager {
    fn new() -> Result<Self> {
        // 获取桌面路径
        let desktop_path =
            dirs::desktop_dir().ok_or_else(|| anyhow::anyhow!("无法获取桌面路径"))?;

        log::info!("macOS DesktopManager 初始化, 桌面路径: {:?}", desktop_path);

        Ok(Self { desktop_path })
    }

    fn get_desktop_icons(&self) -> Vec<DesktopIcon> {
        // TODO: 实现实际的 macOS 桌面图标读取
        // 1. 读取 ~/Desktop 文件夹内容
        // 2. 获取 Finder 图标位置信息（需要特殊权限）

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
