//! 平台抽象层
//!
//! 提供跨平台的桌面管理接口，通过 trait 统一不同平台的实现。
//!
//! # 模块结构
//! - `types`: 跨平台数据类型（DesktopIcon, IconInfo 等）
//! - `windows`: Windows 平台实现（Shell API, D2D 渲染等）
//! - `macos`: macOS 平台实现（实验性）

pub mod types;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

// Re-export cross-platform types
pub use types::*;

// Re-export platform-specific DesktopManager
#[cfg(target_os = "macos")]
pub use macos::DesktopManager;
#[cfg(windows)]
pub use windows::DesktopManager;

// Re-export platform-specific functions that app.rs uses
#[cfg(windows)]
pub use windows::{
    collect_grid_layout, create_grid_window, destroy_all_grid_windows, extract_file_icon,
    get_grid_count, get_screen_resolution, hide_native_icons, restore_native_icons,
    GridCreateParams, GridFileItem,
};

/// 桌面管理器 trait（跨平台）
pub trait DesktopManagerTrait {
    fn new() -> anyhow::Result<Self>
    where
        Self: Sized;
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;
    fn watch_desktop(&self) -> anyhow::Result<()>;
}
