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

/// ===== WebView 渲染进程挂死看门狗（2026-11-03 验收反馈 bug6）=====
///
/// 现象：面板打开时执行引擎重建/高负载滚动后，WebView2 整页白屏、输入被吞，
/// 但原生侧一切正常（信号开关窗、托盘都活着）——挂死在浏览器/渲染进程一侧，
/// `--disable-gpu` 类浏览器参数实测无效，前端 JS（含 rAF 看门狗）也随 IPC
/// 管道一起死，页面内自愈不可能。唯一确定性恢复 = 销毁并重建该 WebView 窗口。
///
/// 判定：前端每 500ms 发 `panel-heartbeat`；看门狗（poll 线程 100ms 一拍）
/// 只在「窗口可见、未最小化、且自身仍是屏幕命中窗（未被遮挡）」时检查心跳
/// 新鲜度——遮挡/最小化时 Chromium 合法节流定时器，不能误判为挂死。
/// 心跳断流 >8s 即重建窗口（同 label 重建之间至少间隔 30s 防抖，全程 warn 留痕）。
mod hang_watchdog {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    lazy_static::lazy_static! {
        /// label → 最近一次心跳（ms）
        pub static ref HEARTBEAT_MS: Mutex<HashMap<String, u64>> = Mutex::new(HashMap::new());
        /// label → 首次观察到可见的时间（ms，页面加载宽限）
        pub static ref VISIBLE_SINCE_MS: Mutex<HashMap<String, u64>> = Mutex::new(HashMap::new());
        /// label → 最近一次重建时间（ms，防抖）
        pub static ref LAST_RECREATE_MS: Mutex<HashMap<String, u64>> = Mutex::new(HashMap::new());
        /// label → 最近一次心跳携带的 rAF 帧计数（计数冻结 = 呈现停滞）
        pub static ref LAST_FRAMES: Mutex<HashMap<String, u64>> = Mutex::new(HashMap::new());
        /// label → 恢复阶段。0=IDLE，1=已投递浏览器侧 Reload（等复活宽限），
        /// 2=已投递主线程销毁、等 label 释放后由 poll 线程原位重建。
        pub static ref RECOVERY_STAGE: Mutex<HashMap<String, u8>> = Mutex::new(HashMap::new());
        /// stage=2 暂存的几何（物理 x,y,w,h），label 释放后原位重建用
        pub static ref PENDING_GEOMETRY: Mutex<HashMap<String, (i32, i32, u32, u32)>> =
            Mutex::new(HashMap::new());
    }

