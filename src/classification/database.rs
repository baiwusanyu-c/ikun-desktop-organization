//! 分类数据库
//!
//! 管理应用分类规则数据。分类规则来自配置文件 `classification` 段。

use crate::config::CategoryRule;
use serde::{Deserialize, Serialize};

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
    /// 从配置的分类规则构建分类数据库
    pub fn from_category_rules(rules: &[CategoryRule]) -> Self {
        Self {
            version: "2.0".to_string(),
            categories: rules
                .iter()
                .map(|r| AppCategory {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    icon: r.icon.clone(),
                    exe_patterns: r.exe_patterns.clone(),
                    path_patterns: r.path_patterns.clone(),
                    keywords: r.keywords.clone(),
                })
                .collect(),
        }
    }
}

impl Default for AppCategoryDB {
    fn default() -> Self {
        let rules = crate::config::app_config::default_categories();
        Self::from_category_rules(&rules)
    }
}
