//! 持久 Tauri WebView 窗口管理。
//!
//! 控制面板、创建框、编辑框和 AI 分类四个窗口在 `tauri.conf.json` 中
//! 声明为 `"create": false`：尺寸、透明、置顶等属性仍由配置唯一描述，
//! 但启动时不创建，首次需要显示时才用
//! `WebviewWindowBuilder::from_config` 按同一份声明拉起。
//!
//! 为什么不能预创建：每个 WebView2 窗口会拉起一个独立 renderer 进程，
//! 四个隐藏窗口连同共享的 browser / gpu / utility 进程共 9 个，
//! 常驻约 500MB 工作集；而用户绝大多数时间一个面板都不开。
//!
//! 创建后的生命周期与改造前一致：所有关闭请求都转换为隐藏，
//! WebView 不销毁，后续可由托盘、热键或原生分类框再次打开。

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

/// 按 `tauri.conf.json` 中的声明创建持久窗口。
///
/// 直接复用配置对象，不在代码里再抄一份尺寸/透明/置顶参数：
/// 两边不一致时窗口表现会悄悄变样，而且很难被发现。
fn build_managed_window(
    app: &tauri::AppHandle,
    target: ManagedWindow,
) -> Result<tauri::WebviewWindow, String> {
    let label = target.label();
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == label)
        .ok_or_else(|| format!("tauri.conf.json 缺少窗口声明: '{label}'"))?;
    log::info!("按需创建持久 WebView 窗口: {label}");
    tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|error| format!("构造窗口失败: {label}: {error}"))?
        .build()
        .map_err(|error| format!("创建窗口失败: {label}: {error}"))
}

/// 取得持久窗口；尚未创建时按配置声明拉起一个。
fn ensure_managed_window(
    app: &tauri::AppHandle,
    target: ManagedWindow,
) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(target.label()) {
        return Ok(window);
    }
    build_managed_window(app, target)
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
    let window = ensure_managed_window(app, target)?;
    // 默认位置：主屏幕水平垂直居中（每次显示前重置，保证弹窗居中契约）
    if let Err(error) = center_on_primary_monitor(app, &window) {
        log::warn!("窗口主屏幕居中失败: {label}: {error}");
    }
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

/// 将窗口在主屏幕内水平垂直居中（物理像素坐标）。
///
/// 所有 Tauri 绘制的弹窗默认位置为主屏幕正中：
/// 使用主屏幕 position + size 与窗口 outer_size 计算物理坐标，
/// 不依赖 OS 的默认级联摆放，多显示器环境下也固定在主屏幕。
fn center_on_primary_monitor(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
) -> Result<(), String> {
    let monitor = app
        .primary_monitor()
        .map_err(|error| format!("获取主屏幕失败: {error}"))?
        .ok_or_else(|| "主屏幕不存在".to_string())?;
    let monitor_pos = monitor.position();
    let monitor_size = monitor.size();
    let window_size = window
        .outer_size()
        .map_err(|error| format!("读取窗口尺寸失败: {error}"))?;
    let x = monitor_pos.x + (monitor_size.width as i32 - window_size.width as i32) / 2;
    let y = monitor_pos.y + (monitor_size.height as i32 - window_size.height as i32) / 2;
    window
        .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
            x, y,
        )))
        .map_err(|error| format!("设置窗口位置失败: {error}"))
}

/// 隐藏持久窗口而不销毁 WebView。
pub fn hide_managed_window(app: &tauri::AppHandle, target: ManagedWindow) -> Result<(), String> {
    let label = target.label();
    // 窗口还没被创建过 = 已经处于隐藏态，
    // 不能为了隐藏反而拉起一个 WebView2 renderer 进程。
    let Some(window) = app.get_webview_window(label) else {
        return Ok(());
    };
    window
        .hide()
        .map_err(|error| format!("隐藏窗口失败: {label}: {error}"))
}

/// 切换持久窗口可见状态。
pub fn toggle_managed_window(app: &tauri::AppHandle, target: ManagedWindow) -> Result<(), String> {
    // 尚未创建等价于不可见，直接走显示分支（内部会按需创建）。
    let Some(window) = app.get_webview_window(target.label()) else {
        return show_managed_window(app, target);
    };
    match window
        .is_visible()
        .map_err(|error| format!("读取窗口状态失败: {}: {error}", target.label()))?
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
    // 尚未创建等价于不可见；不能为了回答一个 bool 而拉起 WebView2。
    let Some(window) = app.get_webview_window(target.label()) else {
        return Ok(false);
    };
    window
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
