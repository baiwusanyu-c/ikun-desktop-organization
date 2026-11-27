//! 应用更新 Tauri Commands
//!
//! 基于 Tauri 官方 `tauri-plugin-updater` 提供「检查更新」与「一键在线升级」。
//!
//! ## 为什么不调用插件的 `install()` / `download_and_install()`
//!
//! 两者在 Windows 上最终都走到 `install_inner`：它在 `ShellExecuteW` 拉起安装器之后
//! 直接 `std::process::exit(0)`。这条路径**完全跳过**本项目 `platform_runtime.rs` 的退出
//! 清理链——分类框布局落盘、玻璃引擎 shutdown、系统托盘注销、桌面注册表清理、
//! 隐藏图标还原全部不执行，后果是升级后用户桌面图标停留在隐藏态；而且
//! `std::process::exit` 会跑 CRT atexit / DllMain 收尾，与仍在飞 COM 调用的
//! WebView2 / D2D / DComp 线程对撞（见 `platform_runtime.rs` 退出看门狗注释）。
//!
//! 所以这里只借用插件的**下载 + minisign 验签**能力（`Update::download` 在返回前已
//! `verify`，拿到的必然是可信字节），随后按下面的时序自己收尾：
//!
//! 1. 验签字节落盘到临时目录；
//! 2. 拉起一个独立于本进程的安装代理脚本（等本进程 PID 消失后静默安装）；
//! 3. 置 `quit_requested`，让平台轮询线程走完整清理链并正常结束进程。
//!
//! 这样既绕开 NSIS「杀不掉旧进程」的上游问题（旧进程已干净退出，安装器根本不需要 kill），
//! 又保住本项目全部退出清理。
//!
//! ## 权限面
//!
//! 更新逻辑全部留在 Rust 命令层：前端不引入 updater JS 插件，
//! `capabilities/default.json` 也无需放开 `updater` / `process` 权限
//! （capability 只约束 WebView IPC，`app.updater()` 不受其限制）。

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};

use crate::tauri_commands::app_cmd::request_quit;
use crate::tauri_commands::platform_runtime::SharedTauriRuntime;

/// `tauri.conf.json` 中 `plugins.updater.pubkey` 的未配置占位值。
///
/// 带着占位值发布会让老用户收到更新却验签失败，因此命令层必须短路并给出可读错误，
/// 而不是让插件抛一个 base64 解析异常糊在前端。
pub const PUBLIC_KEY_PLACEHOLDER: &str = "REPLACE_WITH_MINISIGN_PUBLIC_KEY";

/// 手动下载兜底页面（任何失败路径都引导用户到这里）。
const RELEASES_PAGE: &str = "https://github.com/baiwusanyu-c/ikun-desktop-organization/releases";

/// 下载进度事件名，前端通过 `listen` 订阅。
pub const UPDATE_PROGRESS_EVENT: &str = "update-download-progress";

/// 进度事件最小发送间隔（字节）：每个 chunk 都发事件会打满 IPC 并刷爆日志。
const PROGRESS_EMIT_INTERVAL_BYTES: u64 = 256 * 1024;

/// 安装代理等待本进程退出的上限（秒），防止 PID 复用或代理被挂起后死等。
const PROXY_PID_WAIT_TIMEOUT_SECS: u32 = 300;

/// 本进程退出后额外等待的毫秒数：让操作系统释放 exe 文件句柄，
/// 否则安装器可能撞上「文件被占用」。
const PROXY_HANDLE_RELEASE_DELAY_MS: u32 = 800;

/// 下载回调与完成回调共享的进度状态。
#[derive(Debug, Default)]
struct DownloadProgressState {
    downloaded: u64,
    total: Option<u64>,
    last_emitted: u64,
}

