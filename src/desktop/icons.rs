//! 桌面图标数据结构

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 桌面图标信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopIcon {
    /// 图标唯一 ID
    pub id: uuid::Uuid,
    /// 显示名称
    pub display_name: String,
    /// 图标路径（.lnk 或文件路径）
    pub path: PathBuf,
    /// 目标路径（快捷方式指向的实际路径）
    pub target_path: Option<PathBuf>,
    /// 图标图片路径
    pub icon_path: Option<PathBuf>,
    /// 桌面位置 (x, y)
    pub position: (i32, i32),
    /// 是否是快捷方式
    pub is_shortcut: bool,
}

/// 图标额外信息
#[derive(Debug, Clone)]
pub struct IconInfo {
    /// 可执行文件名
    pub exe_name: Option<String>,
    /// 文件描述
    pub description: Option<String>,
    /// 文件大小
    pub size: Option<u64>,
}
