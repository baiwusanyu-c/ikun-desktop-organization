//! 应用分类匹配算法

use crate::desktop::DesktopIcon;
use crate::classification::database::{AppCategory, AppCategoryDB};

/// 应用匹配器
pub struct AppMatcher {
    db: AppCategoryDB,
}

impl AppMatcher {
    pub fn new(db: AppCategoryDB) -> Self {
        Self { db }
    }

    /// 分类图标
    pub fn classify(&self, icon: &DesktopIcon) -> Option<AppCategory> {
        // 获取可执行文件名
        let exe_name = icon.target_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_lowercase());

        // 获取路径
        let path = icon.target_path
            .as_ref()
            .or(Some(&icon.path))
            .and_then(|p| p.to_str())
            .map(|s| s.to_lowercase());

        // 遍历分类规则
        for category in &self.db.categories {
            // 匹配可执行文件名
            if let Some(exe) = &exe_name {
                for pattern in &category.exe_patterns {
                    if self.match_pattern(exe, pattern) {
                        log::info!("图标 '{}' 匹配分类 '{}' (exe: {})",
                            icon.display_name, category.name, exe);
                        return Some(category.clone());
                    }
                }
            }

            // 匹配路径
            if let Some(p) = &path {
                for pattern in &category.path_patterns {
                    if p.contains(&pattern.to_lowercase()) {
                        log::info!("图标 '{}' 匹配分类 '{}' (path: {})",
                            icon.display_name, category.name, p);
                        return Some(category.clone());
                    }
                }
            }

            // 匹配关键词
            let name_lower = icon.display_name.to_lowercase();
            for keyword in &category.keywords {
                if name_lower.contains(&keyword.to_lowercase()) {
                    log::info!("图标 '{}' 匹配分类 '{}' (keyword: {})",
                        icon.display_name, category.name, keyword);
                    return Some(category.clone());
                }
            }
        }

        // 未匹配
        log::info!("图标 '{}' 未匹配任何分类", icon.display_name);
        None
    }

    /// 匹配模式（支持简单通配符）
    fn match_pattern(&self, text: &str, pattern: &str) -> bool {
        let pattern_lower = pattern.to_lowercase();

        if pattern_lower.contains('*') {
            // 简单通配符匹配
            // TODO: 可以使用 regex 或 glob crate 增强匹配能力
            let parts: Vec<&str> = pattern_lower.split('*').collect();
            if parts.len() == 2 {
                let prefix = parts[0];
                let suffix = parts[1];
                return text.starts_with(prefix) && text.ends_with(suffix);
            }
        }

        text == pattern_lower
    }
}