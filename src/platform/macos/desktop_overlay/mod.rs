//! macOS 桌面覆盖窗口模块
//!
//! 管理覆盖窗口的创建、渲染和交互

pub mod box_render;
pub mod icon_layout;
pub mod overlay_render;
pub mod overlay_window;

pub use overlay_window::{create_desktop_overlay, destroy_desktop_overlay, has_desktop_overlay};
