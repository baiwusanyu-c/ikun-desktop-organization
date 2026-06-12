//! 内置插件注册
//!
//! 注册所有内置功能插件到 PluginRegistry。

pub mod classification_plugin;
pub mod style_plugin;

use crate::plugin::PluginRegistry;

/// 注册所有内置插件
pub fn register_builtin_plugins(registry: &mut PluginRegistry) {
    registry.register(Box::new(classification_plugin::ClassificationPlugin::new()));
    registry.register(Box::new(style_plugin::StylePlugin::new()));
}
