//! 托盘菜单逻辑
//!
//! 定义系统托盘的右键菜单项及其行为。
//! TODO: 实际托盘窗口由 platform/windows/sys_tray.rs 实现。

use crate::context::AppContext;

/// 托盘菜单动作
#[derive(Debug, Clone)]
pub enum TrayAction {
    /// 显示/隐藏控制面板
    TogglePanel,
    /// 创建分类框
    CreateBox,
    /// 一键 AI 分类
    AutoClassify,
    /// 打开设置
    OpenSettings,
    /// 退出
    Quit,
}

/// 获取托盘菜单项列表
pub fn tray_menu_items() -> Vec<(String, TrayAction)> {
    vec![
        ("📦 显示控制面板".into(), TrayAction::TogglePanel),
        ("➕ 创建桌面分类框".into(), TrayAction::CreateBox),
        ("🤖 一键 AI 桌面分类".into(), TrayAction::AutoClassify),
        ("⚙️ 设置".into(), TrayAction::OpenSettings),
        ("❌ 退出桌面分类".into(), TrayAction::Quit),
    ]
}

/// 处理托盘菜单动作
pub fn handle_tray_action(action: &TrayAction, ctx: &mut AppContext) {
    match action {
        TrayAction::TogglePanel => {
            ctx.state.panel_visible = !ctx.state.panel_visible;
            log::info!("控制面板可见性: {}", ctx.state.panel_visible);
        }
        TrayAction::CreateBox => {
            ctx.set_status("创建分类框功能待实现");
            // TODO: 创建新分类框
        }
        TrayAction::AutoClassify => {
            ctx.set_status("一键 AI 分类功能待实现");
            // TODO: 执行 AI 分类
        }
        TrayAction::OpenSettings => {
            ctx.state.panel_visible = true;
            // TODO: 切换到设置面板
        }
        TrayAction::Quit => {
            log::info!("用户请求退出");
            // TODO: 执行清理并退出
        }
    }
}
