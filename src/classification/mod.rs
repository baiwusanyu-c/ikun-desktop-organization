//! 应用分类模块
//!
//! 负责智能分类桌面图标

pub mod ai_classifier;
mod database;
mod matcher;

pub use database::{AppCategory, AppCategoryDB};
pub use matcher::AppMatcher;

use crate::platform::DesktopIcon;

/// 应用分类器
pub struct AppClassifier {
    /// 分类数据库
    db: AppCategoryDB,
    /// 匹配器
    matcher: AppMatcher,
}

impl Default for AppClassifier {
    fn default() -> Self {
        let db = AppCategoryDB::default();
        let matcher = AppMatcher::new(db.clone());
        Self { db, matcher }
    }
}

impl AppClassifier {
    pub fn new() -> anyhow::Result<Self> {
        let db = AppCategoryDB::load()?;
        let matcher = AppMatcher::new(db.clone());

        Ok(Self { db, matcher })
    }

    /// 分类单个图标
    pub fn classify(&self, icon: &DesktopIcon) -> Option<AppCategory> {
        self.matcher.classify(icon)
    }

    /// 获取分类信息
    pub fn get_category(&self, id: &str) -> Option<AppCategory> {
        self.db.categories.iter().find(|c| c.id == id).cloned()
    }

    /// 获取所有分类
    pub fn get_all_categories(&self) -> Vec<AppCategory> {
        self.db.categories.clone()
    }
}
