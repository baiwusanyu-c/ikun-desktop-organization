//! 桌面覆盖窗口模块
//!
//! 通过全屏分层窗口覆盖桌面，在原生 SysListView32 被隐藏后由应用自行渲染
//! 桌面图标与处理交互。阶段 2 已接入未管理桌面图标的首帧渲染，后续阶段
//! 会逐步接入命中测试、选择、右键菜单和拖拽交互。

mod overlay_render;
mod overlay_window;
mod overlay_wnd_proc;

pub(crate) use overlay_window::{add_icon_to_overlay, is_desktop_overlay_window};
pub use overlay_window::{create_desktop_overlay, destroy_desktop_overlay};
