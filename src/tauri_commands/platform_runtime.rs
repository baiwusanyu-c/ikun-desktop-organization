//! Tauri platform runtime
//!
//! Initializes system tray, desktop registry, global hotkeys, and overlay windows.
//! Bridges native platform events to Tauri WebView windows.

use crate::context::AppContext;
use crate::platform::{
    cleanup_desktop_registry, cleanup_system_tray, create_grid_window, destroy_all_grid_windows,
    extract_file_icon, icon_exists_for_box, resolve_icon_read_path, set_tray_auto_start_state,
    setup_desktop_registry, setup_global_hotkey, setup_system_tray, take_hotkey_toggle,
    take_pending_create_box, take_pending_tray_action, GridCreateParams, GridFileItem,
    SystemTrayAction,
};
use crate::tauri_commands::window_cmd::{
    show_managed_window, toggle_managed_window, ManagedWindow,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

/// Tauri Commands 与平台轮询线程共享的唯一运行时实例。
pub type SharedTauriRuntime = Arc<Mutex<TauriRuntime>>;

/// Desktop icon position restore retry delays (milliseconds).
/// Applied when SysListView32 control is not yet ready after layout changes.
const DESKTOP_RESTORE_RETRY_DELAYS_MS: [u64; 4] = [500, 1000, 1500, 2000];

/// Shared state for the Tauri runtime
pub struct TauriRuntime {
    pub ctx: AppContext,
    pub initialized: bool,
    pub quit_requested: bool,
    pub toggle_panel_requested: bool,
    pub create_box_requested: bool,
    pub ai_classify_requested: bool,
    /// 启动时快照的桌面图标位置，用于退出时恢复
    pub desktop_positions_snapshot: std::collections::HashMap<String, (i32, i32)>,
}

impl TauriRuntime {
    /// 创建尚未初始化的平台运行时。
    pub fn new() -> Self {
        let ctx = AppContext::new().unwrap_or_else(|e| {
            log::warn!("Failed to init AppContext: {}, using defaults", e);
            AppContext::default()
        });
        Self {
            ctx,
            initialized: false,
            quit_requested: false,
            toggle_panel_requested: false,
            create_box_requested: false,
            ai_classify_requested: false,
            desktop_positions_snapshot: std::collections::HashMap::new(),
        }
    }

    /// Platform initialization
    pub fn initialize(&mut self, app: &tauri::AppHandle) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        log::info!("[TauriRuntime] Initialization started");

        // 0. 开机自启同步（必须最先执行）
        //
        // 放在托盘/布局恢复等耗时或可能失败的步骤之前，确保只要进程被拉起，
        // 自启注册表状态就会被重写一遍（抵御 StartupApproved 被系统标记禁用）。
        // 配置加载失败时跳过：此时 config 是默认值（auto_start=false），
        // 若照常同步会把用户已开启的自启条目删掉。
        #[cfg(windows)]
        self.sync_auto_start();

        // Clear any stale signal files from previous runs
        let _ = std::fs::remove_file(signal_file_path());
        let _ = std::fs::remove_file(open_panel_signal_path());
        let _ = std::fs::remove_file(ai_classify_signal_path());
        let _ = std::fs::remove_file(quit_signal_path());
        log::info!("[TauriRuntime] Cleared stale signal files");
        // 1. System tray
        if let Err(e) = setup_system_tray() {
            log::warn!("[TauriRuntime] System tray setup failed: {}", e);
        } else {
            log::info!("[TauriRuntime] System tray initialized");
        }

        // 2. Restore overlay layout
        crate::tauri_commands::config_cmd::apply_runtime_config(&self.ctx.config);
        // 先恢复并整理桌面图标状态，再创建分类框，确保液态玻璃首个快照不含残留图标层。
        crate::platform::restore_all_hidden_icons();
        self.snapshot_desktop_positions();
        self.restore_layout(app);

        // 3. Desktop icon diagnostics
        #[cfg(windows)]
        crate::platform::windows::icon_manager::diagnose();

        // 4. Desktop right-click registry
        cleanup_desktop_registry();
        setup_desktop_registry();
        log::info!("[TauriRuntime] Desktop registry initialized");

        // 6. Global hotkey
        let hotkey_str = self.ctx.config.hotkeys.toggle_panel.clone();
        if let Err(e) = setup_global_hotkey(&hotkey_str) {
            log::warn!(
                "[TauriRuntime] Global hotkey failed: {} (hotkey: {})",
                e,
                hotkey_str
            );
        } else {
            log::info!("[TauriRuntime] Global hotkey: {}", hotkey_str);
        }

        log::info!("[TauriRuntime] Initialization complete");
    }

    /// 开机自启同步（含诊断与重试）。
    ///
    /// 1. 先输出当前注册表状态诊断（Run 值原文、是否指向当前 exe、
    ///    StartupApproved 首字节），便于在用户机器上定位"时灵时不灵"问题；
    /// 2. 配置加载失败时直接跳过（不能按默认值删除用户已开启的条目）；
    /// 3. 同步失败最多重试 3 次，每次失败都记日志。
    ///
    /// 注册表写入统一走 `utils::platform::set_auto_start`（开机自启唯一写入口），
    /// 运行时层不直连注册表 API。
    #[cfg(windows)]
    fn sync_auto_start(&mut self) {
        crate::utils::platform::diagnose_auto_start_state();

        if !self.ctx.config_loaded {
            log::warn!(
                "[TauriRuntime] 配置未成功加载，跳过开机自启同步（避免按默认值删除注册表条目）"
            );
            return;
        }

        let auto_start = self.ctx.config.general.auto_start;
        set_tray_auto_start_state(auto_start);

        for attempt in 1..=3u32 {
            match crate::utils::platform::set_auto_start(auto_start) {
                Ok(()) => {
                    log::info!(
                        "[TauriRuntime] 开机自启同步完成: auto_start={auto_start}（第 {attempt} 次尝试）"
                    );
                    return;
                }
                Err(error) => {
                    log::warn!("[TauriRuntime] 开机自启同步失败（第 {attempt}/3 次）: {error:#}");
                    std::thread::sleep(std::time::Duration::from_millis(500 * attempt as u64));
                }
            }
        }
        log::error!("[TauriRuntime] 开机自启同步连续 3 次失败，本次启动放弃");
    }

    /// 用新配置替换运行时配置。
    pub fn replace_config(&mut self, config: crate::config::AppConfig) {
        self.ctx.config = config;
    }

    /// 快照当前桌面图标位置（用于退出时恢复）
    fn snapshot_desktop_positions(&mut self) {
        log::info!("[TauriRuntime] 开始快照桌面图标位置");
        self.desktop_positions_snapshot =
            crate::platform::windows::icon_manager::snapshot_all_desktop_positions();
        log::info!(
            "[TauriRuntime] 快照完成，记录了 {} 个图标位置",
            self.desktop_positions_snapshot.len()
        );
    }

    /// 恢复仓库图标到快照的桌面位置
    fn restore_desktop_positions(&self, vault_display_names: &std::collections::HashSet<String>) {
        log::info!("[TauriRuntime] 开始恢复桌面图标位置");

        let positions = &self.desktop_positions_snapshot;
        if positions.is_empty() {
            log::info!("[TauriRuntime] 没有快照位置，跳过恢复");
            return;
        }

        let mut success_count: u32 = 0;
        let mut fail_count: u32 = 0;
        let mut pending: Vec<(String, i32, i32)> = Vec::new();

        // 第一次尝试恢复
        for (name, &(x, y)) in positions {
            let name_lower = name.to_lowercase();
            if vault_display_names.contains(&name_lower) {
                match crate::platform::windows::icon_manager::set_icon_position_by_name(name, x, y)
                {
                    Ok(()) => {
                        success_count += 1;
                        log::debug!("[TauriRuntime] 恢复图标 '{}' 位置成功", name);
                    }
                    Err(e) => {
                        let msg = format!("{}", e);
                        if msg.contains("未找到图标") {
                            log::info!(
                                "[TauriRuntime] 图标 '{}' 尚未出现在 SysListView32，加入重试队列",
                                name
                            );
                            pending.push((name.clone(), x, y));
                        } else {
                            log::warn!("[TauriRuntime] 恢复图标 '{}' 位置失败: {e}", name);
                            fail_count += 1;
                        }
                    }
                }
            }
        }

        // 重试队列：递增延迟重试未找到的图标
        let retry_delays = DESKTOP_RESTORE_RETRY_DELAYS_MS;
        for (attempt, &delay_ms) in retry_delays.iter().enumerate() {
            if pending.is_empty() {
                break;
            }
            log::info!(
                "[TauriRuntime] 等待 {}ms 后重试（第 {} 次），剩余 {} 个图标未找到",
                delay_ms,
                attempt + 1,
                pending.len()
            );
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));

            let mut still_pending = Vec::new();
            for (name, x, y) in &pending {
                match crate::platform::windows::icon_manager::set_icon_position_by_name(
                    name, *x, *y,
                ) {
                    Ok(()) => {
                        success_count += 1;
                        log::info!("[TauriRuntime] 重试成功: '{}' 已找到并恢复位置", name);
                    }
                    Err(e) => {
                        let msg = format!("{}", e);
                        if msg.contains("未找到图标") {
                            still_pending.push((name.clone(), *x, *y));
                        } else {
                            log::debug!("[TauriRuntime] 重试时恢复图标位置失败: '{}': {}", name, e);
                        }
                    }
                }
            }
            pending = still_pending;
        }

        if !pending.is_empty() {
            log::warn!(
                "[TauriRuntime] 以下 {} 个图标始终未在 SysListView32 中找到，无法恢复位置: {}",
                pending.len(),
                pending
                    .iter()
                    .map(|(n, _, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        log::info!(
            "[TauriRuntime] 桌面图标位置恢复完成: 成功 {}, 失败 {}",
            success_count,
            fail_count
        );
    }

    fn request_create_box(&mut self) -> bool {
        self.create_box_requested = true;
        true
    }

    fn request_auto_classify(&mut self) -> bool {
        self.ai_classify_requested = true;
        true
    }

    /// Restore category box overlay windows from saved layout
    fn restore_layout(&mut self, app: &tauri::AppHandle) {
        if !self.ctx.config.general.restore_layout_on_start {
            log::info!("[TauriRuntime] Layout restore disabled");
            return;
        }

        let boxes = self.ctx.config.layout.boxes.clone();
        if boxes.is_empty() {
            log::info!("[TauriRuntime] No saved layout to restore");
            return;
        }

        log::info!("[TauriRuntime] Restoring {} category boxes", boxes.len());

        #[cfg(windows)]
        for box_data in &boxes {
            for icon_path in &box_data.icon_names {
                if let Err(error) =
                    crate::platform::windows::icon_hider::hide_single_icon(icon_path)
                {
                    log::debug!("恢复布局时隐藏图标失败 '{}': {error}", icon_path);
                }
            }
        }

        let mut created = 0;
        for (i, box_data) in boxes.iter().enumerate() {
            let mut items: Vec<GridFileItem> = Vec::new();
            for (idx, file_path) in box_data.icon_names.iter().enumerate() {
                // 跳过已删除的图标
                if !icon_exists_for_box(file_path) {
                    log::info!("[TauriRuntime] Skipping deleted icon: {}", file_path);
                    continue;
                }

                // Resolve actual file path for icon extraction
                let read_path = resolve_icon_read_path(file_path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.clone());

                // 提取文件图标
                let hicon = extract_file_icon(&read_path)
                    .map(|fi| fi.hicon)
                    .unwrap_or(std::ptr::null_mut());

                // Restore display name
                let display_name = box_data
                    .display_names
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| file_path.clone());

                // Restore grid position
                let (grid_col, grid_row) = if idx < box_data.grid_positions.len() {
                    (
                        box_data.grid_positions[idx][0],
                        box_data.grid_positions[idx][1],
                    )
                } else {
                    (-1, -1)
                };

                items.push(GridFileItem {
                    file_path: file_path.clone(),
                    display_name,
                    icon_handle: hicon,
                    grid_col,
                    grid_row,
                });
            }

            let params = GridCreateParams {
                id: i,
                title: box_data.name.clone(),
                x: box_data.position[0] as i32,
                y: box_data.position[1] as i32,
                width: box_data.size[0] as i32,
                height: box_data.size[1] as i32,
                items,
                view_mode: box_data.view_mode.clone(),
                locked: box_data.locked,
                sort_mode: box_data.sort_mode.clone(),
                sort_ascending: box_data.sort_ascending,
                group_by: box_data.group_by.clone(),
                config_id: box_data.id,
                edit_box_callback: Some(crate::tauri_commands::box_cmd::edit_box_callback(
                    app.clone(),
                )),
            };

            match create_grid_window(params) {
                Ok(_) => {
                    created += 1;
                    log::info!("[TauriRuntime] Restored box: {}", box_data.name);
                }
                Err(e) => log::warn!(
                    "[TauriRuntime] Failed to restore '{}': {}",
                    box_data.name,
                    e
                ),
            }
        }

        log::info!("[TauriRuntime] Restored {}/{} boxes", created, boxes.len());

        // 诊断开关：IKUN_DEBUG_RECREATE=1 时，启动恢复后额外做一轮
        // 「销毁全部 + 重建」，用于实证 DComp 旧视觉残影是否还会出现。
        // 默认关闭；开启时也只在启动阶段多一轮重建，不影响功能。
        #[cfg(windows)]
        if std::env::var("IKUN_DEBUG_RECREATE")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            static RECREATE_ONCE: std::sync::Once = std::sync::Once::new();
            if !RECREATE_ONCE.is_completed() {
                RECREATE_ONCE.call_once(|| {});
                log::warn!("[diag] IKUN_DEBUG_RECREATE=1: 额外执行一轮销毁全部 + 重建");
                destroy_all_grid_windows();
                self.restore_layout(app);
            }
        }
    }

    /// Poll tick - returns (changed, quit)
    pub fn tick(&mut self) -> (bool, bool) {
        let mut changed = false;

        // 0. 玻璃引擎事件 → 项目日志系统（低频轮询消费，事件本身只在
        //    启动/显示器变化/采集失效等场景出现，不属于热路径）。
        //    同时驱动内存分解周期日志（原挂在渲染 tick 上，现随渲染定时器退役）。
        #[cfg(windows)]
        {
            crate::platform::windows::glass_engine::poll_events_to_log();
            // 显示器热插拔可能杀死库渲染线程（0.5.0 已知缺陷）：每周期检查并
            // 立即复活引擎，避免用户看到玻璃永久消失退回旧渲染路径。
            crate::platform::windows::glass_engine::revive_if_dead(&self.ctx.config);
            // 画布主循环唤醒保险丝：0.5.1 已在库侧内建同构 100ms 定时器（#3），
            // 这里按库侧指南降频保留为双保险（只 PostMessage(WM_NULL)，零阻塞）。
            crate::platform::windows::glass_engine::wake_canvas_idle();
            // 块状态诊断快照（debug 级，1 s 一条）：定位「某个框点不动」的唯一硬证据。
            crate::platform::windows::glass_engine::debug_log_blocks();
            // 画布层级兜底（判据 + 120ms 防抖）：0.5.2 起库的 set_window_level 等值
            // 也会重发层级，且画布显示与层级同一次 SetWindowPos 完成，这里只需
            // 保留 O(1) 位次复查作为启动后的持续兜底，不再需要 4ms 守护线程。
            crate::platform::windows::glass_engine::reassert_canvas_bottom();
            if crate::platform::windows::mem_probe::due() {
                crate::platform::windows::mem_probe::log_breakdown();
            }
        }

        // 1. Tray actions
        if let Some(action) = take_pending_tray_action() {
            match action {
                SystemTrayAction::OpenControlPanel => {
                    self.toggle_panel_requested = true;
                    changed = true;
                }
                SystemTrayAction::CreateBox => {
                    changed |= self.request_create_box();
                }
                SystemTrayAction::Quit => {
                    self.quit_requested = true;
                    return (true, true);
                }
                SystemTrayAction::AiClassify => {
                    log::info!("[TauriRuntime] 收到系统托盘一键分类请求");
                    changed |= self.request_auto_classify();
                }
                SystemTrayAction::ToggleAutoStart => {
                    let mut config = self.ctx.config.clone();
                    let enabled = !config.general.auto_start;
                    config.general.auto_start = enabled;
                    if let Err(error) = config.save() {
                        log::warn!("保存开机自启配置失败: {error}");
                    }
                    self.replace_config(config);
                    // 通过 utils::platform 写入/删除自启条目
                    if let Err(error) = crate::utils::platform::set_auto_start(enabled) {
                        log::warn!("切换开机自启失败: {error}");
                    }
                    set_tray_auto_start_state(enabled);
                    changed = true;
                }
            }
        }

        // 2. Global hotkey
        if take_hotkey_toggle() {
            self.toggle_panel_requested = true;
            changed = true;
        }

        // 3. Pending create box
        if take_pending_create_box() {
            changed |= self.request_create_box();
        }

        // 4. Second-instance signal files reuse the same request path.
        if signal_file_path().exists() {
            let _ = std::fs::remove_file(signal_file_path());
            changed |= self.request_create_box();
        }
        if open_panel_signal_path().exists() {
            let _ = std::fs::remove_file(open_panel_signal_path());
            self.toggle_panel_requested = true;
            changed = true;
        }
        if ai_classify_signal_path().exists() {
            let _ = std::fs::remove_file(ai_classify_signal_path());
            changed |= self.request_auto_classify();
        }
        if quit_signal_path().exists() {
            let _ = std::fs::remove_file(quit_signal_path());
            self.quit_requested = true;
            return (true, true);
        }

        (changed, self.quit_requested)
    }
}

