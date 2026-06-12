//! 分类数据库
//!
//! 从 YAML 文件加载应用分类数据

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 应用分类数据库
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppCategoryDB {
    /// 版本
    pub version: String,
    /// 分类列表
    pub categories: Vec<AppCategory>,
}

/// 应用分类
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppCategory {
    /// 分类 ID
    pub id: String,
    /// 分类名称
    pub name: String,
    /// 分类图标（emoji 或图片路径）
    pub icon: String,
    /// 可执行文件匹配模式
    pub exe_patterns: Vec<String>,
    /// 路径匹配模式
    pub path_patterns: Vec<String>,
    /// 关键词
    pub keywords: Vec<String>,
}

impl AppCategoryDB {
    /// 从文件加载分类数据库
    pub fn load() -> anyhow::Result<Self> {
        // 尝试从用户配置目录加载
        let user_path = dirs::config_dir()
            .map(|p| p.join("ikun-desktop-organization/categories.yaml"))
            .unwrap_or_else(|| PathBuf::from(""));

        if !user_path.as_os_str().is_empty() && user_path.exists() {
            let content = std::fs::read_to_string(&user_path)?;
            let db: AppCategoryDB = serde_yaml::from_str(&content)?;
            log::info!("加载用户自定义分类数据库: {:?}", user_path);
            return Ok(db);
        }

        // 返回默认分类
        log::info!("使用默认分类数据库");
        Ok(Self::default())
    }

    /// 保存分类数据库
    pub fn save(&self) -> anyhow::Result<()> {
        let path = dirs::config_dir()
            .map(|p| p.join("ikun-desktop-organization/categories.yaml"))
            .ok_or_else(|| anyhow::anyhow!("无法获取配置目录"))?;

        std::fs::create_dir_all(path.parent().unwrap())?;
        let content = serde_yaml::to_string(self)?;
        std::fs::write(path, content)?;

        log::info!("保存分类数据库成功");
        Ok(())
    }
}

impl Default for AppCategoryDB {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            categories: vec![
                AppCategory {
                    id: "games".to_string(),
                    name: "游戏".to_string(),
                    icon: "🎮".to_string(),
                    exe_patterns: vec![
                        "steam.exe".to_string(),
                        "epicgameslauncher.exe".to_string(),
                        "lol.exe".to_string(),
                    ],
                    path_patterns: vec!["\\Games\\".to_string(), "\\Steam\\".to_string()],
                    keywords: vec!["game".to_string(), "play".to_string(), "娱乐".to_string()],
                },
                AppCategory {
                    id: "dev".to_string(),
                    name: "开发工具".to_string(),
                    icon: "💻".to_string(),
                    exe_patterns: vec![
                        "code.exe".to_string(),
                        "idea*.exe".to_string(),
                        "rust*.exe".to_string(),
                    ],
                    path_patterns: vec!["\\Dev\\".to_string(), "\\IDE\\".to_string()],
                    keywords: vec![
                        "dev".to_string(),
                        "code".to_string(),
                        "ide".to_string(),
                        "editor".to_string(),
                    ],
                },
                AppCategory {
                    id: "office".to_string(),
                    name: "办公".to_string(),
                    icon: "📄".to_string(),
                    exe_patterns: vec![
                        "word.exe".to_string(),
                        "excel.exe".to_string(),
                        "powerpoint.exe".to_string(),
                    ],
                    path_patterns: vec![
                        "\\Office\\".to_string(),
                        "\\Microsoft Office\\".to_string(),
                    ],
                    keywords: vec![
                        "office".to_string(),
                        "document".to_string(),
                        "文档".to_string(),
                    ],
                },
                AppCategory {
                    id: "media".to_string(),
                    name: "媒体".to_string(),
                    icon: "🎵".to_string(),
                    exe_patterns: vec!["vlc.exe".to_string(), "spotify.exe".to_string()],
                    path_patterns: vec!["\\Music\\".to_string(), "\\Video\\".to_string()],
                    keywords: vec![
                        "music".to_string(),
                        "video".to_string(),
                        "media".to_string(),
                        "音乐".to_string(),
                    ],
                },
                AppCategory {
                    id: "tools".to_string(),
                    name: "工具".to_string(),
                    icon: "🔧".to_string(),
                    exe_patterns: vec!["notepad.exe".to_string(), "calc.exe".to_string()],
                    path_patterns: vec!["\\Tools\\".to_string(), "\\Utilities\\".to_string()],
                    keywords: vec![
                        "tool".to_string(),
                        "utility".to_string(),
                        "工具".to_string(),
                    ],
                },
            ],
        }
    }
}
