//! Desktop Organizer - Entry point

// Windows subsystem (no console window), logs written to file
#![windows_subsystem = "windows"]

#[cfg(feature = "eframe-ui")]
mod app;
mod box_core;
mod classification;
mod config;
mod context;
#[allow(dead_code)]
mod ipc;
mod platform;
mod plugin;
mod shared;
#[cfg(feature = "eframe-ui")]
mod ui;
mod utils;

// Tauri commands module (new for Tauri migration)
#[cfg(feature = "tauri-ui")]
mod tauri_commands;

#[cfg(all(windows, feature = "gpui-ui"))]
mod gpui_app;

fn main() -> anyhow::Result<()> {
    utils::init_logger();
    install_panic_hook();
    #[cfg(windows)]
    log_boot_context();
    log::info!("========================================");
    log::info!("Starting Desktop Organizer (Rewrite)");
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

    // === Tauri 2.0 entry (new migration path) ===
    #[cfg(feature = "tauri-ui")]
    {
        run_tauri_app()?;
        Ok(())
    }

    // Legacy GPUI fallback (only with the gpui-ui feature)
    #[cfg(all(target_os = "windows", feature = "gpui-ui"))]
    {
        gpui_app::windows_main::run_gpui_app();
        return Ok(());
    }

    // eframe fallback (only when eframe-ui feature is enabled)
    #[cfg(feature = "eframe-ui")]
    {
        run_eframe_app()?;
        Ok(())
    }

    #[cfg(not(any(feature = "gpui-ui", feature = "eframe-ui", feature = "tauri-ui")))]
    {
        anyhow::bail!("Current platform not supported");
    }
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
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!("[PANIC] 回溯:\n{backtrace}");
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
#[cfg(feature = "tauri-ui")]
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

    Ok(())
}

/// Run eframe application (fallback UI runtime)
#[cfg(feature = "eframe-ui")]
fn run_eframe_app() -> anyhow::Result<()> {
    use eframe::egui;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(app::CONTROL_PANEL_TITLE)
            .with_inner_size([320.0, 380.0])
            .with_position([-32000.0, -32000.0])
            .with_decorations(true)
            .with_always_on_top()
            .with_resizable(true)
            .with_visible(true)
            .with_taskbar(false),
        ..Default::default()
    };

    eframe::run_native(
        "Desktop Organizer",
        options,
        Box::new(|cc| {
            setup_chinese_fonts(&cc.egui_ctx);
            Box::new(app::MyApp::new(cc))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe run failed: {}", e))?;

    Ok(())
}

#[cfg(feature = "eframe-ui")]
fn setup_chinese_fonts(ctx: &eframe::egui::Context) {
    let mut fonts = eframe::egui::FontDefinitions::default();

    let font_paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyhbd.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];

    for path in &font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            log::info!("Loading Chinese font: {}", path);
            fonts.font_data.insert(
                "chinese".to_owned(),
                eframe::egui::FontData::from_owned(font_data),
            );
            if let Some(f) = fonts
                .families
                .get_mut(&eframe::egui::FontFamily::Proportional)
            {
                f.insert(0, "chinese".to_owned());
            }
            if let Some(f) = fonts.families.get_mut(&eframe::egui::FontFamily::Monospace) {
                f.insert(0, "chinese".to_owned());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
    log::warn!("Chinese font not found");
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
