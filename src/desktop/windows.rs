//! Windows 平台桌面管理实现
//!
//! 使用 Windows Shell API 读取桌面图标信息

use crate::desktop::{DesktopIcon, DesktopManagerTrait, icons::IconInfo};
use anyhow::Result;
use std::path::PathBuf;

/// Windows 桌面管理器
pub struct DesktopManager {
    /// COM 是否已初始化
    com_initialized: bool,
}

impl DesktopManagerTrait for DesktopManager {
    fn new() -> Result<Self> {
        // 初始化 COM
        // TODO: 实现实际的 COM 初始化
        // use windows::Win32::System::Com::*;
        // CoInitializeEx(None, COINIT_APARTMENTTHUNKED)?;

        log::info!("Windows DesktopManager 初始化");

        Ok(Self {
            com_initialized: true,
        })
    }

    fn get_desktop_icons(&self) -> Vec<DesktopIcon> {
        // TODO: 实现实际的 Windows Shell API 调用
        // 1. SHGetDesktopFolder
        // 2. IShellFolder::EnumObjects
        // 3. 解析每个图标的信息

        log::info!("获取 Windows 桌面图标...");

        // 返回模拟数据（用于 MVP 测试）
        vec![
            DesktopIcon {
                id: uuid::Uuid::new_v4(),
                display_name: "VS Code".to_string(),
                path: PathBuf::from("C:\\Users\\Desktop\\VS Code.lnk"),
                target_path: Some(PathBuf::from("C:\\Program Files\\VS Code\\Code.exe")),
                icon_path: None,
                position: (100, 100),
                is_shortcut: true,
            },
            DesktopIcon {
                id: uuid::Uuid::new_v4(),
                display_name: "Steam".to_string(),
                path: PathBuf::from("C:\\Users\\Desktop\\Steam.lnk"),
                target_path: Some(PathBuf::from("C:\\Program Files\\Steam\\Steam.exe")),
                icon_path: None,
                position: (200, 100),
                is_shortcut: true,
            },
        ]
    }

    fn watch_desktop(&self) -> Result<()> {
        // TODO: 实现桌面变化监听
        // 使用 notify crate 监听桌面文件夹变化
        log::info!("开始监听桌面变化...");
        Ok(())
    }
}

impl Drop for DesktopManager {
    fn drop(&mut self) {
        if self.com_initialized {
            // 释放 COM
            // CoUninitialize();
            log::info!("释放 Windows COM");
        }
    }
}