/// 检查更新结果。
///
/// Rust 侧为协议源，`frontend/src/lib/api/tauri-api.ts` 的 `UpdateCheck` 必须同步这些字段
/// （沿用项目既有的 snake_case 直传风格，不做 rename）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateCheck {
    /// 当前运行版本
    pub current_version: String,
    /// 是否存在可用更新
    pub available: bool,
    /// 可更新的版本号；无更新时为 `None`
    pub latest_version: Option<String>,
    /// 更新说明（来自更新清单的 notes）
    pub notes: Option<String>,
    /// 发布时间（RFC 3339 文本）
    pub pub_date: Option<String>,
    /// 手动下载兜底页面
    pub download_page: String,
}

/// 下载进度事件负载。
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateProgress {
    /// 已下载字节数
    pub downloaded: u64,
    /// 总字节数；服务端未给出 Content-Length 时为 `None`
    pub total: Option<u64>,
    /// 百分比 0-100；总量未知时恒为 0
    pub percent: u32,
}

/// 更新包在本机的暂存位置。
#[derive(Debug, Default)]
struct StagedUpdate {
    /// 验签通过的 NSIS 安装器本体（v2 原生 updater 产物，不是 zip 载体）
    package_path: PathBuf,
    /// 安装代理脚本
    script_path: PathBuf,
}

/// 从 `plugins.updater` 配置节判断签名公钥是否已填入真实值。
///
/// 公钥缺失或仍是占位值时，检查/安装都必须给出可诊断的中文错误，而不是让插件抛一个
/// base64 解析异常糊在前端。抽成纯函数是为了能脱离 `AppHandle` 做单元测试。
fn pubkey_configured(updater_config: Option<&serde_json::Value>) -> bool {
    updater_config
        .and_then(|config| config.get("pubkey"))
        .and_then(|value| value.as_str())
        .map(|key| {
            let key = key.trim();
            !key.is_empty() && key != PUBLIC_KEY_PLACEHOLDER
        })
        .unwrap_or(false)
}

/// 更新功能是否已配置真实签名公钥（读取 Tauri 配置）。
fn updater_pubkey_configured(app: &AppHandle) -> bool {
    pubkey_configured(app.config().plugins.0.get("updater"))
}

/// 未配置公钥时的统一错误文案。
fn ensure_updater_configured(app: &AppHandle) -> Result<(), String> {
    if updater_pubkey_configured(app) {
        return Ok(());
    }
    log::warn!("[update_cmd] 更新插件公钥缺失或仍为占位值，拒绝执行更新流程");
    Err(format!(
        "更新功能尚未配置签名公钥：请在 tauri.conf.json 的 plugins.updater.pubkey 填入 minisign 公钥（当前仍为 {PUBLIC_KEY_PLACEHOLDER}）"
    ))
}

/// 把插件错误翻译成用户可读的中文，并区分「网络问题 / 清单缺失 / 验签失败」三类。
fn describe_updater_error(error: UpdaterError) -> String {
    log::warn!("[update_cmd] 更新插件返回错误: {error}");
    match error {
        UpdaterError::EmptyEndpoints => {
            "更新检查未配置任何更新源，请检查 tauri.conf.json 的 plugins.updater.endpoints"
                .to_string()
        }
        UpdaterError::ReleaseNotFound
        | UpdaterError::TargetNotFound(_)
        | UpdaterError::TargetsNotFound(_) => {
            "更新服务器上没有可用的更新清单（latest.json 缺失或不含当前平台），请先完成一次发布"
                .to_string()
        }
        UpdaterError::Reqwest(_) | UpdaterError::Network(_) => {
            "无法连接更新服务器，请检查网络后重试".to_string()
        }
        UpdaterError::Minisign(_)
        | UpdaterError::Base64(_)
        | UpdaterError::SignatureUtf8(_)
        | UpdaterError::SignedVersionMismatch { .. }
        | UpdaterError::MissingSignedVersion => {
            "更新包签名校验未通过，已拒绝安装（公钥与发布私钥不匹配，或发布链路被篡改）".to_string()
        }
        UpdaterError::Io(_) => "写入更新临时文件失败，请检查磁盘空间与目录权限".to_string(),
        other => format!("更新失败，请稍后重试，或到发布页手动下载：{other}"),
    }
}

