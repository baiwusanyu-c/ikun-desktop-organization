//! 分类数据库
//!
//! 从 YAML 文件加载应用分类数据

use serde::{Serialize, Deserialize};
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
        // 尝试从多个位置加载
        let paths = [
            PathBuf::from("data/categories.yaml"),
            dirs::config_dir()
                .map(|p| p.join("ikun-desktop-organization/categories.yaml"))
        ];

        for path in paths.iter().flatten() {
            if path.exists() {
                let content = std::fs::read_to_string(path)?;
                let db: AppCategoryDB = serde_yaml::from_str(&content)?;
                log::info!("加载分类数据库成功，版本: {}", db.version);
                return Ok(db);
            }
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
                    exe_patterns: vec!["steam.exe", "epicgameslauncher.exe", "lol.exe"],
                    path_patterns: vec!["\\Games\\", "\\Steam\\"],
                    keywords: vec!["game", "play", "娱乐"],
                },
                AppCategory {
                    id: "dev".to_string(),
                    name: "开发工具".to_string(),
                    icon: "💻".to_string(),
                    exe_patterns: vec!["code.exe", "idea*.exe", "rust*.exe"],
                    path_patterns: vec!["\\Dev\\", "\\IDE\\"],
                    keywords: vec!["dev", "code", "ide", "editor"],
                },
                AppCategory {
                    id: "office".to_string(),
                    name: "办公".to_string(),
                    icon: "📄".to_string(),
                    exe_patterns: vec!["word.exe", "excel.exe", "powerpoint.exe"],
                    path_patterns: vec!["\\Office\\", "\\Microsoft Office\\"],
                    keywords: vec!["office", "document", "文档"],
                },
                AppCategory {
                    id: "media".to_string(),
                    name: "媒体".to_string(),
                    icon: "🎵".to_string(),
                    exe_patterns: vec!["vlc.exe", "spotify.exe"],
                    path_patterns: vec!["\\Music\\", "\\Video\\"],
                    keywords: vec!["music", "video", "media", "音乐"],
                },
                AppCategory {
                    id: "tools".to_string(),
                    name: "工具".to_string(),
                    icon: "🔧".to_string(),
                    exe_patterns: vec!["notepad.exe", "calc.exe"],
                    path_patterns: vec!["\\Tools\\", "\\Utilities\\"],
                    keywords: vec!["tool", "utility", "工具"],
                },
            ],
        }
    }
}