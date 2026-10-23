//! 平台层（Windows-only 产品）
//!
//! 历史版本通过 `DesktopManagerTrait` / `PlatformService` trait 抽象为未来
//! 跨平台预留接口；经评估产品不会移植 macOS，该抽象只剩下成本没有收益，
//! 已移除。现在所有平台实现都收敛在 `windows` 子模块，本模块仅保留薄
//! re-export 与托盘动作枚举。

pub mod windows;

pub use windows::*;

/// System tray menu actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemTrayAction {
    OpenControlPanel,
    CreateBox,
    AiClassify,
    ToggleAutoStart,
    Quit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_manager_enumerates_without_panic() {
        // 测试环境允许返回空列表，但不得 panic
        let dm = DesktopManager;
        let _icons = dm.get_desktop_icons();
    }

    #[test]
    fn grid_count_and_layout_empty_before_windows_created() {
        assert_eq!(get_grid_count(), 0);
        assert!(collect_grid_layout().is_empty());
    }

    #[test]
    fn tray_action_none_initially() {
        assert!(take_pending_tray_action().is_none());
    }

    #[test]
    fn screen_resolution_positive() {
        let (w, h) = get_screen_resolution();
        assert!(w > 0 && h > 0, "screen resolution should be positive");
    }

    #[test]
    fn primary_monitor_dpi_in_valid_range() {
        let dpi = get_primary_monitor_dpi();
        assert!((96..=600).contains(&dpi), "DPI out of range: {dpi}");
    }
}
