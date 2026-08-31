//! Platform abstraction layer
#![allow(dead_code)]
//!
//! Cross-platform desktop management interface via trait.
//!
//! # Architecture
//! - DesktopManagerTrait: desktop icon management (enum, extract, launch, monitor)
//! - PlatformService: system tray, auto-start, control panel
//! - AppContext holds Box<dyn DesktopManagerTrait> + Box<dyn PlatformService>

pub mod types;

#[cfg(windows)]
pub mod windows;

pub use types::*;

/// System tray menu actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemTrayAction {
    OpenControlPanel,
    CreateBox,
    AiClassify,
    ToggleAutoStart,
    Quit,
}

#[cfg(windows)]
pub use windows::DesktopManager;

#[cfg(windows)]
#[allow(unused_imports)]
pub use windows::{
    cleanup_desktop_registry, cleanup_system_tray, clsid_display_name, collect_grid_layout,
    create_grid_window, destroy_all_grid_windows, extract_file_icon,
    get_cursor_monitor_work_area_phys, get_grid_count, get_primary_monitor_dpi,
    get_primary_monitor_work_area_phys, get_screen_resolution, get_vault_display_names,
    hide_control_panel_window, icon_exists_for_box, register_tray_action_waker,
    reposition_and_resize_window, reposition_window_by_title, rerender_all_grid_windows,
    resolve_icon_read_path, restore_all_hidden_icons, set_tray_auto_start_state,
    setup_desktop_registry, setup_global_hotkey, setup_system_tray, show_control_panel_window,
    show_desktop_context_menu, take_hotkey_toggle, take_pending_create_box,
    take_pending_tray_action, unregister_global_hotkey, update_grid_window_title, GridCreateParams,
    GridFileItem,
};

// =============================================================================
// Cross-platform Trait definitions
// =============================================================================

/// Desktop manager trait (cross-platform).
///
/// Windows and macOS each implement this trait.
/// AppContext holds it as Box<dyn DesktopManagerTrait>.
pub trait DesktopManagerTrait: Send + Sync {
    /// Enumerate desktop icons
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;

    /// Watch desktop changes
    fn watch_desktop(&self) -> anyhow::Result<()>;

    /// Extract file icon pixel data
    fn extract_icon(&self, path: &str) -> anyhow::Result<IconData>;

    /// Launch file with system default handler
    fn launch_file(&self, path: &str) -> anyhow::Result<()>;

    /// Get screen resolution (physical pixels)
    fn get_screen_resolution(&self) -> (i32, i32);

    /// Get primary monitor DPI
    fn get_primary_monitor_dpi(&self) -> u32;

    /// Get monitor work area at given point (physical pixels)
    fn get_monitor_work_area(&self, point: (i32, i32)) -> (i32, i32, i32, i32);
}

/// Platform service trait (cross-platform).
///
/// Consolidates platform-specific free functions into a single trait.
pub trait PlatformService: Send + Sync {
    /// Get classification box count
    fn get_grid_count(&self) -> usize;

    /// Collect all box layout data
    fn collect_grid_layout(&self) -> Vec<crate::config::app_config::BoxLayoutData>;

    /// Setup system tray
    fn setup_system_tray(&self) -> anyhow::Result<()>;

    /// Cleanup system tray
    fn cleanup_system_tray(&self);

    /// Get pending tray action
    fn take_pending_tray_action(&self) -> Option<SystemTrayAction>;

    /// Set tray auto-start state
    fn set_tray_auto_start_state(&self, enabled: bool);

    /// Show control panel window
    fn show_control_panel(&self) -> bool;

    /// Hide control panel window
    fn hide_control_panel(&self) -> bool;

    /// Set auto-start on boot
    fn set_auto_start(&self, enabled: bool) -> anyhow::Result<()>;

    /// Query auto-start status
    fn is_auto_start_enabled(&self) -> bool;
}

// =============================================================================
// Platform factory functions
// =============================================================================

/// Create platform-specific DesktopManager instance
#[cfg(windows)]
pub fn create_desktop_manager() -> Box<dyn DesktopManagerTrait> {
    Box::new(DesktopManager::new().unwrap_or_else(|e| {
        log::warn!("Desktop manager init failed: {}, using default", e);
        DesktopManager
    }))
}

/// Create platform-specific PlatformService instance
#[cfg(windows)]
pub fn create_platform_service() -> Box<dyn PlatformService> {
    Box::new(windows::WindowsPlatformService)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_desktop_manager_returns_trait_object() {
        let dm = create_desktop_manager();
        // Should be able to call trait methods without panic
        let _icons = dm.get_desktop_icons();
    }

    #[test]
    fn test_create_platform_service_returns_trait_object() {
        let ps = create_platform_service();
        // Grid count should be 0 in test environment
        assert_eq!(ps.get_grid_count(), 0);
    }

    #[test]
    fn test_desktop_manager_screen_resolution() {
        let dm = create_desktop_manager();
        let (w, h) = dm.get_screen_resolution();
        assert!(w > 0, "Screen width should be positive");
        assert!(h > 0, "Screen height should be positive");
    }

    #[test]
    fn test_desktop_manager_dpi() {
        let dm = create_desktop_manager();
        let dpi = dm.get_primary_monitor_dpi();
        assert!(dpi >= 96, "DPI should be at least 96");
        assert!(dpi <= 600, "DPI should not exceed 600");
    }

    #[test]
    fn test_desktop_manager_monitor_work_area() {
        let dm = create_desktop_manager();
        let (_x, _y, w, h) = dm.get_monitor_work_area((0, 0));
        assert!(w > 0, "Work area width should be positive");
        assert!(h > 0, "Work area height should be positive");
    }

    #[test]
    fn test_system_tray_action_eq() {
        assert_eq!(
            SystemTrayAction::OpenControlPanel,
            SystemTrayAction::OpenControlPanel
        );
        assert_ne!(SystemTrayAction::OpenControlPanel, SystemTrayAction::Quit);
    }

    #[test]
    fn test_system_tray_action_clone() {
        let a = SystemTrayAction::CreateBox;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn test_platform_service_collect_layout() {
        let ps = create_platform_service();
        let layouts = ps.collect_grid_layout();
        // No grid windows in test environment
        assert!(layouts.is_empty());
    }

    #[test]
    fn test_platform_service_auto_start_query() {
        let ps = create_platform_service();
        // Just verify it doesn't panic; actual value depends on system state
        let _enabled = ps.is_auto_start_enabled();
    }

    /// 平台抽象（PlatformService）是运行时层写开机自启的唯一入口：
    /// 开启后 is_auto_start_enabled 必须为 true，关闭后为 false。
    /// 注册表测试串行化与恢复由 utils::platform::test_support 保证。
    #[cfg(windows)]
    #[test]
    fn test_platform_service_auto_start_toggle_via_trait() {
        use crate::utils::platform::test_support as ts;

        let _guard = ts::registry_test_lock();
        let original = ts::read_run_value();

        let ps = create_platform_service();
        ps.set_auto_start(true).unwrap();
        assert!(
            ps.is_auto_start_enabled(),
            "通过 PlatformService 开启后应查询为启用"
        );
        ps.set_auto_start(false).unwrap();
        assert!(
            !ps.is_auto_start_enabled(),
            "通过 PlatformService 关闭后应查询为禁用"
        );

        ts::restore_run_value(original.as_deref());
    }

    #[test]
    fn test_platform_service_tray_action_none_initially() {
        let ps = create_platform_service();
        // No tray action pending in test environment
        assert!(ps.take_pending_tray_action().is_none());
    }
}