/// 按当前版本构造检查结果。
fn build_check(current_version: String, available: bool) -> UpdateCheck {
    UpdateCheck {
        latest_version: None,
        notes: None,
        pub_date: None,
        download_page: RELEASES_PAGE.to_string(),
        current_version,
        available,
    }
}

/// 构造 updater 实例，公钥取 trim 后的配置值。
///
/// 不直接用 `app.updater()`（那会把配置里的公钥原值交给插件）：守卫判的是 trim 后的结果，
/// 若此处放行未 trim 的原值，就会出现「守卫通过、插件 base64 解码失败」的不一致——
/// 手工编辑 `tauri.conf.json` 时极易顺手带上空格或换行。
fn build_updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let pubkey = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|config| config.get("pubkey"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    app.updater_builder()
        .pubkey(pubkey)
        .build()
        .map_err(describe_updater_error)
}

/// 检查应用更新。
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateCheck, String> {
    log::info!("[update_cmd] 收到检查更新请求");
    ensure_updater_configured(&app)?;

    let updater = build_updater(&app)?;
    let update = updater.check().await.map_err(describe_updater_error)?;
    let current_version = app.package_info().version.to_string();

    match update {
        Some(update) => {
            log::info!(
                "[update_cmd] 发现可用更新: {current_version} -> {}",
                update.version
            );
            Ok(UpdateCheck {
                latest_version: Some(update.version.clone()),
                notes: update.body.clone(),
                pub_date: update.date.map(|date| date.to_string()),
                available: true,
                current_version,
                download_page: RELEASES_PAGE.to_string(),
            })
        }
        None => {
            log::info!("[update_cmd] 已是最新版本: {current_version}");
            Ok(build_check(current_version, false))
        }
    }
}

/// 下载更新包，按固定字节间隔把进度广播给前端。
async fn download_with_progress(app: &AppHandle, update: &Update) -> Result<Vec<u8>, String> {
    let progress_state = Arc::new(Mutex::new(DownloadProgressState::default()));
    let chunk_state = Arc::clone(&progress_state);
    let finish_state = Arc::clone(&progress_state);
    let handle = app.clone();
    let finish_handle = app.clone();

    let bytes = update
        .download(
            move |chunk_len, total| {
                let Ok(mut state) = chunk_state.lock() else {
                    return;
                };
                state.downloaded = state.downloaded.saturating_add(chunk_len as u64);
                if total.is_some() {
                    state.total = total;
                }
                if state.downloaded.saturating_sub(state.last_emitted)
                    < PROGRESS_EMIT_INTERVAL_BYTES
                {
                    return;
                }
                state.last_emitted = state.downloaded;
                let downloaded = state.downloaded;
                let total = state.total;
                drop(state);
                emit_progress(&handle, downloaded, total);
            },
            move || {
                // 收尾保留已知总量，确保最后一帧能显示 100%，而不是回退到 0%。
                if let Ok(state) = finish_state.lock() {
                    let downloaded = state.downloaded;
                    let total = state.total;
                    drop(state);
                    emit_progress(&finish_handle, downloaded, total);
                }
            },
        )
        .await
        .map_err(describe_updater_error)?;

    log::info!("[update_cmd] 更新包下载并验签通过: {} 字节", bytes.len());
    Ok(bytes)
}

/// 发送一次下载进度事件。
fn emit_progress(app: &AppHandle, downloaded: u64, total: Option<u64>) {
    let percent = match total {
        Some(total) if total > 0 => (downloaded.saturating_mul(100) / total).min(100) as u32,
        _ => 0,
    };
    let progress = UpdateProgress {
        downloaded,
        total,
        percent,
    };
    if let Err(error) = app.emit(UPDATE_PROGRESS_EVENT, &progress) {
        log::warn!("[update_cmd] 下载进度事件发送失败: {error}");
    }
}

