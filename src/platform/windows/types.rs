//! Windows 层共享数据类型
//!
//! 桌面图标、屏幕信息、图标像素数据等共享数据结构。
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 桌面图标信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopIcon {
    pub id: uuid::Uuid,
    pub display_name: String,
    pub path: PathBuf,
    pub target_path: Option<PathBuf>,
    pub icon_path: Option<PathBuf>,
    pub position: (i32, i32),
    pub is_shortcut: bool,
}

/// Extra icon info
#[derive(Debug, Clone)]
pub struct IconInfo {
    pub exe_name: Option<String>,
    pub description: Option<String>,
    pub size: Option<u64>,
}

/// Screen info
#[derive(Debug, Clone)]
pub struct ScreenInfo {
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
}

/// 提取出的图标像素数据
#[derive(Debug, Clone)]
pub struct IconData {
    /// RGBA pixel data (row-major, top-to-bottom)
    pub pixels: Vec<u8>,
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
}
