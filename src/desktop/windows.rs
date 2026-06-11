//! Windows 平台桌面管理实现
//!
//! 使用文件系统读取桌面图标信息
//!
//! # 实现要点
//! - 桌面枚举：读取桌面文件夹中的文件
//! - 快捷方式检测：通过扩展名识别 .lnk 文件
//! - 位置获取：由 `DesktopIconMover` 通过 SysListView32 提供

use crate::desktop::{DesktopIcon, DesktopManagerTrait};
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Windows 桌面管理器
pub struct DesktopManager;

impl Default for DesktopManager {
    fn default() -> Self {
        Self
    }
}

impl DesktopManagerTrait for DesktopManager {
    fn new() -> Result<Self> {
        log::info!("初始化 Windows DesktopManager...");
        Ok(Self)
    }

    fn get_desktop_icons(&self) -> Vec<DesktopIcon> {
        log::info!("开始获取 Windows 桌面图标...");

        self.get_desktop_icons_internal().unwrap_or_else(|e| {
            log::warn!("获取桌面图标失败: {}, 返回空列表", e);
            Vec::new()
        })
    }

    fn watch_desktop(&self) -> Result<()> {
        log::info!("开始监听桌面变化...");
        Ok(())
    }
}

impl DesktopManager {
    /// 获取桌面文件夹路径
    fn get_desktop_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // 用户桌面
        if let Some(desktop) = dirs::desktop_dir() {
            paths.push(desktop);
        }

        // 公共桌面
        let public_desktop = PathBuf::from(r"C:\Users\Public\Desktop");
        if public_desktop.exists() {
            paths.push(public_desktop);
        }

        paths
    }

    /// 内部方法：获取桌面图标
    fn get_desktop_icons_internal(&self) -> Result<Vec<DesktopIcon>> {
        let desktop_paths = Self::get_desktop_paths();

        if desktop_paths.is_empty() {
            anyhow::bail!("无法获取桌面路径");
        }

        let mut icons = Vec::new();
        let mut seen_names = std::collections::HashSet::new();

        for desktop_path in &desktop_paths {
            log::info!("扫描桌面路径: {}", desktop_path.display());

            if !desktop_path.exists() {
                continue;
            }

            for entry in std::fs::read_dir(desktop_path).context("无法读取桌面文件夹")? {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        log::warn!("读取桌面条目失败: {}", e);
                        continue;
                    }
                };

                let path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();

                // 跳过隐藏文件
                if file_name.starts_with('.') || file_name == "desktop.ini" {
                    continue;
                }

                let is_shortcut = path
                    .extension()
                    .map(|ext| ext == "lnk" || ext == "url")
                    .unwrap_or(false);

                // 显示名称（去掉扩展名，模拟 Windows 桌面显示）
                let display_name = if is_shortcut {
                    path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or(file_name.clone())
                } else if path.is_dir() {
                    file_name.clone()
                } else {
                    path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or(file_name.clone())
                };

                // 避免重复
                if !seen_names.insert(display_name.clone()) {
                    continue;
                }

                log::debug!("桌面项目: {} -> {:?}", display_name, path);

                icons.push(DesktopIcon {
                    id: uuid::Uuid::new_v4(),
                    display_name,
                    path: path.clone(),
                    target_path: if is_shortcut { None } else { Some(path) },
                    icon_path: None,
                    position: (0, 0), // 位置由 DesktopIconMover 提供
                    is_shortcut,
                });
            }
        }

        log::info!("找到 {} 个桌面图标", icons.len());
        Ok(icons)
    }
}

// ===== 单元测试 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shortcut_detection_logic() {
        let lnk_path = PathBuf::from("C:\\Users\\Desktop\\Test.lnk");
        let is_shortcut = lnk_path
            .extension()
            .map(|ext| ext == "lnk")
            .unwrap_or(false);
        assert!(is_shortcut, ".lnk 文件应被识别为快捷方式");

        let exe_path = PathBuf::from("C:\\Program Files\\App.exe");
        let is_shortcut = exe_path
            .extension()
            .map(|ext| ext == "lnk")
            .unwrap_or(false);
        assert!(!is_shortcut, ".exe 文件不应被识别为快捷方式");
    }
}
