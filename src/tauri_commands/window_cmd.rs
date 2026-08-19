//! 持久 Tauri WebView 窗口管理。
//!
//! 控制面板、创建框和编辑框在整个进程生命周期内复用同一 WebView。
//! 所有关闭请求都转换为隐藏，后续可由托盘、热键或原生分类框再次打开。

use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

/// 应用管理的持久 WebView 窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedWindow {
    /// 控制面板。
    ControlPanel,
    /// 创建分类框对话框。
    CreateBox,
    /// 编辑分类框对话框。
    EditBox,
    /// AI 分类对话框。
    AiClassify,
}

impl ManagedWindow {
    /// 从 Tauri 标签解析持久窗口。
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "control-panel" => Some(Self::ControlPanel),
            "create-box" => Some(Self::CreateBox),
            "edit-box" => Some(Self::EditBox),
            "ai-classify" => Some(Self::AiClassify),
            _ => None,
        }
    }

    /// 返回 Tauri 配置中的稳定窗口标签。
    pub const fn label(self) -> &'static str {
        match self {
            Self::ControlPanel => "control-panel",
            Self::CreateBox => "create-box",
            Self::EditBox => "edit-box",
            Self::AiClassify => "ai-classify",
        }
    }
}

fn parse_window(label: &str) -> Result<ManagedWindow, String> {
    ManagedWindow::from_label(label).ok_or_else(|| format!("不允许管理未知窗口: '{label}'"))
}

/// 判断标签是否属于必须隐藏复用的持久窗口。
pub fn is_persistent_window_label(label: &str) -> bool {
    ManagedWindow::from_label(label).is_some()
}

/// 显示并聚焦持久窗口。
pub fn show_managed_window(app: &tauri::AppHandle, target: ManagedWindow) -> Result<(), String> {
    let label = target.label();
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| format!("窗口不存在: '{label}'"))?;
    window
        .show()
        .map_err(|error| format!("显示窗口失败: {label}: {error}"))?;
    window
        .set_focus()
        .map_err(|error| format!("聚焦窗口失败: {label}: {error}"))?;
    window
        .emit("window-opened", ())
        .map_err(|error| format!("发送窗口打开事件失败: {label}: {error}"))?;
    Ok(())
}

/// 隐藏持久窗口而不销毁 WebView。
pub fn hide_managed_window(app: &tauri::AppHandle, target: ManagedWindow) -> Result<(), String> {
    let label = target.label();
    app.get_webview_window(label)
        .ok_or_else(|| format!("窗口不存在: '{label}'"))?
        .hide()
        .map_err(|error| format!("隐藏窗口失败: {label}: {error}"))
}

/// 切换持久窗口可见状态。
pub fn toggle_managed_window(app: &tauri::AppHandle, target: ManagedWindow) -> Result<(), String> {
    let label = target.label();
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| format!("窗口不存在: '{label}'"))?;
    match window
        .is_visible()
        .map_err(|error| format!("读取窗口状态失败: {label}: {error}"))?
    {
        true => hide_managed_window(app, target),
        false => show_managed_window(app, target),
    }
}

/// 显示指定持久窗口。
#[tauri::command]
pub async fn show_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    show_managed_window(&app, parse_window(&label)?)
}

/// 隐藏指定持久窗口。
#[tauri::command]
pub async fn hide_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    hide_managed_window(&app, parse_window(&label)?)
}

/// 兼容旧前端命令；持久窗口的 close 语义固定为 hide。
#[tauri::command]
pub async fn close_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    log::warn!("close_window 已转换为 hide_window: {label}");
    hide_managed_window(&app, parse_window(&label)?)
}

/// 切换指定持久窗口可见状态。
#[tauri::command]
pub async fn toggle_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    toggle_managed_window(&app, parse_window(&label)?)
}

/// 查询指定持久窗口是否可见。
#[tauri::command]
pub async fn is_window_visible(app: tauri::AppHandle, label: String) -> Result<bool, String> {
    let target = parse_window(&label)?;
    app.get_webview_window(target.label())
        .ok_or_else(|| format!("窗口不存在: '{}'", target.label()))?
        .is_visible()
        .map_err(|error| format!("读取窗口状态失败: {}: {error}", target.label()))
}

fn validate_external_url(url: &str) -> Result<(), String> {
    let parsed = tauri::Url::parse(url).map_err(|error| format!("URL 无效: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("只允许打开有效的 HTTP/HTTPS 地址".to_string());
    }
    Ok(())
}

/// 使用 Tauri Shell 插件在默认浏览器中打开 HTTP/HTTPS 地址。
#[tauri::command]
pub async fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    validate_external_url(&url)?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| format!("打开 URL 失败: {error}"))
}

/// 检查应用更新。
#[tauri::command]
pub async fn check_update() -> Result<String, String> {
    log::info!("[tauri_cmd] check_update called");
    Ok("No updates available".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_window_labels_round_trip() {
        for window in [
            ManagedWindow::ControlPanel,
            ManagedWindow::CreateBox,
            ManagedWindow::EditBox,
        ] {
            assert_eq!(ManagedWindow::from_label(window.label()), Some(window));
            assert!(is_persistent_window_label(window.label()));
        }
    }

    #[test]
    fn unknown_window_label_is_rejected() {
        assert!(ManagedWindow::from_label("main").is_none());
        assert!(parse_window("main").is_err());
        assert!(!is_persistent_window_label("main"));
    }

    #[test]
    fn external_url_policy_accepts_only_http_and_https() {
        assert!(validate_external_url("https://github.com/chenzhihui4869").is_ok());
        assert!(validate_external_url("http://127.0.0.1:8080/path").is_ok());
        assert!(validate_external_url("file:///C:/Windows/System32/cmd.exe").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
        assert!(validate_external_url("not-a-url").is_err());
    }
}
