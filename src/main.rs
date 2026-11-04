//! Desktop Organizer - Entry point

// Windows subsystem (no console window), logs written to file
#![windows_subsystem = "windows"]

mod box_core;
mod classification;
mod config;
mod context;
mod platform;
mod tauri_commands;
mod utils;

/// 全局分配器：透传 `System`，只做计数。
///
/// 用途见 `utils::alloc_probe` —— 把「Rust 堆」从「D2D / D3D11 /
/// 显卡驱动私有分配」里切出来，否则进程私有字节缓慢爬升时无法归因。
#[global_allocator]
static ALLOCATOR: utils::alloc_probe::CountingAllocator = utils::alloc_probe::CountingAllocator;

fn main() -> anyhow::Result<()> {
    utils::init_logger();
    install_panic_hook();
    #[cfg(windows)]
    log_boot_context();
    log::info!("========================================");
    log::info!("Starting Desktop Organizer");
    log::info!("========================================");

    // Set Per-Monitor DPI V2 awareness (runtime setting, supplement to manifest)
    #[cfg(target_os = "windows")]
    {
        set_per_monitor_dpi_v2();
        init_ole();
    }

    // Desktop right-click menu registry trigger: communicate via signal files
    // Write signal file and exit immediately; running instance polls and acts
    if let Some(arg) = std::env::args().nth(1) {
        let signal_name = match arg.as_str() {
            "--create-box" => Some("ikun_create_box.signal"),
            "--open-panel" => Some("ikun_open_panel.signal"),
            "--ai-classify" => Some("ikun_ai_classify.signal"),
            "--quit" => Some("ikun_quit.signal"),
            _ => None,
        };
        if let Some(name) = signal_name {
            let signal = std::env::temp_dir().join(name);
            let _ = std::fs::write(&signal, "1");
            return Ok(());
        }
    }

    // Tauri 2.0 is the sole UI runtime (Windows-only product).
    run_tauri_app()
}

/// 安装全局 panic hook：panic 时把信息与回溯写入日志文件。
///
/// `windows_subsystem = "windows"` 下进程无控制台，启动早期的 panic 会静默退出，
/// 表现为"开机自启没有运行"。把崩溃记录到日志后，可以通过日志区分
/// "Windows 没拉起"和"拉起后崩溃"两种情况。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("[PANIC] {info}");
        if let Some(location) = info.location() {
            log::error!(
                "[PANIC] 位置: {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }
        // 库侧 panic 会直接杀死玻璃渲染线程，且库的 `running` 标志停在 true
        // （退出路径被 panic 跳过），宿主无法靠 `is_running()` 发现。这里按栈地址
        // 记一个「嫌疑渲染线程已死」标志，交给 runtime tick 做确认后复活。
        // 只置标志，绝不在 hook 里调引擎 API：hook 运行在正在死亡的那个线程上，
        // 可能正持着库内写锁，一调就死锁。
        #[cfg(windows)]
        if let Some(location) = info.location() {
            if location.file().replace('\\', "/").contains("liquid-verre") {
                // 库渲染线程名为 `liquid-verre-render`：只接受渲染线程（或未命名
                // 线程）上的库 panic 作为嫌疑信号，避免其它线程的库内 panic 把
                // 健康引擎误判死亡并触发无谓重建（2026-11-04 自检审计 P2）。
                let on_engine_thread = std::thread::current()
                    .name()
                    .map(|n| n.contains("liquid-verre") || n.contains("glass"))
                    .unwrap_or(true);
                if on_engine_thread {
                    crate::platform::windows::glass_engine::mark_engine_suspect();
                }
            }
        }
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!("[PANIC] 回溯:\n{backtrace}");
        // 旁路直写文件：日志器本身卡死时 panic 也要留下证据
        // （2026-09-15 拖拽后线程死亡且日志全停，无从查起）。
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("ikun_panic.log"))
        {
            use std::io::Write;
            let _ = writeln!(f, "[PANIC] {info}");
            if let Some(location) = info.location() {
                let _ = writeln!(
                    f,
                    "[PANIC] loc {}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                );
            }
            let _ = writeln!(f, "{}", std::backtrace::Backtrace::force_capture());
        }
        default_hook(info);
    }));
}

