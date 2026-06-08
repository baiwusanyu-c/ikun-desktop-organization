//! 应用设置

use serde::{Deserialize, Serialize};

/// 应用设置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// 是否开机自启
    pub auto_start: bool,
    /// 是否自动分类
    pub auto_classify: bool,
    /// 默认分类框样式
    pub default_box_style: BoxStyleSettings,
}

/// 分类框样式设置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxStyleSettings {
    /// 圆角半径
    pub border_radius: f32,
    /// 背景透明度 (0-255)
    pub background_alpha: u8,
    /// 背景颜色 RGB
    pub background_color: (u8, u8, u8),
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_start: false,
            auto_classify: true,
            default_box_style: BoxStyleSettings {
                border_radius: 12.0,
                background_alpha: 150,
                background_color: (200, 200, 220),
            },
        }
    }
}
