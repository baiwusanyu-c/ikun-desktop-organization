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

/// 将配置推送到原生渲染器。
///
/// 不在本线程直接调用 `rerender_all_grid_windows`：分类框窗口属于平台渲染
/// 线程，跨线程同步调用会阻塞 Tauri 事件循环，曾经导致 WebView2 窗口
/// 底部出现异常空白。所有刷新都通过 `PostMessageW` 交给窗口自己的消息循环。
///
/// 液态玻璃开关变化时必须走 `sync_all_grid_windows_to_glass_state`：开关
/// 同时决定分类框外投影留白与 200ms 周期重绘定时器是否装配，
/// 只改配置不重建外框会让玻璃关闭后仍按放大 3 倍的面积分配每帧 DIB。
pub fn apply_runtime_config(config: &AppConfig) {
    #[cfg(windows)]
    {
        use crate::platform::windows::{glass_engine, grid_window};

        // 样式缓存先行（内容层渲染读它），再确保引擎存活（首次调用即启动；
        // 此前启动失败时每次配置应用都会重试一次），最后应用玻璃参数。
        glass_engine::set_box_style_cache(&config.box_style);
        glass_engine::ensure_engine(config);
        glass_engine::apply_settings(config);
        // 引擎路径下内容层不画底板，但命中捕获带宽度随 grip 配置变化，
        // 任何样式改动都必须显式重绘一帧；外投影留白恒为 0，无需重建窗口外框。
        grid_window::request_all_grid_windows_redraw();
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
        // 透明模式圆角继续联动分类框整体圆角（液态模式圆角独立可调）
        app_config.box_style.liquid_glass.transparent.corner_radius =
            app_config.box_style.border_radius;
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
