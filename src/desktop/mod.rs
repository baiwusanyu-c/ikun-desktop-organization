//! 桌面管理模块
//!
//! 负责与平台桌面系统集成，读取和操控桌面图标

mod icons;
mod icon_mover;
pub mod overlay;

// 平台特定实现
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

pub use icons::{DesktopIcon, IconInfo};
pub use icon_mover::{DesktopIconInfo, DesktopIconMover};
pub use overlay::{BoxDrawData, SharedBoxData, create_shared_box_data, start_overlay};

// 平台特定的导出
#[cfg(windows)]
pub use windows::DesktopManager;

#[cfg(target_os = "macos")]
pub use macos::DesktopManager;

/// 桌面管理器接口（跨平台）
///
/// 不同平台的具体实现在各自的模块中
pub trait DesktopManagerTrait {
    /// 初始化桌面管理器
    fn new() -> anyhow::Result<Self>
    where
        Self: Sized;

    /// 获取桌面图标列表
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;

    /// 监听桌面变化
    fn watch_desktop(&self) -> anyhow::Result<()>;
}
