//! 进程内插件系统。
//!
//! `PluginRegistry` 管理编译进应用的 trait 插件，支持运行时安装、卸载、
//! 命令分发和逆序清理。它不是动态库加载器，不承诺跨版本 ABI 兼容。

pub mod builtin;

use std::collections::HashSet;

use anyhow::{anyhow, Context};

use crate::context::AppContext;

/// 插件菜单项。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginMenuItem {
    /// 命令 ID。
    pub id: String,
    /// 显示标签。
    pub label: String,
    /// 图标名称或资源路径。
    pub icon: String,
    /// 快捷键。
    pub shortcut: Option<String>,
}

/// 单个插件初始化失败信息。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginInitFailure {
    /// 插件 ID。
    pub plugin_id: String,
    /// 错误文本。
    pub error: String,
}

/// 批量初始化结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginInitReport {
    /// 成功进入已初始化状态的插件 ID。
    pub initialized: Vec<String>,
    /// 初始化失败但仍保留在注册表中的插件。
    pub failed: Vec<PluginInitFailure>,
}

/// 所有进程内功能插件必须实现的生命周期契约。
pub trait Plugin: Send {
    /// 插件唯一 ID。
    fn id(&self) -> &str;

    /// 插件显示名称。
    fn name(&self) -> &str;

    /// 初始化插件。
    fn init(&mut self, ctx: &mut AppContext) -> anyhow::Result<()>;

    /// 返回插件提供的菜单项。
    fn menu_items(&self) -> Vec<PluginMenuItem>;

    /// 判断插件是否处理给定命令。
    fn handles_command(&self, command: &str) -> bool {
        self.menu_items().iter().any(|item| item.id == command)
    }

    /// 处理插件命令。
    fn handle_command(&mut self, command: &str, ctx: &mut AppContext) -> anyhow::Result<()>;

    /// 清理插件资源。
    fn cleanup(&mut self, ctx: &mut AppContext);
}

struct PluginEntry {
    plugin: Box<dyn Plugin>,
    initialized: bool,
}

/// 插件注册表。
///
/// 注册表保持确定的安装顺序；清理时使用逆序，便于插件按依赖栈释放资源。
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<PluginEntry>,
}

impl PluginRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    fn validate_candidate(&self, plugin: &dyn Plugin) -> anyhow::Result<()> {
        let plugin_id = plugin.id().trim();
        if plugin_id.is_empty() {
            return Err(anyhow!("插件 ID 不能为空"));
        }
        if self
            .plugins
            .iter()
            .any(|entry| entry.plugin.id() == plugin_id)
        {
            return Err(anyhow!("插件 ID 重复: {plugin_id}"));
        }

