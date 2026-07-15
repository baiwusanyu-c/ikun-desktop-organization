//! 插件系统
//!
//! 功能模块作为插件注册到 `PluginRegistry`，支持灵活扩展。
//! 所有插件实现 `Plugin` trait，通过 `AppContext` 与核心交互。

#![allow(dead_code)] // 插件系统为架构预留，当前 GPUI 构建路径尚未接入插件注册

pub mod builtin;

use crate::context::AppContext;

/// 插件菜单项
#[derive(Debug, Clone)]
pub struct PluginMenuItem {
    /// 命令 ID
    pub id: String,
    /// 显示标签
    pub label: String,
    /// 图标（emoji 或路径）
    pub icon: String,
    /// 快捷键（可选）
    pub shortcut: Option<String>,
}

/// 插件 trait — 所有功能模块都作为插件注册
pub trait Plugin: Send {
    /// 插件唯一 ID
    fn id(&self) -> &str;
    /// 插件名称
    fn name(&self) -> &str;
    /// 初始化（在 AppContext 创建后调用）
    fn init(&mut self, ctx: &mut AppContext) -> anyhow::Result<()>;
    /// 插件提供的菜单项（控制面板 & 托盘菜单）
    fn menu_items(&self) -> Vec<PluginMenuItem>;
    /// 处理菜单命令
    fn handle_command(&mut self, cmd: &str, ctx: &mut AppContext) -> anyhow::Result<()>;
    /// 清理
    fn cleanup(&mut self, ctx: &mut AppContext);
}

/// 插件注册表
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册插件
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        log::info!("注册插件: {}", plugin.name());
        self.plugins.push(plugin);
    }

    /// 初始化所有插件
    pub fn init_all(&mut self, ctx: &mut AppContext) {
        for plugin in &mut self.plugins {
            if let Err(e) = plugin.init(ctx) {
                log::error!("插件 '{}' 初始化失败: {}", plugin.name(), e);
            }
        }
    }

    /// 获取所有菜单项
    pub fn all_menu_items(&self) -> Vec<PluginMenuItem> {
        self.plugins.iter().flat_map(|p| p.menu_items()).collect()
    }

    /// 分发命令到对应插件
    pub fn dispatch_command(&mut self, cmd: &str, ctx: &mut AppContext) {
        for plugin in &mut self.plugins {
            if let Err(e) = plugin.handle_command(cmd, ctx) {
                log::error!("插件 '{}' 处理命令 '{}' 失败: {}", plugin.name(), cmd, e);
            }
        }
    }

    /// 清理所有插件
    pub fn cleanup_all(&mut self, ctx: &mut AppContext) {
        for plugin in &mut self.plugins {
            plugin.cleanup(ctx);
        }
    }
}