/// 把已验签的更新包与安装代理脚本写入临时目录。
///
/// `createUpdaterArtifacts: true`（v2 原生）下 `download()` 拿到的字节就是 NSIS 安装器本体，
/// 落盘即可直接执行。只有 `"v1Compatible"` 才是 zip 载体，那种格式刻意不采用：
/// 少一层解压就少一个失败点，且产物文件名与现有发布 asset 完全一致。
fn stage_update_package(bytes: &[u8]) -> Result<StagedUpdate, String> {
    let staging_dir = std::env::temp_dir().join("ikun_updater");
    std::fs::create_dir_all(&staging_dir)
        .map_err(|error| format!("创建更新暂存目录失败: {}: {error}", staging_dir.display()))?;

    let package_path = staging_dir.join("update-setup.exe");
    std::fs::write(&package_path, bytes)
        .map_err(|error| format!("写入更新包失败: {}: {error}", package_path.display()))?;

    let script_path = staging_dir.join("install-proxy.ps1");
    std::fs::write(&script_path, build_proxy_script())
        .map_err(|error| format!("写入安装代理脚本失败: {error}"))?;

    Ok(StagedUpdate {
        package_path,
        script_path,
    })
}

/// 生成安装代理脚本。
///
/// 脚本必须**完全独立于本进程**运行：本进程随后会走优雅退出清理链并结束，
/// 所以由脚本负责「等旧进程消失 -> 静默覆盖安装 -> 无论成败都把应用拉回来」。
/// 最后一步是无条件的：安装失败时旧版 exe 仍在原地，用户不该看到「升级把应用弄没了」。
///
/// 路径只经命令行参数传入、不拼进脚本文本，避免安装路径里的引号破坏脚本。
fn build_proxy_script() -> String {
    format!(
        r#"param([int]$OldPid, [string]$Package, [string]$AppExe)
$ErrorActionPreference = 'SilentlyContinue'
try {{
    $deadline = (Get-Date).AddSeconds({timeout_secs})
    while (Get-Process -Id $OldPid -ErrorAction SilentlyContinue) {{
        if ((Get-Date) -gt $deadline) {{ break }}
        Start-Sleep -Milliseconds 200
    }}
    Start-Sleep -Milliseconds {delay_ms}
    if (Test-Path -LiteralPath $Package) {{
        Start-Process -FilePath $Package -ArgumentList '/P','/UPDATE' -Wait
    }}
}} finally {{
    Start-Process -FilePath $AppExe
}}
Remove-Item -LiteralPath $Package -Force
"#,
        timeout_secs = PROXY_PID_WAIT_TIMEOUT_SECS,
        delay_ms = PROXY_HANDLE_RELEASE_DELAY_MS,
    )
}

/// 拉起安装代理（detached，不等待、不随本进程结束而被回收）。
fn spawn_install_proxy(staged: &StagedUpdate) -> Result<Child, String> {
    let app_exe =
        std::env::current_exe().map_err(|error| format!("无法定位当前可执行文件路径: {error}"))?;

    let mut command = Command::new("powershell.exe");
    command
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-WindowStyle")
        .arg("Hidden")
        .arg("-File")
        .arg(&staged.script_path)
        .arg("-OldPid")
        .arg(std::process::id().to_string())
        .arg("-Package")
        .arg(&staged.package_path)
        .arg("-AppExe")
        .arg(&app_exe);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 0x08000000 = CREATE_NO_WINDOW：代理是纯后台脚本，不该闪一个控制台窗口
        command.creation_flags(0x0800_0000);
    }

    let child = command
        .spawn()
        .map_err(|error| format!("启动安装代理失败: {error}"))?;
    log::info!(
        "[update_cmd] 安装代理已就绪: {}",
        staged.script_path.display()
    );
    Ok(child)
}

