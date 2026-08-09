//! Tauri 配置命令。
//!
//! Rust 的 `AppConfig` 是前后端唯一配置协议。所有命令通过 Tauri 托管的
//! `SharedTauriRuntime` 修改同一个 `AppContext`，持久化后再同步原生渲染器。

use tauri::State;

use crate::config::{AiConfig, AppConfig, BoxStyleConfig, LiquidGlassConfig};
use crate::tauri_commands::platform_runtime::{SharedTauriRuntime, TauriRuntime};

fn lock_error(error: impl std::fmt::Display) -> String {
    format!("运行时状态不可用: {error}")
}

fn current_config(runtime: &SharedTauriRuntime) -> Result<AppConfig, String> {
    runtime
        .lock()
        .map(|runtime| runtime.ctx.config.clone())
        .map_err(lock_error)
}

fn persist_config(runtime: &SharedTauriRuntime, config: AppConfig) -> Result<AppConfig, String> {
    let mut runtime = runtime.lock().map_err(lock_error)?;
    persist_locked(&mut runtime, config)
}

fn persist_locked(runtime: &mut TauriRuntime, mut config: AppConfig) -> Result<AppConfig, String> {
    config.box_style.normalize();
    config.last_modified = chrono::Utc::now();
    config
        .save()
        .map_err(|error| format!("保存配置失败: {error}"))?;
    apply_runtime_config(&config);
    runtime.replace_config(config.clone());
    Ok(config)
}

fn update_config(
    runtime: &SharedTauriRuntime,
    update: impl FnOnce(&mut AppConfig),
) -> Result<AppConfig, String> {
    let mut runtime = runtime.lock().map_err(lock_error)?;
    let mut config = runtime.ctx.config.clone();
    update(&mut config);
    persist_locked(&mut runtime, config)
}

/// 将配置推送到原生渲染器，并重绘所有现有分类框。
pub fn apply_runtime_config(config: &AppConfig) {
    #[cfg(windows)]
    {
        crate::platform::windows::liquid_glass::update_box_style_config(&config.box_style);
        crate::platform::rerender_all_grid_windows();
    }
}

/// 获取运行时中的完整配置。
#[tauri::command]
pub fn get_config(runtime: State<'_, SharedTauriRuntime>) -> Result<AppConfig, String> {
    current_config(runtime.inner())
}

/// 从磁盘重新加载配置并替换当前运行时状态。
#[tauri::command]
pub fn load_config(runtime: State<'_, SharedTauriRuntime>) -> Result<AppConfig, String> {
    let mut config = AppConfig::load().map_err(|error| format!("加载配置失败: {error}"))?;
    config.box_style.normalize();
    apply_runtime_config(&config);
    runtime
        .lock()
        .map_err(lock_error)?
        .replace_config(config.clone());
    Ok(config)
}

/// 保存完整配置。
#[tauri::command]
pub fn save_config(
    runtime: State<'_, SharedTauriRuntime>,
    config: AppConfig,
) -> Result<AppConfig, String> {
    persist_config(runtime.inner(), config)
}

/// 更新所有窗口使用的主题色。
#[tauri::command]
pub fn update_theme_color(
    runtime: State<'_, SharedTauriRuntime>,
    color: String,
) -> Result<AppConfig, String> {
    update_config(runtime.inner(), |config| {
        config.ui_theme.light.primary = Some(color);
    })
}

/// 更新分类框样式并实时重绘。
#[tauri::command]
pub fn update_box_style(
    runtime: State<'_, SharedTauriRuntime>,
    mut style: BoxStyleConfig,
) -> Result<AppConfig, String> {
    style.normalize();
    update_config(runtime.inner(), |config| config.box_style = style)
}

/// 更新液态玻璃参数并实时重绘。
#[tauri::command]
pub fn update_liquid_glass(
    runtime: State<'_, SharedTauriRuntime>,
    mut config: LiquidGlassConfig,
) -> Result<AppConfig, String> {
    config.normalize();
    update_config(runtime.inner(), |app_config| {
        app_config.box_style.liquid_glass = config;
        app_config.box_style.liquid_glass.corner_radius = app_config.box_style.border_radius;
    })
}

/// 更新 AI 配置。
#[tauri::command]
pub fn update_ai_config(
    runtime: State<'_, SharedTauriRuntime>,
    config: AiConfig,
) -> Result<AppConfig, String> {
    update_config(runtime.inner(), |app_config| app_config.ai = config)
}

/// 恢复默认配置。
#[tauri::command]
pub fn reset_config(runtime: State<'_, SharedTauriRuntime>) -> Result<AppConfig, String> {
    persist_config(runtime.inner(), AppConfig::default())
}

/// 导出配置。
#[tauri::command]
pub fn export_config(runtime: State<'_, SharedTauriRuntime>, path: String) -> Result<(), String> {
    current_config(runtime.inner())?
        .export_to(std::path::Path::new(&path))
        .map_err(|error| format!("导出配置失败: {error}"))
}

/// 导入配置。
#[tauri::command]
pub fn import_config(
    runtime: State<'_, SharedTauriRuntime>,
    path: String,
) -> Result<AppConfig, String> {
    let config = AppConfig::import_from(std::path::Path::new(&path))
        .map_err(|error| format!("导入配置失败: {error}"))?;
    persist_config(runtime.inner(), config)
}
