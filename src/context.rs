//! 应用上下文 — 中心状态管理对象

#![allow(dead_code)] // 应用上下文，部分字段/方法在当前构建路径未使用
//!
//! 所有模块通过 `AppContext` 共享状态和配置，替代散落的全局变量。
//! 这是整个应用的核心依赖注入点。
//!
//! ## 启动流程
//! 1. `AppConfig::load()` 读取 `~/.ikun_organizer.json`
//! 2. `AppContext::new()` 注入配置，初始化各子系统
//! 3. 各模块从 `ctx.config` 读取所需配置

use crate::classification::AppClassifier;
use crate::config::AppConfig;
use crate::platform::{DesktopManager, DesktopManagerTrait};

/// 应用运行时状态
pub struct AppState {
    /// 原生桌面图标是否已隐藏
    pub icons_hidden: bool,
    /// 控制面板是否可见
    pub panel_visible: bool,
    /// 当前状态消息
    pub status_message: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            icons_hidden: false,
            panel_visible: false,
            status_message: String::new(),
        }
    }
}

/// 应用上下文（核心依赖注入对象）
///
/// 所有模块通过此对象访问：
/// - 统一配置 (`config`) — 来自 `~/.ikun_organizer.json`
/// - 平台服务 (`desktop_manager`)
/// - 分类引擎 (`classifier`)
/// - 运行时状态 (`state`)
pub struct AppContext {
    /// 统一配置（JSON，可导入/导出）
    pub config: AppConfig,
    /// 桌面管理器（平台相关）
    pub desktop_manager: DesktopManager,
    /// 应用分类器（使用配置中的兜底规则）
    pub classifier: AppClassifier,
    /// 运行时状态
    pub state: AppState,
}

impl Default for AppContext {
    fn default() -> Self {
        let config = AppConfig::default();
        let classifier = AppClassifier::from_config(&config.classification);
        Self {
            config,
            desktop_manager: DesktopManager,
            classifier,
            state: AppState::default(),
        }
    }
}

impl AppContext {
    /// 创建应用上下文（加载配置 + 初始化各子系统）
    ///
    /// 从 `~/.ikun_organizer.json` 读取配置，文件不存在则使用默认值。
    /// 配置加载失败时优雅降级，不阻塞应用启动。
    pub fn new() -> anyhow::Result<Self> {
        // 1. 加载配置
        let config = AppConfig::load().unwrap_or_else(|e| {
            log::warn!("加载配置失败: {}，使用默认配置", e);
            AppConfig::default()
        });

        log::info!("配置版本: {}", config.version);
        log::info!("日志级别: {}", config.debug.log_level);
        log::info!("启动恢复布局: {}", config.general.restore_layout_on_start);

        // 2. 初始化分类器（使用配置中的兜底分类规则）
        let classifier = AppClassifier::from_config(&config.classification);
        log::info!(
            "初始化分类器，加载 {} 条兜底规则",
            config.classification.len()
        );

        // 3. 初始化桌面管理器
        let desktop_manager = DesktopManager::new().unwrap_or_else(|e| {
            log::warn!("初始化桌面管理器失败: {}，使用默认值", e);
            DesktopManager
        });

        Ok(Self {
            config,
            desktop_manager,
            classifier,
            state: AppState::default(),
        })
    }

    /// 保存配置到 `~/.ikun_organizer.json`
    pub fn save_config(&mut self) -> anyhow::Result<()> {
        self.config.last_modified = chrono::Utc::now();
        self.config.save()
    }

    /// 导出配置到指定路径
    pub fn export_config(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.config.export_to(path)
    }

    /// 从指定路径导入配置并保存
    pub fn import_config(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        self.config = AppConfig::import_from(path)?;
        // 重新初始化分类器（使用新配置的兜底规则）
        self.classifier = AppClassifier::from_config(&self.config.classification);
        self.save_config()
    }

    /// 设置状态消息
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.state.status_message = msg.into();
    }
}
