//! 桌面管理模块

mod file_icon;
mod grid_window;
mod icon_hider;
mod icons;

#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

pub use file_icon::extract_file_icon;
pub use grid_window::{
    create_grid_window, destroy_all_grid_windows, get_grid_count, GridCreateParams, GridFileItem,
};
pub use icon_hider::{hide_native_icons, restore_native_icons};
pub use icons::DesktopIcon;

#[cfg(windows)]
pub use windows::DesktopManager;

#[cfg(target_os = "macos")]
pub use macos::DesktopManager;

pub trait DesktopManagerTrait {
    fn new() -> anyhow::Result<Self>
    where
        Self: Sized;
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;
    fn watch_desktop(&self) -> anyhow::Result<()>;
}
