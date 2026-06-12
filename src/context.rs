//! 应用上下文 — 中心状态管理对象
//!
//! 所有模块通过 `AppContext` 共享状态和配置，替代散落的全局变量。
//! 这是整个应用的核心依赖注入点。

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
            panel_visible: true,
            status_message: String::new(),
        }
    }
}

/// 应用上下文（核心依赖注入对象）
///
/// 所有模块通过此对象访问：
/// - 统一配置 (`config`)
/// - 平台服务 (`desktop_manager`)
/// - 分类引擎 (`classifier`)
/// - 运行时状态 (`state`)
/// - 插件注册表 (`plugins`) — TODO: Phase 5
pub struct AppContext {
    /// 统一配置（JSON，可导入/导出）
    pub config: AppConfig,
    /// 桌面管理器（平台相关）
    pub desktop_manager: DesktopManager,
    /// 应用分类器
    pub classifier: AppClassifier,
    /// 运行时状态
    pub state: AppState,
    // TODO: Phase 5 - 插件注册表
    // pub plugins: PluginRegistry,
    // TODO: Phase 4 - 分类框管理器
    // pub box_manager: BoxManager,
}

impl Default for AppContext {
    fn default() -> Self {
        Self {
            config: AppConfig::default(),
            desktop_manager: DesktopManager::default(),
            classifier: AppClassifier::default(),
            state: AppState::default(),
        }
    }
}

impl AppContext {
    /// 创建应用上下文（加载配置 + 初始化各子系统）
    pub fn new() -> anyhow::Result<Self> {
        let config = AppConfig::load()?;
        let desktop_manager = DesktopManager::new().unwrap_or_default();
        let classifier = AppClassifier::new().unwrap_or_default();

        Ok(Self {
            config,
            desktop_manager,
            classifier,
            state: AppState::default(),
        })
    }

    /// 保存配置到磁盘
    pub fn save_config(&mut self) -> anyhow::Result<()> {
        self.config.last_modified = chrono::Utc::now();
        self.config.save()
    }

    /// 导出配置到指定路径
    pub fn export_config(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.config.export_to(path)
    }

    /// 从指定路径导入配置
    pub fn import_config(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        self.config = AppConfig::import_from(path)?;
        self.save_config()
    }

    /// 设置状态消息
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.state.status_message = msg.into();
    }
}