/// 记录开机上下文诊断日志。
///
/// 输出系统已运行时长与可执行文件路径；若系统运行时长小于 180 秒，
/// 提示本次启动可能由开机自启触发，便于排查“设置了自启却没有启动”的问题：
/// - 开机后日志文件中存在该行 → Windows 已拉起程序，问题在程序内部；
/// - 不存在该行 → Windows 未拉起程序，问题在注册表/安全软件。
#[cfg(windows)]
fn log_boot_context() {
    extern "system" {
        fn GetTickCount64() -> u64;
    }

    let uptime_s = unsafe { GetTickCount64() } / 1000;
    if let Ok(exe) = std::env::current_exe() {
        log::info!(
            "[autostart] 启动上下文: 系统已运行 {}s, exe={}",
            uptime_s,
            exe.display()
        );
    } else {
        log::info!("[autostart] 启动上下文: 系统已运行 {}s, exe=未知", uptime_s);
    }
    if uptime_s < 180 {
        log::info!("[autostart] 系统运行时长小于 180s，本次启动可能由开机自启触发");
    }
}

/// Run Tauri 2.0 application
fn run_tauri_app() -> anyhow::Result<()> {
    log::info!("Starting Tauri 2.0 application...");

    // Create the shared runtime BEFORE the builder so it can be registered via
    // `Builder::manage()`. This guarantees the state is available the moment
    // any WebView window is created, eliminating the window where a command
    // could fire before setup() has finished.
    let runtime = tauri_commands::platform_runtime::new_shared_runtime();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(runtime.clone())
        .invoke_handler(tauri::generate_handler![
            // Config commands
            tauri_commands::get_config,
            tauri_commands::load_config,
            tauri_commands::save_config,
            tauri_commands::update_theme_color,
            tauri_commands::update_box_style,
            tauri_commands::update_liquid_glass,
            tauri_commands::update_ai_config,
            tauri_commands::reset_config,
            tauri_commands::export_config,
            tauri_commands::import_config,
            // Box commands
            tauri_commands::validate_box_name,
            tauri_commands::create_box,
            tauri_commands::edit_box,
            tauri_commands::delete_box,
            tauri_commands::list_boxes,
            tauri_commands::get_box,
            tauri_commands::get_pending_edit_box,
            // Window commands
            tauri_commands::show_window,
            tauri_commands::hide_window,
            tauri_commands::close_window,
            tauri_commands::open_url,
            tauri_commands::check_update,
            // App commands
            tauri_commands::get_app_info,
            tauri_commands::quit_app,
            tauri_commands::toggle_window,
            tauri_commands::is_window_visible,
            tauri_commands::get_screen_resolution,
            // Log commands
            tauri_commands::log_message,
            // AI Classification commands
            tauri_commands::show_ai_classify_dialog,
            tauri_commands::start_ai_classification,
        ])
        .setup(|app| {
            log::info!("Tauri setup complete");

            // WebView 挂死看门狗的前半段：记录各窗口心跳（前端 500ms 一发）。
            {
                use tauri::Listener;
                app.listen("panel-heartbeat", |event| {
                    // payload：{ label, frames }（frames = rAF 帧计数，冻结即呈现停滞）
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                        let label = value.get("label").and_then(|v| v.as_str()).unwrap_or("");
                        let frames = value.get("frames").and_then(|v| v.as_u64()).unwrap_or(0);
                        if !label.is_empty() {
                            tauri_commands::window_cmd::record_heartbeat(label, frames);
                        }
                    }
                });
            }

            // Start the platform poller (system tray, hotkey, desktop registry).
            // The SharedTauriRuntime state is already managed via Builder::manage().
            let app_handle = app.handle().clone();
            tauri_commands::platform_runtime::start_platform_runtime(app_handle, runtime);

            Ok(())
        })
        .on_window_event(|window, event| {
            if tauri_commands::window_cmd::is_persistent_window_label(window.label()) {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if let Err(error) = window.hide() {
                        log::warn!("隐藏窗口失败: label={}, error={error}", window.label());
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Tauri runtime error: {}", e))?;

    // 走到这里说明 Tauri 事件循环已正常返回：给退出看门狗一个明确信号，
    // 它就不必再 TerminateProcess 补刀（顺带也验证「正常退出到底走不走得通」）。
    log::info!("[boot] Tauri 事件循环已返回，进程正常收尾");
    tauri_commands::platform_runtime::mark_teardown_done();

    Ok(())
}

/// Set Per-Monitor DPI V2 awareness
#[cfg(target_os = "windows")]
fn set_per_monitor_dpi_v2() {
    use std::ffi::c_void;

    #[allow(non_camel_case_types)]
    type DPI_AWARENESS_CONTEXT = *mut c_void;
    const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: DPI_AWARENESS_CONTEXT =
        -4isize as *mut c_void;

    extern "system" {
        fn SetProcessDpiAwarenessContext(value: DPI_AWARENESS_CONTEXT) -> i32;
    }

    let result =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if result != 0 {
        log::info!("Per-Monitor DPI V2 awareness set (runtime)");
    } else {
        log::warn!("Failed to set Per-Monitor DPI V2, relying on manifest");
    }
}

/// Initialize OLE (required for drag-and-drop)
#[cfg(target_os = "windows")]
fn init_ole() {
    extern "system" {
        fn OleInitialize(pMalloc: *mut std::ffi::c_void) -> i32;
    }
    let hr = unsafe { OleInitialize(std::ptr::null_mut()) };
    if hr == 0 || hr == 1 {
        log::info!("OLE initialized successfully");
    } else {
        log::warn!("OleInitialize failed: hr={:#010X}", hr as u32);
    }
    check_integrity_level();
}

/// Check process integrity level
#[cfg(target_os = "windows")]
fn check_integrity_level() {
    use std::ffi::c_void;

    type HANDLE = *mut c_void;
    type PSID = *mut c_void;

    extern "system" {
        fn GetCurrentProcess() -> HANDLE;
        fn OpenProcessToken(
            process_handle: HANDLE,
            desired_access: u32,
            token_handle: *mut HANDLE,
        ) -> i32;
        fn GetTokenInformation(
            token_handle: HANDLE,
            token_information_class: u32,
            token_information: *mut c_void,
            token_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: HANDLE) -> i32;
    }

    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_INTEGRITY_LEVEL: u32 = 25;
    const SECURITY_MANDATORY_HIGH_RID: u32 = 0x00003000;

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return;
        }
        let mut needed: u32 = 0;
        GetTokenInformation(
            token,
            TOKEN_INTEGRITY_LEVEL,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );
        if needed > 0 {
            let mut buffer = vec![0u8; needed as usize];
            if GetTokenInformation(
                token,
                TOKEN_INTEGRITY_LEVEL,
                buffer.as_mut_ptr() as *mut c_void,
                needed,
                &mut needed,
            ) != 0
            {
                let label_ptr = *(buffer.as_ptr() as *const PSID);
                if !label_ptr.is_null() {
                    extern "system" {
                        fn GetSidSubAuthorityCount(pSid: PSID) -> *mut u8;
                        fn GetSidSubAuthority(pSid: PSID, nSubAuthority: u32) -> *mut u32;
                    }
                    let count = *GetSidSubAuthorityCount(label_ptr) as u32;
                    if count > 0 {
                        let rid = *GetSidSubAuthority(label_ptr, count - 1);
                        if rid >= SECURITY_MANDATORY_HIGH_RID {
                            log::warn!("HIGH integrity (admin) - OLE drag will be blocked by UIPI");
                        }
                    }
                }
            }
        }
        CloseHandle(token);
    }
}
