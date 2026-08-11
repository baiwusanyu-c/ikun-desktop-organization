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
use crate::plugin::{builtin::register_builtin_plugins, PluginRegistry};
use crate::tauri_commands::window_cmd::{
    show_managed_window, toggle_managed_window, ManagedWindow,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

/// Tauri Commands 与平台轮询线程共享的唯一运行时实例。
pub type SharedTauriRuntime = Arc<Mutex<TauriRuntime>>;

/// Shared state for the Tauri runtime
pub struct TauriRuntime {
    pub ctx: AppContext,
    pub plugins: PluginRegistry,
    pub initialized: bool,
    pub quit_requested: bool,
    pub toggle_panel_requested: bool,
    pub create_box_requested: bool,
    pub ai_classify_requested: bool,
    /// 启动时快照的桌面图标位置，用于退出时恢复
    pub desktop_positions_snapshot: std::collections::HashMap<String, (i32, i32)>,
}

impl TauriRuntime {
    /// 创建尚未初始化的平台运行时，并注册内置插件。
    pub fn new() -> Self {
        let ctx = AppContext::new().unwrap_or_else(|e| {
            log::warn!("Failed to init AppContext: {}, using defaults", e);
            AppContext::default()
        });
        let mut plugins = PluginRegistry::new();
        if let Err(error) = register_builtin_plugins(&mut plugins) {
            log::error!("注册内置插件失败: {error:#}");
        }
        Self {
            ctx,
            plugins,
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


          // Clear any stale signal files from previous runs
          let _ = std::fs::remove_file(signal_file_path());
          let _ = std::fs::remove_file(open_panel_signal_path());
          let _ = std::fs::remove_file(ai_classify_signal_path());
          let _ = std::fs::remove_file(quit_signal_path());
          log::info!("[TauriRuntime] Cleared stale signal files");
        // 1. In-process plugins
        let plugin_report = self.plugins.init_all(&mut self.ctx);
        log::info!(
            "[TauriRuntime] Plugins initialized: {}, failed: {}, menu items: {}",
            plugin_report.initialized.len(),
            plugin_report.failed.len(),
            self.plugins.all_menu_items().len()
        );

        // 2. System tray
        if let Err(e) = setup_system_tray() {
            log::warn!("[TauriRuntime] System tray setup failed: {}", e);
        } else {
            log::info!("[TauriRuntime] System tray initialized");
        }

        // 3. Restore overlay layout
        crate::tauri_commands::config_cmd::apply_runtime_config(&self.ctx.config);
        // 先恢复并整理桌面图标状态，再创建分类框，确保液态玻璃首个快照不含残留图标层。
        crate::platform::restore_all_hidden_icons();
        self.snapshot_desktop_positions();
        self.restore_layout(app);

        // 4. Desktop icon diagnostics
        #[cfg(windows)]
        crate::platform::windows::icon_manager::diagnose();

        // 5. Desktop right-click registry
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

        // 7. Auto-start sync
        #[cfg(windows)]
        {
            let auto_start = self.ctx.config.general.auto_start;
            set_tray_auto_start_state(auto_start);
            let _ = crate::utils::platform::set_auto_start(auto_start);
        }

        log::info!("[TauriRuntime] Initialization complete");
    }

    /// 用新配置替换运行时配置，并同步依赖配置构建的分类器。
    pub fn replace_config(&mut self, config: crate::config::AppConfig) {
        self.ctx.classifier =
            crate::classification::AppClassifier::from_config(&config.classification);
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
                match crate::platform::windows::icon_manager::set_icon_position_by_name(name, x, y) {
                    Ok(()) => {
                        success_count += 1;
                        log::debug!("[TauriRuntime] 恢复图标 '{}' 位置成功", name);
                    }
                    Err(e) => {
                        let msg = format!("{}", e);
                        if msg.contains("未找到图标") {
                            log::info!("[TauriRuntime] 图标 '{}' 尚未出现在 SysListView32，加入重试队列", name);
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
        let retry_delays = [500u64, 1000, 1500, 2000];
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
                match crate::platform::windows::icon_manager::set_icon_position_by_name(name, *x, *y) {
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

    fn dispatch_plugin_command(&mut self, command: &str) -> bool {
        match self.plugins.dispatch_command(command, &mut self.ctx) {
            Ok(true) => true,
            Ok(false) => {
                log::warn!("没有已初始化的插件处理命令: {command}");
                false
            }
            Err(error) => {
                log::warn!("插件命令分发失败 '{command}': {error:#}");
                false
            }
        }
    }

    fn request_create_box(&mut self) -> bool {
        if !self.dispatch_plugin_command("box.create") {
            return false;
        }
        self.create_box_requested = true;
        true
    }

    fn request_auto_classify(&mut self) -> bool {
        if !self.dispatch_plugin_command("classify.auto") {
            return false;
        }
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
                // Skip deleted icons (same logic as GPUI version)
                if !icon_exists_for_box(file_path) {
                    log::info!("[TauriRuntime] Skipping deleted icon: {}", file_path);
                    continue;
                }

                // Resolve actual file path for icon extraction
                let read_path = resolve_icon_read_path(file_path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.clone());

                // Extract file icon (same as GPUI version)
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
    }

    /// Poll tick - returns (changed, quit)
    pub fn tick(&mut self) -> (bool, bool) {
        let mut changed = false;

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
                    log::info!("Tauri 运行时收到一键分类请求");
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

        // 4. Second-instance signal files use the same plugin-gated request path.
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

    /// 使用现有分类规则重建原生分类框，保持托盘的一键分类行为。
    fn perform_auto_classify(&mut self, app: &tauri::AppHandle) {
        let icons = self.ctx.desktop_manager.get_desktop_icons();
        if icons.is_empty() {
            log::info!("一键分类跳过：未找到桌面图标");
            return;
        }

        #[cfg(windows)]
        {
            use crate::platform::{
                create_grid_window, extract_file_icon, get_primary_monitor_dpi,
                get_screen_resolution, GridCreateParams, GridFileItem,
            };
            let mut groups: [(&str, Vec<crate::platform::DesktopIcon>); 3] = [
                ("游戏", Vec::new()),
                ("开发工具", Vec::new()),
                ("办公", Vec::new()),
            ];
            for icon in icons.iter().cloned() {
                match self.ctx.classifier.classify(&icon) {
                    Some(category) if category.id == "games" => groups[0].1.push(icon),
                    Some(category) if category.id == "dev" => groups[1].1.push(icon),
                    _ => groups[2].1.push(icon),
                }
            }
            for icon in &icons {
                crate::platform::windows::icon_hider::hide_single_icon(
                    &icon.path.to_string_lossy(),
                )
                .ok();
            }
            destroy_all_grid_windows();
            let (screen_w_phys, screen_h_phys) = get_screen_resolution();
            let dpi = get_primary_monitor_dpi().max(96);
            let screen_w = screen_w_phys * 96 / dpi as i32;
            let screen_h = screen_h_phys * 96 / dpi as i32;
            let box_w = self.ctx.config.box_defaults.default_width;
            let box_h = self.ctx.config.box_defaults.default_height;
            let gap = 20;
            let start_x = ((screen_w - 3 * box_w - 2 * gap) / 2).max(10);
            let start_y = (screen_h as f32 * 0.08) as i32;
            let mut created = 0;
            for (index, (title, group)) in groups.iter().enumerate() {
                if group.is_empty() {
                    continue;
                }
                let items = group
                    .iter()
                    .map(|icon| {
                        let path = icon.path.to_string_lossy().to_string();
                        let hicon = extract_file_icon(&path)
                            .map(|info| info.hicon)
                            .unwrap_or(std::ptr::null_mut());
                        GridFileItem {
                            file_path: path,
                            display_name: icon.display_name.clone(),
                            icon_handle: hicon,
                            grid_col: -1,
                            grid_row: -1,
                        }
                    })
                    .collect();
                let params = GridCreateParams {
                    id: index,
                    title: (*title).to_string(),
                    x: start_x + created * (box_w + gap),
                    y: start_y,
                    width: box_w,
                    height: box_h,
                    items,
                    view_mode: self.ctx.config.box_defaults.view_mode.clone(),
                    locked: false,
                    sort_mode: self.ctx.config.box_defaults.sort_mode.clone(),
                    sort_ascending: self.ctx.config.box_defaults.sort_ascending,
                    group_by: self.ctx.config.box_defaults.group_by.clone(),
                    config_id: uuid::Uuid::new_v4(),
                    edit_box_callback: Some(crate::tauri_commands::box_cmd::edit_box_callback(
                        app.clone(),
                    )),
                };
                if create_grid_window(params).is_ok() {
                    created += 1;
                }
            }
            self.ctx.config.layout.boxes = crate::platform::collect_grid_layout();
            if let Err(error) = self.ctx.config.save() {
                log::warn!("一键分类后保存布局失败: {error}");
            }
            log::info!(
                "一键分类完成: {} 个图标 -> {} 个分类框",
                icons.len(),
                created
            );
        }
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
                    if let Ok(mut rt) = runtime_for_poll.lock() {
                        rt.perform_auto_classify(&app_handle);
                    }
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // 先保存布局并清理插件
        if let Ok(mut runtime) = runtime_for_poll.lock() {
            runtime.ctx.config.layout.boxes = crate::platform::collect_grid_layout();
            if let Err(error) = runtime.ctx.config.save() {
                log::warn!("退出前保存分类框布局失败: {error}");
            }
            let TauriRuntime { ctx, plugins, .. } = &mut *runtime;
            plugins.cleanup_all(ctx);
        }
        
        // 销毁分类框窗口（必须在主线程中执行）
        log::info!("[TauriRuntime] 在主线程中销毁分类框窗口");
        let app_handle_for_destroy = app_handle.clone();
        let _ = app_handle_for_destroy.run_on_main_thread(move || {
            log::info!("[TauriRuntime] 主线程: 开始销毁分类框窗口");
            destroy_all_grid_windows();
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
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_runtime_registers_builtin_plugins_before_initialization() {
        let runtime = TauriRuntime::new();
        assert!(runtime.plugins.contains("builtin.classification"));
        assert!(runtime.plugins.contains("builtin.style"));
        assert!(!runtime.plugins.is_initialized("builtin.classification"));
        assert!(!runtime.plugins.is_initialized("builtin.style"));
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

    #[test]
    fn unplugged_classification_plugin_blocks_owned_runtime_actions() {
        let mut runtime = TauriRuntime::new();
        runtime.plugins.init_all(&mut runtime.ctx);

        assert!(runtime.request_create_box());
        assert!(runtime.create_box_requested);
        runtime.create_box_requested = false;

        assert!(runtime
            .plugins
            .uninstall("builtin.classification", &mut runtime.ctx));
        assert!(!runtime.request_create_box());
        assert!(!runtime.create_box_requested);
        assert!(!runtime.request_auto_classify());
        assert!(!runtime.ai_classify_requested);
    }
}

