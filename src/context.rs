//! Application context - central state management object
//!
//! All modules share state and configuration via AppContext.
//! This is the core dependency injection point.
//!
//! ## Startup flow
//! 1. AppConfig::load() reads ~/.ikun_organizer.json
//! 2. AppContext::new() injects config and initializes global style config
//! 3. Modules read needed config from ctx.config

use crate::config::AppConfig;

/// Application context (core dependency injection object)
///
/// 持有统一配置（JSON，可导入导出）与加载结果。平台能力通过
/// `crate::platform`（Windows-only）直接调用，不再经由 trait 对象。
#[derive(Default)]
pub struct AppContext {
    /// Unified config (JSON, importable/exportable)
    pub config: AppConfig,
    /// 配置文件是否成功加载。
    ///
    /// 为 `false` 时 `config` 是回退的默认值（auto_start=false 等），
    /// 任何"按配置同步系统状态"的逻辑（如开机自启注册表同步）都必须跳过，
    /// 否则会用默认值覆盖用户已开启的系统配置条目。
    pub config_loaded: bool,
}

impl AppContext {
    /// Create application context (load config + initialize subsystems)
    ///
    /// Reads config from ~/.ikun_organizer.json, falls back to defaults.
    /// Config load failure degrades gracefully, does not block app startup;
    /// `config_loaded` records the outcome so callers can avoid syncing
    /// system state (e.g. auto-start registry) from fallback defaults.
    pub fn new() -> anyhow::Result<Self> {
        // 1. Load config
        let mut config_loaded = true;
        let config = AppConfig::load().unwrap_or_else(|e| {
            config_loaded = false;
            log::error!(
                "配置加载失败，回退默认配置（不会按默认值同步系统状态）: {}",
                e
            );
            AppConfig::default()
        });

        log::info!("Config version: {}", config.version);
        log::info!("Log level: {}", config.debug.log_level);
        log::info!(
            "Restore layout on start: {}",
            config.general.restore_layout_on_start
        );

        // 初始化平台层样式缓存（引擎启动与参数应用发生在 platform_runtime::initialize，
        // 这里只保证渲染线程在引擎就绪前也能读到一致的分类框样式）。
        #[cfg(target_os = "windows")]
        crate::platform::windows::glass_engine::set_box_style_cache(&config.box_style);

        Ok(Self {
            config,
            config_loaded,
        })
    }

    /// Save config to ~/.ikun_organizer.json
    pub fn save_config(&mut self) -> anyhow::Result<()> {
        self.config.last_modified = chrono::Utc::now();
        self.config.save()
    }

    /// Export config to specified path
    pub fn export_config(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.config.export_to(path)
    }

    /// Import config from specified path and save
    pub fn import_config(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        self.config = AppConfig::import_from(path)?;
        self.save_config()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_context_marks_config_unloaded() {
        let ctx = AppContext::default();
        assert!(!ctx.config_loaded);
    }

    #[test]
    fn new_context_loads_or_degrades_gracefully() {
        // 无论配置文件是否存在，new() 都不应 panic；失败时回退默认值并置 config_loaded=false
        let ctx = AppContext::new().expect("AppContext::new must not fail on read error");
        let _ = ctx.config.version.as_str();
    }
}
