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

use crate::classification::AppClassifier;
use crate::config::AppConfig;
use crate::platform::{
    create_desktop_manager, create_platform_service, DesktopManagerTrait, PlatformService,
};

/// Application runtime state
pub struct AppState {
    /// Whether native desktop icons are hidden
    pub icons_hidden: bool,
    /// Whether control panel is visible
    pub panel_visible: bool,
    /// Current status message
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

/// Application context (core dependency injection object)
///
/// All modules access:
/// - Unified config (config) from ~/.ikun_organizer.json
/// - Platform services (desktop_manager, platform_service) via trait objects
/// - Classification engine (classifier)
/// - Runtime state (state)
pub struct AppContext {
    /// Unified config (JSON, importable/exportable)
    pub config: AppConfig,
    /// Desktop manager (platform-specific, via trait object)
    pub desktop_manager: Box<dyn DesktopManagerTrait>,
    /// Platform service (tray, auto-start, control panel, via trait object)
    pub platform_service: Box<dyn PlatformService>,
    /// App classifier (using fallback rules from config)
    pub classifier: AppClassifier,
    /// Runtime state
    pub state: AppState,
}

impl Default for AppContext {
    fn default() -> Self {
        let config = AppConfig::default();
        let classifier = AppClassifier::from_config(&config.classification);
        Self {
            config,
            desktop_manager: create_desktop_manager(),
            platform_service: create_platform_service(),
            classifier,
            state: AppState::default(),
        }
    }
}

impl AppContext {
    /// Create application context (load config + initialize subsystems)
    ///
    /// Reads config from ~/.ikun_organizer.json, falls back to defaults.
    /// Config load failure degrades gracefully, does not block app startup.
    pub fn new() -> anyhow::Result<Self> {
        // 1. Load config
        let config = AppConfig::load().unwrap_or_else(|e| {
            log::warn!("Failed to load config: {}, using defaults", e);
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

        // 2. Initialize classifier (using fallback classification rules)
        let classifier = AppClassifier::from_config(&config.classification);
        log::info!(
            "Initialized classifier with {} fallback rules",
            config.classification.len()
        );

        // 3. Initialize platform services via trait objects
        let desktop_manager = create_desktop_manager();
        let platform_service = create_platform_service();

        Ok(Self {
            config,
            desktop_manager,
            platform_service,
            classifier,
            state: AppState::default(),
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
        // Reinitialize classifier (using new config's fallback rules)
        self.classifier = AppClassifier::from_config(&self.config.classification);
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
    fn test_app_context_default_classifier_initialized() {
        let ctx = AppContext::default();
        let _ = &ctx.classifier;
    }
}