/// 下载并安装更新：验签 -> 落盘 -> 拉起安装代理 -> 触发既有优雅退出。
///
/// 注意本命令返回后进程即将退出，前端**不能**依赖返回值渲染最终状态，
/// 必须在调用前就把「应用即将退出并重启」呈现出来。
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    runtime: State<'_, SharedTauriRuntime>,
) -> Result<String, String> {
    log::info!("[update_cmd] 收到安装更新请求");
    ensure_updater_configured(&app)?;

    let updater = build_updater(&app)?;
    let update = updater
        .check()
        .await
        .map_err(describe_updater_error)?
        .ok_or_else(|| "当前已是最新版本，无需更新".to_string())?;

    let version = update.version.clone();
    let bytes = download_with_progress(&app, &update).await?;
    let staged = stage_update_package(&bytes)?;
    let mut proxy = spawn_install_proxy(&staged)?;

    // 复用既有退出路径：平台轮询线程会完成布局落盘、玻璃引擎 shutdown、托盘注销、
    // 桌面注册表清理与隐藏图标还原，最后正常结束进程；安装代理在此之前已就位。
    if let Err(error) = request_quit(runtime.inner()) {
        // 无法触发优雅退出时必须取消代理，避免它在本进程仍存活时超时后强装。
        if let Err(kill_error) = proxy.kill() {
            log::warn!("[update_cmd] 取消安装代理失败: {kill_error}");
        }
        if let Err(wait_error) = proxy.wait() {
            log::warn!("[update_cmd] 等待安装代理结束失败: {wait_error}");
        }
        return Err(format!("触发应用退出失败: {error}"));
    }

    log::info!("[update_cmd] v{version} 已下载并校验，安装代理就绪，开始优雅退出以完成升级");
    Ok(format!(
        "v{version} 已下载并校验通过，应用即将退出并完成安装"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 占位公钥、空公钥、完全没配 pubkey 都必须判为「未配置」，
    /// 否则带着占位值发布出去，老用户只会收到一个看不懂的验签失败。
    #[test]
    fn placeholder_or_missing_pubkey_is_not_configured() {
        let placeholder: serde_json::Value =
            serde_json::json!({ "pubkey": PUBLIC_KEY_PLACEHOLDER });
        let blank: serde_json::Value = serde_json::json!({ "pubkey": "   " });
        let padded: serde_json::Value =
            serde_json::json!({ "pubkey": format!("  {PUBLIC_KEY_PLACEHOLDER}  ") });
        let absent: serde_json::Value = serde_json::json!({ "endpoints": [] });
        assert!(!pubkey_configured(Some(&placeholder)));
        assert!(!pubkey_configured(Some(&blank)));
        assert!(!pubkey_configured(Some(&padded)));
        assert!(!pubkey_configured(Some(&absent)));
        assert!(!pubkey_configured(None));
    }

    /// 真实 minisign 公钥（非占位、非空）才放行。
    #[test]
    fn real_pubkey_is_recognised_as_configured() {
        let configured: serde_json::Value = serde_json::json!({ "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXkgQUJDMTIz" });
        assert!(pubkey_configured(Some(&configured)));
    }

    /// 三类失败必须给出可区分的中文文案：网络问题、清单缺失、验签失败的处理方式完全不同，
    /// 混成一句「更新失败」会让用户和排障的人都无从下手。
    #[test]
    fn updater_errors_are_translated_into_distinct_messages() {
        let network = describe_updater_error(UpdaterError::Network("404 Not Found".into()));
        let missing_manifest = describe_updater_error(UpdaterError::ReleaseNotFound);
        let bad_signature = describe_updater_error(UpdaterError::SignatureUtf8("xx".into()));
        assert!(
            network.contains("网络"),
            "网络类错误要提示检查网络: {network}"
        );
        assert!(
            missing_manifest.contains("清单"),
            "清单缺失要说明 latest.json: {missing_manifest}"
        );
        assert!(
            bad_signature.contains("签名"),
            "验签失败要说明签名问题: {bad_signature}"
        );
        assert_ne!(network, missing_manifest);
        assert_ne!(missing_manifest, bad_signature);
    }

    /// 安装代理脚本的时序契约：等旧进程消失 -> 等句柄释放 -> 静默覆盖安装 ->
    /// **无条件**把应用拉回来。最后一步是兜底：安装失败时旧版 exe 还在原地，
    /// 用户绝不能看到「升级把应用弄没了」。
    #[test]
    fn proxy_script_waits_for_exit_then_always_relaunches_app() {
        let script = build_proxy_script();
        assert!(
            script.contains("Get-Process -Id $OldPid"),
            "必须等旧进程退出"
        );
        assert!(
            script.contains("Start-Sleep -Milliseconds 800"),
            "必须留句柄释放窗口"
        );
        assert!(
            script.contains("Test-Path -LiteralPath $Package"),
            "必须先确认更新包已落盘"
        );
        assert!(
            !script.contains("Expand-Archive"),
            "v2 原生 updater 产物是 setup.exe 本体，不该再走 zip 解压"
        );
        assert!(
            script.contains("param([int]$OldPid, [string]$Package, [string]$AppExe)"),
            "脚本参数必须与 Rust 侧 -OldPid / -Package / -AppExe 一一对应",
        );
        assert!(
            script.contains("'/P','/UPDATE'"),
            "NSIS 必须走 passive + 更新模式，且不携带 /R（重启交给脚本）",
        );
        assert!(!script.contains("/R"), "带上 /R 会和脚本双重拉起应用");
        assert!(
            script.contains("} finally {"),
            "安装阶段必须放在 try/finally 中，保证失败也会重启应用",
        );
        let relaunch = script
            .find("Start-Process -FilePath $AppExe")
            .expect("脚本必须无条件重启应用");
        let install = script.find("-Wait").expect("安装器必须被等待完成后再重启");
        assert!(relaunch > install, "重启必须发生在安装器退出之后");
    }

    /// 占位常量与 `tauri.conf.json` 必须一致：改了配置文件却忘了改常量，
    /// 守卫会误判成「已配置」，把坏清单推给所有老用户。
    #[test]
    fn placeholder_matches_tauri_config() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let raw = std::fs::read_to_string(format!("{manifest_dir}/tauri.conf.json"))
            .expect("tauri.conf.json 必须可读");
        let config: serde_json::Value = serde_json::from_str(&raw).expect("配置必须是合法 JSON");
        let pubkey = config
            .pointer("/plugins/updater/pubkey")
            .and_then(|value| value.as_str())
            .expect("必须存在 plugins.updater.pubkey");
        let endpoint = config
            .pointer("/plugins/updater/endpoints/0")
            .and_then(|value| value.as_str())
            .expect("必须至少配置一个更新源");
        let create_updater_artifacts = config
            .pointer("/bundle/createUpdaterArtifacts")
            .and_then(|value| value.as_bool())
            .expect("必须显式声明 bundle.createUpdaterArtifacts");
        // 未填真实公钥时，配置值必须正好等于占位常量，否则守卫失效
        assert!(
            pubkey == PUBLIC_KEY_PLACEHOLDER || pubkey.starts_with("dW50cnVzdGVk"),
            "pubkey 要么是占位常量，要么是真实 minisign 公钥: {pubkey}"
        );
        // GitHub 的 releases/latest 永不包含 prerelease，beta 通道必须走固定滚动 tag
        assert_eq!(
            endpoint,
            "https://github.com/baiwusanyu-c/ikun-desktop-organization/releases/download/beta/latest.json",
            "更新源必须使用可匿名访问的 beta 滚动 latest.json",
        );
        // 配置文件必须保持 false：写死 true 会让任何 tauri build（含 fork PR 的 CI 构建）
        // 都要求 TAURI_SIGNING_PRIVATE_KEY，而 PR 拿不到 secrets，只能恒红。
        // 签名产物改由 `build.mjs --updater` 经 --config 深合并开启（见下方守卫测试）。
        assert!(
            !create_updater_artifacts,
            "createUpdaterArtifacts 必须为 false，签名产物由 build.mjs --updater 覆盖开启",
        );

        let capabilities =
            std::fs::read_to_string(format!("{manifest_dir}/capabilities/default.json"))
                .expect("capabilities/default.json 必须可读");
        assert!(
            !capabilities.contains("updater:"),
            "WebView 不得获得 updater 权限"
        );
        assert!(
            !capabilities.contains("process:"),
            "WebView 不得获得 process 权限"
        );
    }

    /// Rust 与 TypeScript 是双侧协议：任一字段或事件名只改一侧都必须让测试失败。
    #[test]
    fn frontend_update_contract_matches_rust() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let api =
            std::fs::read_to_string(format!("{manifest_dir}/frontend/src/lib/api/tauri-api.ts"))
                .expect("tauri-api.ts 必须可读");
        let event_contract = format!("const UPDATE_PROGRESS_EVENT = {UPDATE_PROGRESS_EVENT:?};");
        assert!(
            api.contains(&event_contract),
            "前端事件名必须与 Rust UPDATE_PROGRESS_EVENT 完全一致"
        );
        for field in [
            "export interface UpdateCheck {",
            "current_version: string;",
            "available: boolean;",
            "latest_version: string | null;",
            "notes: string | null;",
            "pub_date: string | null;",
            "download_page: string;",
            "export interface UpdateProgress {",
            "downloaded: number;",
            "total: number | null;",
            "percent: number;",
        ] {
            assert!(api.contains(field), "前端更新契约缺少字段或定义: {field}");
        }
    }

    /// `createUpdaterArtifacts` 与发布开关必须分处两地：配置文件保持 false
    /// （普通 CI 构建无需签名私钥），发布链路通过 `build.mjs --updater` 深合并覆盖成 true。
    ///
    /// 任一侧被误改都会静默破坏：写回 true 让 fork PR 恒红（PR 拿不到 secrets）；
    /// 发布侧丢掉 --updater 则产物没有 setup.exe.sig，老用户永远收不到可验签的更新。
    #[test]
    fn updater_artifacts_are_gated_behind_the_release_flag() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let build_script = std::fs::read_to_string(format!("{manifest_dir}/scripts/build.mjs"))
            .expect("scripts/build.mjs 必须可读");
        assert!(
            build_script.contains(r#"case "--updater""#),
            "build.mjs 必须提供 --updater 开关"
        );
        assert!(
            build_script.contains("createUpdaterArtifacts: true"),
            "--updater 必须通过 overlay 把 createUpdaterArtifacts 置为 true"
        );
        assert!(
            build_script.contains("--config"),
            "overlay 必须经 tauri build --config 深合并注入，不能改回写死配置"
        );

        let workflow =
            std::fs::read_to_string(format!("{manifest_dir}/.github/workflows/release.yml"))
                .expect(".github/workflows/release.yml 必须可读");
        assert!(
            workflow.contains("build.mjs --platform win --mode release --updater"),
            "发布 workflow 必须显式带 --updater，否则产物缺少 setup.exe.sig"
        );
        assert!(
            workflow.contains("TAURI_SIGNING_PRIVATE_KEY:"),
            "发布 workflow 必须注入签名私钥环境变量"
        );

        // 普通 CI 构建绝不应该持有生产签名私钥
        let ci = std::fs::read_to_string(format!("{manifest_dir}/.github/workflows/build.yml"))
            .expect(".github/workflows/build.yml 必须可读");
        assert!(
            !ci.contains("--updater"),
            "普通 CI 构建不得带 --updater：它不应该持有生产签名私钥"
        );
        assert!(
            !ci.contains("TAURI_SIGNING_PRIVATE_KEY"),
            "普通 CI 构建不得注入签名私钥环境变量"
        );
    }
}
