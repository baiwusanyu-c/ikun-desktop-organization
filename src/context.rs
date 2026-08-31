//! Application context - central state management object
#![allow(dead_code)]
//!
//! All modules share state and configuration via AppContext.
//! This is the core dependency injection point.
//!
//! ## Startup flow
//! 1. AppConfig::load() reads ~/.ikun_organizer.json
//! 2. AppContext::new() injects config, initializes subsystems
//! 3. Modules read needed config from ctx.config

use crate::config::AppConfig;
use crate::platform::{
    create_desktop_manager, create_platform_service, DesktopManagerTrait, PlatformService,
};

/// Application runtime state
#[derive(Default)]
pub struct AppState {
    /// Whether native desktop icons are hidden
    pub icons_hidden: bool,
    /// Whether control panel is visible
    pub panel_visible: bool,
    /// Current status message
    pub status_message: String,
}

/// Application context (core dependency injection object)
///
/// All modules access:
/// - Unified config (config) from ~/.ikun_organizer.json
/// - Platform services (desktop_manager, platform_service) via trait objects
/// - Runtime state (state)
pub struct AppContext {
    /// Unified config (JSON, importable/exportable)
    pub config: AppConfig,
    /// Desktop manager (platform-specific, via trait object)
    pub desktop_manager: Box<dyn DesktopManagerTrait>,
    /// Platform service (tray, auto-start, control panel, via trait object)
    pub platform_service: Box<dyn PlatformService>,
    /// Runtime state
    pub state: AppState,
    /// 配置是否从文件成功加载。
    ///
    /// 为 `false` 时 `config` 是回退的默认值（auto_start=false 等），
    /// 任何"按配置同步系统状态"的逻辑（如开机自启注册表同步）都必须跳过，
    /// 否则会把默认值写回系统，静默删除用户已开启的自启条目。
    pub config_loaded: bool,
}

impl Default for AppContext {
    fn default() -> Self {
        let config = AppConfig::default();
        Self {
            config,
            desktop_manager: create_desktop_manager(),
            platform_service: create_platform_service(),
            state: AppState::default(),
            config_loaded: false,
        }
    }
}

impl AppContext {
    /// Create application context (load config + initialize subsystems)
    ///
    /// Reads config from ~/.ikun_organizer.json, falls back to defaults.
    /// Config load failure degrades gracefully, does not block app startup;
    /// `config_loaded` records the outcome so callers can avoid syncing
    /// system state (e.g. auto-start registry) from fallback defaults.
    pub fn new() -> anyhow::Result<Self> {
        // 1. Load config
        let mut config_loaded = true;
        let config = AppConfig::load().unwrap_or_else(|e| {
            config_loaded = false;
            log::error!(
                "配置加载失败，回退默认配置（不会按默认值同步系统状态）: {}",
                e
            );
            AppConfig::default()
        });

        log::info!("Config version: {}", config.version);
        log::info!("Log level: {}", config.debug.log_level);
        log::info!(
            "Restore layout on start: {}",
            config.general.restore_layout_on_start
        );

        // Initialize global liquid glass config
        #[cfg(target_os = "windows")]
        crate::platform::windows::liquid_glass::update_liquid_glass_config(
            &config.box_style.liquid_glass,
        );

        // 2. Initialize platform services via trait objects
        let desktop_manager = create_desktop_manager();
        let platform_service = create_platform_service();

        Ok(Self {
            config,
            desktop_manager,
            platform_service,
            state: AppState::default(),
            config_loaded,
        })
    }

    /// Save config to ~/.ikun_organizer.json
    pub fn save_config(&mut self) -> anyhow::Result<()> {
        self.config.last_modified = chrono::Utc::now();
        self.config.save()
    }

    /// Export config to specified path
    pub fn export_config(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.config.export_to(path)
    }

    /// Import config from specified path and save
    pub fn import_config(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        self.config = AppConfig::import_from(path)?;
        self.save_config()
    }

    /// Set status message
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.state.status_message = msg.into();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_context_default_creates_valid_state() {
        let ctx = AppContext::default();
        assert!(!ctx.state.icons_hidden);
        assert!(!ctx.state.panel_visible);
        assert!(ctx.state.status_message.is_empty());
    }

    #[test]
    fn test_app_context_default_creates_trait_objects() {
        let ctx = AppContext::default();
        // DesktopManager trait object should be able to enumerate icons
        // (may return empty in test environment, but should not panic)
        let icons = ctx.desktop_manager.get_desktop_icons();
        let _ = icons.len();
    }

    #[test]
    fn test_app_context_default_platform_service() {
        let ctx = AppContext::default();
        // PlatformService should be callable without panic
        let grid_count = ctx.platform_service.get_grid_count();
        assert_eq!(grid_count, 0, "No grid windows in test environment");
    }

    #[test]
    fn test_app_context_set_status() {
        let mut ctx = AppContext::default();
        assert!(ctx.state.status_message.is_empty());
        ctx.set_status("test message");
        assert_eq!(ctx.state.status_message, "test message");
        ctx.set_status("updated");
        assert_eq!(ctx.state.status_message, "updated");
    }

    #[test]
    fn test_app_context_new_loads_config() {
        let ctx = AppContext::new();
        assert!(ctx.is_ok(), "AppContext::new() should not fail");
        let ctx = ctx.unwrap();
        assert!(!ctx.config.version.is_empty());
    }

    #[test]
    fn test_app_context_default_marks_config_not_loaded() {
        // Default 上下文的配置是回退默认值，不允许作为同步系统状态的依据
        let ctx = AppContext::default();
        assert!(!ctx.config_loaded);
        assert!(!ctx.config.general.auto_start);
    }
}