impl Default for TauriRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// 创建供 Tauri 状态管理器和后台线程共同持有的运行时。
pub fn new_shared_runtime() -> SharedTauriRuntime {
    Arc::new(Mutex::new(TauriRuntime::new()))
}

// Signal file path helpers
fn signal_file_path() -> std::path::PathBuf {
    std::env::temp_dir().join("ikun_create_box.signal")
}
fn open_panel_signal_path() -> std::path::PathBuf {
    std::env::temp_dir().join("ikun_open_panel.signal")
}
fn ai_classify_signal_path() -> std::path::PathBuf {
    std::env::temp_dir().join("ikun_ai_classify.signal")
}
fn quit_signal_path() -> std::path::PathBuf {
    std::env::temp_dir().join("ikun_quit.signal")
}

/// Start the platform runtime in Tauri's setup hook.
/// 正常退出是否已走通（`run_tauri_app` 的 Tauri 事件循环返回时置位）。
///
/// 退出看门狗靠它决定要不要补刀：正常退出走通就什么都不做，避免用
/// TerminateProcess 打断别人正在跑的收尾。
static TEARDOWN_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 由 `main.rs` 在 Tauri 事件循环返回后调用。
pub fn mark_teardown_done() {
    TEARDOWN_DONE.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// 立即结束进程，不跑任何 CRT / COM 收尾（清理完成后的正常收尾路径）。
fn force_finish() {
    use crate::platform::windows::ffi_imports::{GetCurrentProcess, TerminateProcess};
    unsafe {
        TerminateProcess(GetCurrentProcess(), 0);
    }
}

pub fn start_platform_runtime(app: tauri::AppHandle, runtime: SharedTauriRuntime) {
    // Initialize platform
    match runtime.lock() {
        Ok(mut runtime) => runtime.initialize(&app),
        Err(error) => {
            log::error!("[TauriRuntime] 无法获取初始化锁: {error}");
            return;
        }
    }

    // Background poll thread
    let app_handle = app.clone();
    let runtime_for_poll = runtime.clone();
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    std::thread::spawn(move || {
        log::info!("[TauriRuntime] Poll thread started");

        while running_clone.load(Ordering::Relaxed) {
            let (changed, quit) = {
                match runtime_for_poll.lock() {
                    Ok(mut rt) => rt.tick(),
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }
                }
            };

            if quit {
                log::info!("[TauriRuntime] Quit requested");
                running_clone.store(false, Ordering::Relaxed);
                break;
            }

            if changed {
                let requests = match runtime_for_poll.lock() {
                    Ok(mut runtime) => {
                        let requests = (
                            runtime.toggle_panel_requested,
                            runtime.create_box_requested,
                            runtime.ai_classify_requested,
                        );
                        runtime.toggle_panel_requested = false;
                        runtime.create_box_requested = false;
                        runtime.ai_classify_requested = false;
                        Some(requests)
                    }
                    Err(error) => {
                        log::warn!("[TauriRuntime] 无法读取待处理请求: {error}");
                        None
                    }
                };

                let Some((toggle_panel, create_box, ai_classify)) = requests else {
                    continue;
                };

                if toggle_panel {
                    if let Err(error) =
                        toggle_managed_window(&app_handle, ManagedWindow::ControlPanel)
                    {
                        log::warn!("切换控制面板失败: {error}");
                    }
                }

                if create_box {
                    if let Err(error) = show_managed_window(&app_handle, ManagedWindow::CreateBox) {
                        log::warn!("打开创建分类框窗口失败: {error}");
                    }
                }

                if ai_classify {
                    log::info!("[TauriRuntime] 显示 AI 分类确认对话框");
                    if let Err(error) = show_managed_window(&app_handle, ManagedWindow::AiClassify)
                    {
                        log::warn!("显示 AI 分类对话框失败: {error}");
                    }
                }
            }

            // WebView 挂死看门狗（心跳断流 + 未被遮挡 + 可见 → 重建窗口）。
            crate::tauri_commands::window_cmd::watch_webview_hangs(&app_handle);

            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // 退出前保存分类框布局
        if let Ok(mut runtime) = runtime_for_poll.lock() {
            runtime.ctx.config.layout.boxes = crate::platform::collect_grid_layout();
            if let Err(error) = runtime.ctx.config.save() {
                log::warn!("退出前保存分类框布局失败: {error}");
            }
        }

        // 销毁分类框窗口（必须在主线程中执行）
        log::info!("[TauriRuntime] 在主线程中销毁分类框窗口");
        let app_handle_for_destroy = app_handle.clone();
        let _ = app_handle_for_destroy.run_on_main_thread(move || {
            log::info!("[TauriRuntime] 主线程: 开始销毁分类框窗口");
            destroy_all_grid_windows();
            // 玻璃引擎在所有内容窗口解除登记后统一关闭（渲染线程 join）
            crate::platform::windows::glass_engine::shutdown();
            log::info!("[TauriRuntime] 主线程: 分类框窗口已销毁");
        });
        // 等待主线程完成销毁
        std::thread::sleep(std::time::Duration::from_millis(200));

        // 清理系统托盘和注册表
        cleanup_system_tray();
        cleanup_desktop_registry();

        // 恢复桌面图标：获取仓库中的图标显示名称
        let vault_display_names = crate::platform::windows::icon_hider::get_vault_display_names();
        log::info!(
            "[TauriRuntime] 仓库图标显示名称: {} 个，准备恢复这些图标的位置",
            vault_display_names.len()
        );

        // 恢复所有隐藏的图标到桌面
        crate::platform::restore_all_hidden_icons();

        // 等待 Windows 处理图标还原
        std::thread::sleep(std::time::Duration::from_millis(500));

        // 恢复图标到快照的位置
        if let Ok(runtime) = runtime_for_poll.lock() {
            runtime.restore_desktop_positions(&vault_display_names);
        }

        log::info!("[TauriRuntime] Cleanup complete");

        // 退出 Tauri 应用
        log::info!("[TauriRuntime] 调用 app_handle.exit(0)");
        // 退出看门狗：清理若卡在某个等待（模态循环/跨线程同步），最多等 3 秒后
        // 强制结束进程 —— 尸尸进程会长期占着 DXGI 桌面复制，导致后续所有实例
        // 玻璃捕获被拒绝、退化半透明白背景且拖拽不跟随（2026-09-16 实测案发现）。
        //
        // 为什么用 TerminateProcess 而不是 `std::process::exit(0)`：后者会跑 CRT
        // atexit / DllMain 收尾，而此刻 WebView2 / D2D / DComp 线程还在飞 COM 调用，
        // 两者对撞就崩在 KERNELBASE。`TerminateProcess` 不走任何收尾，进程立即消失，
        // 既无崩溃也无弹窗；有意义的清理在上面已经全部做完。
        std::thread::spawn(|| {
            for _ in 0..30 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if TEARDOWN_DONE.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
            }
            log::warn!("[TauriRuntime] 3s 内退出未走通，看门狗强制结束进程");
            force_finish();
        });
        app_handle.exit(0);
        // 走到这里，业务清理（分类框窗口销毁、玻璃引擎关闭、托盘注销、图标还原、
        // 配置落盘）已经全部完成。此后**不能**把收尾交给 Tauri/WebView2：实测
        // `app_handle.exit(0)` 之后约 1s 稳定记一次 APPCRASH（KERNELBASE.dll，
        // 异常码 0x87A，2026-10-26~10-28 的 11 次退出全部命中），而且 Tauri 事件
        // 循环从不返回（`.run()` 之后的日志一次都没出现过）。所以留一小段时间
        // 让日志与落盘写完，再主动结束进程 —— 用户侧不再看到「退出即崩、
        // 分类框消失」，WER 里也不再堆崩溃报告。
        std::thread::sleep(std::time::Duration::from_millis(400));
        mark_teardown_done();
        log::info!("[TauriRuntime] 清理完成，主动结束进程（跳过 Tauri/WebView2 收尾）");
        force_finish();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 配置加载失败（config_loaded=false）时，sync_auto_start 必须跳过
    /// 注册表同步——否则回退的默认值（auto_start=false）会删掉用户
    /// 已开启的自启条目（自毁路径）。
    #[cfg(windows)]
    #[test]
    fn sync_auto_start_skips_when_config_not_loaded() {
        use crate::utils::platform::test_support as ts;

        let _guard = ts::registry_test_lock();
        let original = ts::read_run_value();

        let mut runtime = TauriRuntime::new();
        // 构造"配置加载失败"场景：标记未加载，并把默认配置设为关闭自启
        runtime.ctx.config_loaded = false;
        runtime.ctx.config.general.auto_start = false;

        runtime.sync_auto_start();

        // 同步被跳过：Run 键应保持测试前状态，不得被按默认值删除
        assert_eq!(
            ts::read_run_value(),
            original,
            "config_loaded=false 时不得改动注册表 Run 键"
        );

        ts::restore_run_value(original.as_deref());
    }

    #[test]
    fn replace_config_updates_the_shared_context() {
        let mut runtime = TauriRuntime::new();
        let mut config = runtime.ctx.config.clone();
        config.ai.model = "architecture-test-model".to_string();
        config.classification.clear();

        runtime.replace_config(config);

        assert_eq!(runtime.ctx.config.ai.model, "architecture-test-model");
        assert!(runtime.ctx.config.classification.is_empty());
    }
}