    pub fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// 记录前端心跳（main.rs 里 `listen("panel-heartbeat")` 调用）。
///
/// 只有 rAF 帧计数发生变化才刷新时间戳：`setInterval` 心跳在呈现停滞后
/// 仍会按时发送，但 `requestAnimationFrame` 依赖合成帧——帧计数冻结即
/// 「窗口活着但画面已死」，与渲染进程整体挂死同罪。
pub fn record_heartbeat(label: &str, frames: u64) {
    let progressed = match hang_watchdog::LAST_FRAMES.lock() {
        Ok(mut map) => {
            let last = map.insert(label.to_string(), frames);
            last.map_or(true, |prev| frames != prev)
        }
        Err(_) => true,
    };
    if progressed {
        if let Ok(mut map) = hang_watchdog::HEARTBEAT_MS.lock() {
            map.insert(label.to_string(), hang_watchdog::now_ms());
        }
    }
}

/// 看门狗主入口：poll 线程每拍调用。
pub fn watch_webview_hangs(app: &tauri::AppHandle) {
    use tauri::Manager;
    for target in [
        ManagedWindow::ControlPanel,
        ManagedWindow::CreateBox,
        ManagedWindow::EditBox,
        ManagedWindow::AiClassify,
    ] {
        let label = target.label();
        let Some(window) = app.get_webview_window(label) else {
            // 窗口未创建/已销毁：清掉基线，下次出现重新计时
            if let Ok(mut map) = hang_watchdog::VISIBLE_SINCE_MS.lock() {
                map.remove(label);
            }
            continue;
        };
        let Ok(visible) = window.is_visible() else {
            continue;
        };
        if !visible {
            // 隐藏期间心跳必然断流（Chromium 节流），基线全部清空，
            // 下次显示重新起 8s 宽限，避免「刚打开就被误杀重建」。
            if let Ok(mut map) = hang_watchdog::VISIBLE_SINCE_MS.lock() {
                map.remove(label);
            }
            if let Ok(mut map) = hang_watchdog::HEARTBEAT_MS.lock() {
                map.remove(label);
            }
            if let Ok(mut map) = hang_watchdog::LAST_FRAMES.lock() {
                map.remove(label);
            }
            continue;
        }
        if window.is_minimized().unwrap_or(false) {
            continue;
        }
        let now = hang_watchdog::now_ms();
        let mut fresh = false;
        if let Ok(map) = hang_watchdog::HEARTBEAT_MS.lock() {
            if let Some(ms) = map.get(label) {
                fresh = now.saturating_sub(*ms) < 8_000;
            }
        }
        if fresh {
            // 心跳恢复：若此前处于恢复阶梯（Reload 复活成功），复位为 IDLE，
            // 下次挂死仍可先走轻量 Reload。
            if let Ok(mut m) = hang_watchdog::RECOVERY_STAGE.lock() {
                if m.get(label).copied().unwrap_or(0) != 0 {
                    m.remove(label);
                    log::info!("[hang-watchdog] {label} 心跳已恢复，复位恢复阶梯");
                }
            }
        }
        // 首次观察到可见：建立宽限基线（页面冷加载也需要时间）
        let mut since = now;
        if let Ok(mut map) = hang_watchdog::VISIBLE_SINCE_MS.lock() {
            *map.entry(label.to_string()).or_insert(now)
        } else {
            continue;
        };
        if let Ok(map) = hang_watchdog::VISIBLE_SINCE_MS.lock() {
            if let Some(v) = map.get(label) {
                since = *v;
            }
        }
        // 首次可见宽限（页面冷加载/刚被遮挡恢复也算）：8s 内不判挂死
        if fresh || now - since < 8_000 {
            continue;
        }
        // 遮挡判定：窗口中心点命中的还是不是自己。被其它窗口盖住时
        // Chromium 合法节流定时器，心跳断流不算挂死。
        log::debug!(
            "[hang-watchdog] {label} visible={visible} fresh={fresh} since_age={}ms",
            now - since
        );
        let Ok(pos) = window.outer_position() else {
            continue;
        };
        let Ok(size) = window.outer_size() else {
            continue;
        };
        let cx = pos.x + (size.width as i32) / 2;
        let cy = pos.y + (size.height as i32) / 2;
        let center_hwnd = unsafe {
            super::super::platform::windows::ffi_imports::WindowFromPoint(
                super::super::platform::windows::ffi_types::POINT { x: cx, y: cy },
            )
        };
        // WindowFromPoint 返回最深子窗口（WebView2 的 Chrome_WidgetWin_1 等），
        // 必须归一到根属窗口再与面板顶层 HWND 比较，否则永远判定"被遮挡"。
        let root_hwnd = unsafe {
            super::super::platform::windows::ffi_imports::GetAncestor(
                center_hwnd,
                super::super::platform::windows::ffi_types::GA_ROOT,
            )
        };
        let own = window.hwnd().map(|h| h.0 as usize).unwrap_or(0);
        log::debug!("[hang-watchdog] {label} hit={center_hwnd:?} root={root_hwnd:?} own={own:?}");
        if root_hwnd as usize != own {
            // 被遮挡：顺延基线，避免露出瞬间误杀
            if let Ok(mut map) = hang_watchdog::VISIBLE_SINCE_MS.lock() {
                map.insert(label.to_string(), now);
            }
            continue;
        }
        // 重建防抖：同 label 30s 内只重建一次
        let mut last_recreate = 0u64;
        if let Ok(map) = hang_watchdog::LAST_RECREATE_MS.lock() {
            if let Some(v) = map.get(label) {
                last_recreate = *v;
            }
        }
        // 防抖只约束「新周期（stage 0）启动」；阶梯推进（1→2）由各自宽限
        // 计时驱动——否则 Reload 无效后要再等半分钟才销毁重建。
        let stage_now = hang_watchdog::RECOVERY_STAGE
            .lock()
            .ok()
            .and_then(|m| m.get(label).copied())
            .unwrap_or(0);
        if stage_now == 0 && now - last_recreate < 30_000 {
            continue;
        }
        if stage_now == 0 {
            if let Ok(mut map) = hang_watchdog::LAST_RECREATE_MS.lock() {
                map.insert(label.to_string(), now);
            }
        }
        // —— 恢复阶梯 ——
        // 阶段 0→1：浏览器进程侧 Reload（渲染进程死亡/崩溃页由 browser 重新拉起，
        // 最轻、无窗口闪烁），给 ~15s 复活宽限。
        // 阶段 1→2：Reload 无效 → 投递主线程销毁窗口。
        // 阶段 2：poll 线程观察到 label 已释放后原位重建（两阶段避开 destroy 异步
        // 导致的 "label already exists" 竞态——2026-11-03 实测）。
        let stage = hang_watchdog::RECOVERY_STAGE
            .lock()
            .ok()
            .and_then(|m| m.get(label).copied())
            .unwrap_or(0);
        if stage == 2 {
            continue; // 等 pending-build 收尾
        }
        if stage == 0 {
            log::warn!("[hang-watchdog] {label} 心跳断流且未被遮挡，尝试浏览器侧 Reload");
            let owned = label.to_string();
            let delivered = window
                .with_webview(move |pw| unsafe {
                    // ICoreWebView2Controller → ICoreWebView2 才有 Reload()
                    if let Ok(core) = pw.controller().CoreWebView2() {
                        if core.Reload().is_ok() {
                            log::info!("[hang-watchdog] {owned} Reload 已投递");
                        }
                    }
                })
                .is_ok();
            if delivered {
                if let Ok(mut m) = hang_watchdog::RECOVERY_STAGE.lock() {
                    m.insert(label.to_string(), 1);
                }
                // 复活宽限：清心跳基线（新 renderer 起来会立刻恢复心跳）
                if let Ok(mut m) = hang_watchdog::HEARTBEAT_MS.lock() {
                    m.remove(label);
                }
                if let Ok(mut m) = hang_watchdog::LAST_FRAMES.lock() {
                    m.remove(label);
                }
                if let Ok(mut m) = hang_watchdog::VISIBLE_SINCE_MS.lock() {
                    m.insert(label.to_string(), now - 1_000);
                }
                continue;
            }
            log::warn!("[hang-watchdog] {label} Reload 投递失败，直接进入销毁阶段");
        }
        // stage==1（Reload 无效）或 Reload 投递失败：销毁 + 待重建
        log::warn!("[hang-watchdog] {label} 投递主线程销毁窗口，稍后原位重建");
        if let Ok(mut m) = hang_watchdog::RECOVERY_STAGE.lock() {
            m.insert(label.to_string(), 2);
        }
        if let Ok(mut m) = hang_watchdog::PENDING_GEOMETRY.lock() {
            m.insert(label.to_string(), (pos.x, pos.y, size.width, size.height));
        }
        let app_for_main = app.clone();
        let owned = label.to_string();
        let _ = app.run_on_main_thread(move || {
            if let Some(old) = app_for_main.get_webview_window(&owned) {
                let _ = old.destroy();
            }
        });
    }

    // —— 阶段 2 收尾：label 已释放则原位重建 ——
    let pending: Vec<(String, (i32, i32, u32, u32))> = hang_watchdog::PENDING_GEOMETRY
        .lock()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
        .unwrap_or_default();
    for (label, (px, py, sw, sh)) in pending {
        let Some(target) = ManagedWindow::from_label(&label) else {
            continue;
        };
        if app.get_webview_window(&label).is_some() {
            continue; // destroy 尚未落地，下一拍再看
        }
        match build_managed_window(app, target) {
            Ok(new_window) => {
                let scale = new_window.scale_factor().unwrap_or(1.0).max(0.01);
                let _ = new_window.set_size(tauri::LogicalSize::new(
                    f64::from(sw) / scale,
                    f64::from(sh) / scale,
                ));
                let _ = new_window.set_position(tauri::PhysicalPosition::new(px, py));
                let _ = new_window.show();
                let _ = new_window.set_focus();
                log::info!("[hang-watchdog] {label} 窗口已重建");
            }
            Err(error) => log::error!("[hang-watchdog] {label} 窗口重建失败: {error}"),
        }
        if let Ok(mut m) = hang_watchdog::PENDING_GEOMETRY.lock() {
            m.remove(&label);
        }
        if let Ok(mut m) = hang_watchdog::RECOVERY_STAGE.lock() {
            m.remove(&label);
        }
        if let Ok(mut m) = hang_watchdog::HEARTBEAT_MS.lock() {
            m.remove(&label);
        }
        if let Ok(mut m) = hang_watchdog::LAST_FRAMES.lock() {
            m.remove(&label);
        }
        if let Ok(mut m) = hang_watchdog::VISIBLE_SINCE_MS.lock() {
            m.insert(label, hang_watchdog::now_ms());
        }
    }
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
