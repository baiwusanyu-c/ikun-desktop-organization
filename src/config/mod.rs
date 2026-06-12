//! 配置模块
//!
//! 统一 JSON 配置系统，支持导入/导出。

pub mod app_config;

// 保留旧模块（向后兼容，后续逐步迁移）
mod layout;
mod settings;

pub use app_config::*;
pub use layout::DesktopLayout;
#[allow(unused_imports)]
pub use settings::AppSettings;
