//! macOS 桌面管理器
//!
//! 扫描桌面目录获取文件列表

use crate::platform::types::DesktopIcon;
use crate::platform::DesktopManagerTrait;
use anyhow::Result;
use std::path::PathBuf;
use uuid::Uuid;

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
        log::info!("扫描桌面图标...");

        let desktop_path = dirs::home_dir()
            .map(|h| h.join("Desktop"))
            .unwrap_or_else(|| PathBuf::from("/Users/baiwusanyu/Desktop"));

        let mut icons = Vec::new();

        match std::fs::read_dir(&desktop_path) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        let file_name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();

                        // 跳过隐藏文件
                        if file_name.starts_with('.') {
                            continue;
                        }

                        icons.push(DesktopIcon {
                            id: Uuid::new_v4(),
                            display_name: file_name,
                            path: path.clone(),
                            target_path: None,
                            icon_path: None,
                            position: (0, 0),
                            is_shortcut: false,
                        });
                    }
                }
            }
            Err(e) => {
                log::error!("读取桌面目录失败: {}", e);
            }
        }

        log::info!("扫描到 {} 个桌面图标", icons.len());
        icons
    }

    fn watch_desktop(&self) -> Result<()> {
        log::info!("开始监听 macOS 桌面变化...");
        Ok(())
    }
}
