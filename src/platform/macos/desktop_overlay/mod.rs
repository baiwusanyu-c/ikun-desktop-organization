//! macOS 桌面覆盖窗口模块
//!
//! 通过全屏透明 NSWindow 覆盖桌面，在 Finder 桌面图标被隐藏后由应用自行渲染
//! 桌面图标与处理交互。阶段 1 实现基础窗口创建与生命周期管理。

mod overlay_render;
mod overlay_window;

pub use overlay_window::{create_desktop_overlay, destroy_desktop_overlay, has_desktop_overlay};
