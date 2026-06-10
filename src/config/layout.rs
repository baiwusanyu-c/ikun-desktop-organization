//! 桌面布局管理
//!
//! 保存和恢复分类框布局

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 桌面布局
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopLayout {
    /// 版本
    pub version: String,
    /// 最后修改时间
    pub last_modified: chrono::DateTime<chrono::Utc>,
    /// 分类框数据
    pub boxes: Vec<BoxData>,
}

/// 分类框数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxData {
    /// 框 ID
    pub id: uuid::Uuid,
    /// 框名称
    pub name: String,
    /// 位置 (x, y)
    pub position: (f32, f32),
    /// 尺寸 (width, height)
    pub size: (f32, f32),
    /// 是否锁定
    pub locked: bool,
    /// 包含的图标 ID
    pub icon_ids: Vec<uuid::Uuid>,
    /// 包含的图标名称（用于重新匹配 SysListView32 index）
    pub icon_names: Vec<String>,
}

impl DesktopLayout {
    /// 加载布局
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::get_layout_path()?;

        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let layout: DesktopLayout = serde_json::from_str(&content)?;
            log::info!("加载布局成功");
            Ok(layout)
        } else {
            log::info!("使用默认布局");
            Ok(Self::default())
        }
    }

    /// 保存布局
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::get_layout_path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;

        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;

        log::info!("保存布局成功");
        Ok(())
    }

    /// 获取布局文件路径
    fn get_layout_path() -> anyhow::Result<PathBuf> {
        dirs::config_dir()
            .map(|p| p.join("ikun-desktop-organization/layout.json"))
            .ok_or_else(|| anyhow::anyhow!("无法获取配置目录"))
    }
}

impl Default for DesktopLayout {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            last_modified: chrono::Utc::now(),
            boxes: Vec::new(),
        }
    }
}
