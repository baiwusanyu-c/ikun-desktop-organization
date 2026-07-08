//! 桌面覆盖窗口模块
//!
//! 通过全屏分层窗口覆盖桌面，在原生 SysListView32 被隐藏后由应用自行渲染
//! 桌面图标与处理交互。当前已接入未管理桌面图标渲染、命中测试、选择、
//! 右键菜单、拖拽交互和桌面文件监听刷新。

mod desktop_watcher;
mod overlay_render;
mod overlay_window;
mod overlay_wnd_proc;

pub(crate) use overlay_window::{add_icon_to_overlay, is_desktop_overlay_window};
pub use overlay_window::{create_desktop_overlay, destroy_desktop_overlay};
