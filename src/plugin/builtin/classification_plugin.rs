//! 分类功能插件
//!
//! 提供分类相关的菜单项和命令：一键分类、创建分类框等。

use crate::context::AppContext;
use crate::plugin::{Plugin, PluginMenuItem};

/// 分类插件
pub struct ClassificationPlugin;

impl ClassificationPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for ClassificationPlugin {
    fn id(&self) -> &str {
        "builtin.classification"
    }

    fn name(&self) -> &str {
        "分类功能"
    }

    fn init(&mut self, _ctx: &mut AppContext) -> anyhow::Result<()> {
        log::info!("分类插件已初始化");
        Ok(())
    }

    fn menu_items(&self) -> Vec<PluginMenuItem> {
        vec![
            PluginMenuItem {
                id: "classify.auto".into(),
                label: "一键 AI 桌面分类".into(),
                icon: "🤖".into(),
                shortcut: Some("Ctrl+Alt+C".into()),
            },
            PluginMenuItem {
                id: "box.create".into(),
                label: "创建桌面分类框".into(),
                icon: "📦".into(),
                shortcut: None,
            },
        ]
    }

    fn handle_command(&mut self, cmd: &str, ctx: &mut AppContext) -> anyhow::Result<()> {
        match cmd {
            "classify.auto" => {
                log::info!("执行一键分类...");
                ctx.set_status("一键分类功能待实现");
                // TODO: 实现一键 AI 分类
            }
            "box.create" => {
                log::info!("创建分类框...");
                ctx.set_status("创建分类框功能待实现");
                // TODO: 实现创建分类框
            }
            _ => {}
        }
        Ok(())
    }

    fn cleanup(&mut self, _ctx: &mut AppContext) {
        log::info!("分类插件已清理");
    }
}
