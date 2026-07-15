//! 样式设置插件

#![allow(dead_code)] // 插件架构预留
//!
//! 提供样式相关的菜单项和命令：背景色、圆角、图标大小等。

use crate::context::AppContext;
use crate::plugin::{Plugin, PluginMenuItem};

/// 样式插件
pub struct StylePlugin;

impl StylePlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for StylePlugin {
    fn id(&self) -> &str {
        "builtin.style"
    }

    fn name(&self) -> &str {
        "样式设置"
    }

    fn init(&mut self, _ctx: &mut AppContext) -> anyhow::Result<()> {
        log::info!("样式插件已初始化");
        Ok(())
    }

    fn menu_items(&self) -> Vec<PluginMenuItem> {
        vec![
            PluginMenuItem {
                id: "style.background".into(),
                label: "分类框背景色调调节".into(),
                icon: "🎨".into(),
                shortcut: None,
            },
            PluginMenuItem {
                id: "style.radius".into(),
                label: "分类框圆角调节".into(),
                icon: "⭕".into(),
                shortcut: None,
            },
            PluginMenuItem {
                id: "style.icon_size".into(),
                label: "快捷方式图标调节".into(),
                icon: "🖼️".into(),
                shortcut: None,
            },
            PluginMenuItem {
                id: "style.font_size".into(),
                label: "文本尺寸调节".into(),
                icon: "🔤".into(),
                shortcut: None,
            },
        ]
    }

    fn handle_command(&mut self, cmd: &str, ctx: &mut AppContext) -> anyhow::Result<()> {
        match cmd {
            "style.background" => {
                ctx.set_status("背景色调节功能待实现");
                // TODO: 打开背景色设置面板
            }
            "style.radius" => {
                ctx.set_status("圆角调节功能待实现");
                // TODO: 打开圆角设置
            }
            "style.icon_size" => {
                ctx.set_status("图标大小调节功能待实现");
                // TODO: 打开图标大小设置
            }
            "style.font_size" => {
                ctx.set_status("文本尺寸调节功能待实现");
                // TODO: 打开文本尺寸设置
            }
            _ => {}
        }
        Ok(())
    }

    fn cleanup(&mut self, _ctx: &mut AppContext) {
        log::info!("样式插件已清理");
    }
}