        let mut candidate_commands = HashSet::new();
        for item in plugin.menu_items() {
            let command_id = item.id.trim();
            if command_id.is_empty() {
                return Err(anyhow!("插件 '{plugin_id}' 包含空命令 ID"));
            }
            if !candidate_commands.insert(command_id.to_string()) {
                return Err(anyhow!("插件 '{plugin_id}' 的命令 ID 重复: {command_id}"));
            }
            if self.plugins.iter().any(|entry| {
                entry
                    .plugin
                    .menu_items()
                    .iter()
                    .any(|existing| existing.id == command_id)
            }) {
                return Err(anyhow!("插件命令 ID 冲突: {command_id}"));
            }
        }
        Ok(())
    }

    /// 注册插件，但不立即初始化。
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> anyhow::Result<()> {
        self.validate_candidate(plugin.as_ref())?;
        log::info!("注册插件: {} ({})", plugin.name(), plugin.id());
        self.plugins.push(PluginEntry {
            plugin,
            initialized: false,
        });
        Ok(())
    }

    /// 在运行期安装并初始化插件。
    ///
    /// 初始化失败时不会把插件加入注册表，避免半初始化状态泄漏。
    #[allow(dead_code)] // 供后续运行时宿主扩展调用，内置插件走批量启动路径。
    pub fn install(
        &mut self,
        mut plugin: Box<dyn Plugin>,
        ctx: &mut AppContext,
    ) -> anyhow::Result<()> {
        self.validate_candidate(plugin.as_ref())?;
        let plugin_id = plugin.id().to_string();
        plugin
            .init(ctx)
            .with_context(|| format!("初始化插件失败: {plugin_id}"))?;
        log::info!("运行时安装插件: {} ({plugin_id})", plugin.name());
        self.plugins.push(PluginEntry {
            plugin,
            initialized: true,
        });
        Ok(())
    }

    /// 初始化所有尚未初始化的已注册插件。
    ///
    /// 单个插件失败不会阻止其他插件启动，失败项留在注册表中以便后续重试或卸载。
    pub fn init_all(&mut self, ctx: &mut AppContext) -> PluginInitReport {
        let mut report = PluginInitReport::default();
        for entry in &mut self.plugins {
            if entry.initialized {
                continue;
            }
            let plugin_id = entry.plugin.id().to_string();
            match entry.plugin.init(ctx) {
                Ok(()) => {
                    entry.initialized = true;
                    report.initialized.push(plugin_id);
                }
                Err(error) => {
                    log::error!("插件 '{plugin_id}' 初始化失败: {error:#}");
                    report.failed.push(PluginInitFailure {
                        plugin_id,
                        error: format!("{error:#}"),
                    });
                }
            }
        }
        report
    }

    /// 卸载指定插件，并在移除前清理已初始化资源。
    #[allow(dead_code)] // 与 install 配对的宿主扩展 API。
    pub fn uninstall(&mut self, plugin_id: &str, ctx: &mut AppContext) -> bool {
        let Some(index) = self
            .plugins
            .iter()
            .position(|entry| entry.plugin.id() == plugin_id)
        else {
            return false;
        };
        let mut entry = self.plugins.remove(index);
        if entry.initialized {
            entry.plugin.cleanup(ctx);
        }
        log::info!("卸载插件: {} ({plugin_id})", entry.plugin.name());
        true
    }

    /// 判断插件是否已注册。
    #[cfg(test)]
    pub fn contains(&self, plugin_id: &str) -> bool {
        self.plugins
            .iter()
            .any(|entry| entry.plugin.id() == plugin_id)
    }

    /// 判断插件是否已初始化。
    #[cfg(test)]
    pub fn is_initialized(&self, plugin_id: &str) -> bool {
        self.plugins
            .iter()
            .any(|entry| entry.plugin.id() == plugin_id && entry.initialized)
    }

    /// 获取所有已初始化插件的菜单项。
    pub fn all_menu_items(&self) -> Vec<PluginMenuItem> {
        self.plugins
            .iter()
            .filter(|entry| entry.initialized)
            .flat_map(|entry| entry.plugin.menu_items())
            .collect()
    }

    /// 将命令定向分发给声明处理该命令的已初始化插件。
    ///
    /// 返回 `Ok(false)` 表示没有插件声明该命令。
    pub fn dispatch_command(
        &mut self,
        command: &str,
        ctx: &mut AppContext,
    ) -> anyhow::Result<bool> {
        let Some(entry) = self
            .plugins
            .iter_mut()
            .find(|entry| entry.initialized && entry.plugin.handles_command(command))
        else {
            return Ok(false);
        };
        entry
            .plugin
            .handle_command(command, ctx)
            .with_context(|| format!("插件 '{}' 处理命令 '{command}' 失败", entry.plugin.id()))?;
        Ok(true)
    }

    /// 逆序清理所有已初始化插件。
    pub fn cleanup_all(&mut self, ctx: &mut AppContext) {
        for entry in self.plugins.iter_mut().rev() {
            if entry.initialized {
                entry.plugin.cleanup(ctx);
                entry.initialized = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    struct TestPlugin {
        id: &'static str,
        command: &'static str,
        fail_init: bool,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl TestPlugin {
        fn new(
            id: &'static str,
            command: &'static str,
            fail_init: bool,
            events: Arc<Mutex<Vec<String>>>,
        ) -> Self {
            Self {
                id,
                command,
                fail_init,
                events,
            }
        }

        fn record(&self, event: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("{}:{event}", self.id));
        }
    }

    impl Plugin for TestPlugin {
        fn id(&self) -> &str {
            self.id
        }

        fn name(&self) -> &str {
            self.id
        }

        fn init(&mut self, _ctx: &mut AppContext) -> anyhow::Result<()> {
            self.record("init");
            if self.fail_init {
                return Err(anyhow!("test init failure"));
            }
            Ok(())
        }

        fn menu_items(&self) -> Vec<PluginMenuItem> {
            vec![PluginMenuItem {
                id: self.command.to_string(),
                label: self.command.to_string(),
                ..PluginMenuItem::default()
            }]
        }

        fn handle_command(&mut self, command: &str, ctx: &mut AppContext) -> anyhow::Result<()> {
            self.record(command);
            ctx.set_status(command);
            Ok(())
        }

        fn cleanup(&mut self, _ctx: &mut AppContext) {
            self.record("cleanup");
        }
    }

    #[test]
    fn duplicate_plugin_and_command_ids_are_rejected() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(TestPlugin::new(
                "plugin.one",
                "command.one",
                false,
                events.clone(),
            )))
            .unwrap();

        let duplicate_plugin = registry.register(Box::new(TestPlugin::new(
            "plugin.one",
            "command.two",
            false,
            events.clone(),
        )));
        let duplicate_command = registry.register(Box::new(TestPlugin::new(
            "plugin.two",
            "command.one",
            false,
            events,
        )));

        assert!(duplicate_plugin.is_err());
        assert!(duplicate_command.is_err());
        assert!(registry.contains("plugin.one"));
        assert!(!registry.contains("plugin.two"));
    }

    #[test]
    fn runtime_install_dispatch_and_uninstall_follow_lifecycle() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut registry = PluginRegistry::new();
        let mut ctx = AppContext::default();

        registry
            .install(
                Box::new(TestPlugin::new(
                    "plugin.live",
                    "command.live",
                    false,
                    events.clone(),
                )),
                &mut ctx,
            )
            .unwrap();

        assert!(registry.is_initialized("plugin.live"));
        assert!(registry.dispatch_command("command.live", &mut ctx).unwrap());
        assert!(!registry
            .dispatch_command("command.unknown", &mut ctx)
            .unwrap());
        assert_eq!(ctx.state.status_message, "command.live");
        assert!(registry.uninstall("plugin.live", &mut ctx));
        assert!(!registry.contains("plugin.live"));
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                "plugin.live:init",
                "plugin.live:command.live",
                "plugin.live:cleanup"
            ]
        );
    }

    #[test]
    fn failed_runtime_install_is_atomic() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut registry = PluginRegistry::new();
        let mut ctx = AppContext::default();

        let result = registry.install(
            Box::new(TestPlugin::new(
                "plugin.failed",
                "command.failed",
                true,
                events,
            )),
            &mut ctx,
        );

        assert!(result.is_err());
        assert!(!registry.contains("plugin.failed"));
    }

    #[test]
    fn cleanup_runs_in_reverse_initialization_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut registry = PluginRegistry::new();
        let mut ctx = AppContext::default();
        registry
            .register(Box::new(TestPlugin::new(
                "plugin.first",
                "command.first",
                false,
                events.clone(),
            )))
            .unwrap();
        registry
            .register(Box::new(TestPlugin::new(
                "plugin.second",
                "command.second",
                false,
                events.clone(),
            )))
            .unwrap();

        let report = registry.init_all(&mut ctx);
        assert_eq!(report.initialized, vec!["plugin.first", "plugin.second"]);
        assert!(report.failed.is_empty());
        registry.cleanup_all(&mut ctx);

        assert_eq!(
            *events.lock().unwrap(),
            vec![
                "plugin.first:init",
                "plugin.second:init",
                "plugin.second:cleanup",
                "plugin.first:cleanup"
            ]
        );
    }
}
