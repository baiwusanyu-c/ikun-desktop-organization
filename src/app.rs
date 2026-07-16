//! 控制面板 - egui 主窗口
//!
//! 通过 `AppContext` 统一管理配置、平台服务和分类器。
//! 创建分类框使用独立的 egui viewport（独立窗口，非控制面板内弹窗）。
//!
//! 右键菜单触发方式：
//! - 分类框空白区域右键 → IContextMenu 编程式注入 → PENDING_CREATE_BOX 原子标志
//! - 桌面空白区域右键 → 注册表注入（程序退出时自动清理）→ --create-box → 信号文件

use crate::config::app_config::AppConfig;
use crate::context::AppContext;
use crate::platform::{
    cleanup_desktop_registry, cleanup_system_tray, collect_grid_layout, create_desktop_overlay,
    create_grid_window, destroy_all_grid_windows, destroy_desktop_overlay, extract_file_icon,
    get_cursor_monitor_work_area_phys, get_grid_count, get_primary_monitor_dpi,
    get_screen_resolution, hide_control_panel_window, hide_native_icons, icon_exists_for_box,
    native_icons_hidden, register_tray_action_waker, reposition_window_by_title,
    resolve_icon_read_path, restore_all_hidden_icons, restore_native_icons, setup_desktop_registry,
    setup_system_tray, show_control_panel_window, take_pending_create_box,
    take_pending_tray_action, DesktopIcon, DesktopManagerTrait, GridCreateParams, GridFileItem,
    SystemTrayAction,
};
use eframe::egui;
use std::sync::{Arc, Mutex};

/// 新建分类框名称最大长度（字符数）
const BOX_NAME_MAX_LEN: usize = 16;
/// 控制面板根窗口标题。
pub const CONTROL_PANEL_TITLE: &str = "Desktop Organizer 控制面板";

/// 信号文件路径（--create-box 从注册表菜单写入，运行实例轮询消费）
fn signal_file_path() -> std::path::PathBuf {
    std::env::temp_dir().join("ikun_create_box.signal")
}

/// 创建分类框对话框状态机
///
/// - `Closed`：对话框未打开，不渲染 viewport
/// - `Open`：对话框正在显示，每帧渲染 viewport
/// - `Closing`：用户关闭了对话框，需要发送 ViewportCommand::Close 并停止渲染
#[derive(Debug, Clone, Copy, PartialEq)]
enum DialogOpenState {
    Closed,
    Open,
    Closing,
}

impl Default for DialogOpenState {
    fn default() -> Self {
        DialogOpenState::Closed
    }
}

/// 创建分类框对话框的共享状态（主窗口 ↔ 独立 viewport 通信）
#[derive(Default)]
struct DialogState {
    name: String,
    /// 对话框状态机（由主窗口管理，viewport 回调只设置 confirmed/cancelled）
    open_state: DialogOpenState,
    /// 用户点击了确定（viewport 设置，主窗口读取后创建分类框）
    confirmed: bool,
    /// 用户点击了取消或关闭窗口
    cancelled: bool,
    /// Win32 目标位置（物理像素虚拟桌面坐标），打开时计算一次
    target_pos: Option<(i32, i32)>,
}

/// 主应用
pub struct MyApp {
    ctx: AppContext,
    initialized: bool,
    quit_requested: bool,
    /// 创建分类框对话框状态（Arc<Mutex> 用于跨 viewport 共享）
    dialog: Arc<Mutex<DialogState>>,
}

impl Default for MyApp {
    fn default() -> Self {
        let ctx = AppContext::new().unwrap_or_default();
        Self {
            ctx,
            initialized: false,
            quit_requested: false,
            dialog: Arc::new(Mutex::new(DialogState::default())),
        }
    }
}

impl MyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let repaint_ctx = cc.egui_ctx.clone();
        register_tray_action_waker(move || {
            repaint_ctx.request_repaint();
        });
        Self::default()
    }

    fn open_control_panel(&mut self, egui_ctx: &egui::Context) {
        log::info!("打开控制面板窗口");
        self.ctx.state.panel_visible = true;
        egui_ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize([320.0, 380.0].into()));
        egui_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        show_control_panel_window(CONTROL_PANEL_TITLE);
    }

    fn hide_control_panel(&mut self, egui_ctx: &egui::Context) {
        log::info!("隐藏控制面板窗口");
        self.ctx.state.panel_visible = false;
        egui_ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
            [-32000.0, -32000.0].into(),
        ));
        hide_control_panel_window(CONTROL_PANEL_TITLE);
    }

    fn handle_tray_action(&mut self, action: SystemTrayAction, egui_ctx: &egui::Context) {
        log::info!("消费系统托盘动作: {:?}", action);
        match action {
            SystemTrayAction::OpenControlPanel => {
                self.open_control_panel(egui_ctx);
            }
            SystemTrayAction::CreateBox => {
                log::info!("系统托盘菜单: 创建分类框");
                self.open_create_box_dialog();
            }
            SystemTrayAction::AiClassify => {
                log::info!("托盘菜单: 一键AI分类暂未实现");
            }
            SystemTrayAction::Quit => {
                log::info!("系统托盘菜单: 退出应用");
                self.quit_requested = true;
                cleanup_system_tray();
                egui_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// 初始化创建分类框对话框状态（设置 Open，计算 Win32 目标位置）
    fn open_create_box_dialog(&self) {
        // 获取鼠标所在显示器的工作区域（物理像素虚拟桌面坐标）
        let (work_x, work_y, work_w, work_h) = get_cursor_monitor_work_area_phys();
        let dlg_w = 360i32;
        let dlg_h = 200i32; // 含标题栏的估算物理像素高度
        let x = work_x + (work_w - dlg_w) / 2;
        let y = work_y + (work_h - dlg_h) / 3;
        log::info!(
            "对话框: 目标位置 物理({}, {}) 工作区({},{},{}x{})",
            x,
            y,
            work_x,
            work_y,
            work_w,
            work_h
        );

        if let Ok(mut s) = self.dialog.lock() {
            s.name.clear();
            s.open_state = DialogOpenState::Open;
            s.confirmed = false;
            s.cancelled = false;
            s.target_pos = Some((x, y));
        }
        log::info!("对话框: state=Open");
    }

    /// 每帧调用：根据状态机渲染或关闭 egui viewport
    ///
    /// 使用 `show_viewport_immediate`（同步模式）代替 `show_viewport_deferred`，
    /// 在多屏 + DPI 缩放下位置更可靠。
    fn render_create_box_dialog(&self, egui_ctx: &egui::Context) {
        let state = self
            .dialog
            .lock()
            .map(|s| s.open_state)
            .unwrap_or(DialogOpenState::Closed);

        match state {
            DialogOpenState::Closed => return,
            DialogOpenState::Closing => {
                // 不再渲染 viewport → egui 自动销毁
                log::info!("对话框: Closing → 停止渲染以销毁 viewport");
                if let Ok(mut s) = self.dialog.lock() {
                    s.open_state = DialogOpenState::Closed;
                }
                log::info!("对话框: state=Closed");
                return;
            }
            DialogOpenState::Open => {} // 正常渲染
        }

        let dialog = self.dialog.clone();

        // 读取 Win32 目标位置（打开时计算的物理像素坐标）
        let target_pos = self.dialog.lock().map(|s| s.target_pos).unwrap_or(None);
        let dlg_w = 360.0_f32;
        let dlg_h = 180.0_f32;

        egui_ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("create_box_dialog"),
            egui::ViewportBuilder::default()
                .with_title("创建分类框")
                .with_inner_size([dlg_w, dlg_h])
                .with_decorations(true)
                .with_resizable(false)
                .with_always_on_top(),
            move |ctx, _class| {
                let mut close = false;
                let mut close_reason = "";

                // 检测 X 按钮（OS 关闭请求）→ 等同取消
                let x_clicked = ctx.input(|i| i.viewport().close_requested());
                if x_clicked {
                    close = true;
                    close_reason = "X按钮";
                    if let Ok(mut s) = dialog.lock() {
                        s.cancelled = true;
                    }
                }

                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(8.0);
                    ui.heading("创建分类框");
                    ui.add_space(4.0);

                    let mut state = dialog.lock().unwrap();

                    ui.horizontal(|ui| {
                        ui.label("名称：");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut state.name)
                                .desired_width(220.0)
                                .hint_text("必填，最多16字符"),
                        );
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            close = true;
                            close_reason = "Enter";
                            state.confirmed = true;
                        }
                    });

                    let cc = state.name.chars().count();
                    let valid = !state.name.trim().is_empty() && cc <= BOX_NAME_MAX_LEN;
                    ui.horizontal(|ui| {
                        if cc > BOX_NAME_MAX_LEN {
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 80, 80),
                                format!("⚠ 最多 {} 字符（当前 {}）", BOX_NAME_MAX_LEN, cc),
                            );
                        } else if state.name.trim().is_empty() {
                            ui.colored_label(egui::Color32::GRAY, "名称不能为空");
                        } else {
                            ui.colored_label(
                                egui::Color32::from_rgb(100, 200, 100),
                                format!("✓ {}/{}", cc, BOX_NAME_MAX_LEN),
                            );
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        if ui.add_enabled(valid, egui::Button::new("✓ 确定")).clicked() {
                            close = true;
                            close_reason = "确定按钮";
                            state.confirmed = true;
                        }
                        if ui.button("✗ 取消").clicked() {
                            close = true;
                            close_reason = "取消按钮";
                            state.cancelled = true;
                        }
                    });

                    // ESC 取消
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                        close_reason = "ESC";
                        state.cancelled = true;
                    }
                });

                if close {
                    log::info!("对话框关闭: reason={}", close_reason);
                    if let Ok(mut s) = dialog.lock() {
                        s.open_state = DialogOpenState::Closing;
                    }
                }
            },
        );

        // Win32 重新定位：修正 egui 在多屏 + DPI 缩放下的定位错误
        if let Some((tx, ty)) = target_pos {
            reposition_window_by_title("创建分类框", tx, ty);
        }
    }

    fn restore_layout(&mut self) {
        if !self.ctx.config.general.restore_layout_on_start {
            log::info!("启动恢复布局已禁用，跳过桌面覆盖生命周期初始化");
            return;
        }

        // 阶段 6：覆盖窗口与应用生命周期绑定。
        // 先隐藏原生 SysListView32，再创建覆盖窗口渲染非管理图标，最后恢复分类框。
        restore_all_hidden_icons();
        self.ensure_native_icons_hidden();
        if let Err(e) = create_desktop_overlay() {
            log::warn!("创建桌面覆盖窗口失败: {}", e);
        }

        let boxes = &self.ctx.config.layout.boxes;
        if boxes.is_empty() {
            log::info!("启动布局为空，仅启用桌面覆盖窗口");
            self.ctx.set_status("桌面覆盖窗口已启用");
            return;
        }
        log::info!("开始恢复布局: {} 个分类框", boxes.len());

        destroy_all_grid_windows();

        let mut created_count = 0;
        for (i, box_data) in boxes.iter().enumerate() {
            let mut items: Vec<GridFileItem> = Vec::new();
            for (idx, file_path) in box_data.icon_names.iter().enumerate() {
                // 过滤已删除的图标：桌面或隐藏目录中存在即可
                if !icon_exists_for_box(file_path) {
                    log::info!("跳过已删除的图标: {}", file_path);
                    continue;
                }
                let read_path = resolve_icon_read_path(file_path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.clone());
                log::info!(
                    "恢复图标: file_path={}, display_names[{}]={:?}",
                    file_path,
                    idx,
                    box_data
                        .display_names
                        .get(idx)
                        .map(|s| s.as_str())
                        .unwrap_or("<missing>")
                );
                // 根据路径类型决定显示名称：
                // - CLSID 路径始终从路径本身解析（避免 display_names 数组不同步）
                // - 普通文件路径优先使用持久化的 display_name，回退到 file_stem
                let display_name = if file_path.starts_with("::{") {
                    log::info!("[restore] 检测到 CLSID 路径: {}", file_path);
                    // CLSID 路径（已知虚拟 Shell 对象）使用预定义的显示名称
                    let clsid_result = crate::platform::clsid_display_name(file_path);
                    log::info!("[restore] clsid_display_name 返回: {:?}", clsid_result);
                    clsid_result.map(|s| s.to_string()).unwrap_or_else(|| {
                        log::warn!("[restore] CLSID 路径未在已知列表中: {}", file_path);
                        file_path.clone()
                    })
                } else if idx < box_data.display_names.len()
                    && !box_data.display_names[idx].is_empty()
                {
                    // 普通文件路径：优先使用持久化的 display_name
                    box_data.display_names[idx].clone()
                } else {
                    // 回退：从文件名推导
                    std::path::Path::new(file_path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(file_path)
                        .to_string()
                };
                let hicon = extract_file_icon(&read_path)
                    .map(|fi| fi.hicon)
                    .unwrap_or(std::ptr::null_mut());
                items.push(GridFileItem {
                    file_path: file_path.clone(),
                    display_name,
                    icon_handle: hicon,
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
            };
            if create_grid_window(params).is_ok() {
                created_count += 1;
            }
        }

        self.ctx
            .set_status(format!("恢复了 {} 个分类框", created_count));
    }

    fn merge_latest_overlay_config(&mut self) {
        match AppConfig::load() {
            Ok(config) => {
                self.ctx.config.overlay = config.overlay;
            }
            Err(e) => {
                log::warn!("保存配置前读取覆盖窗口配置失败，保留内存配置: {}", e);
            }
        }
    }

    fn ensure_native_icons_hidden(&mut self) -> bool {
        if self.ctx.state.icons_hidden && native_icons_hidden() {
            return true;
        }

        if self.ctx.state.icons_hidden {
            log::warn!("原生桌面图标窗口已恢复或失效，尝试重新隐藏");
        }
        match hide_native_icons() {
            Ok(()) => {
                self.ctx.state.icons_hidden = true;
                true
            }
            Err(e) => {
                self.ctx.state.icons_hidden = false;
                self.ctx.set_status(format!("隐藏图标失败: {}", e));
                false
            }
        }
    }

    fn ensure_desktop_overlay_running(&mut self) {
        if !self.ensure_native_icons_hidden() {
            return;
        }
        if let Err(e) = create_desktop_overlay() {
            log::warn!("创建桌面覆盖窗口失败: {}", e);
        }
    }

    fn save_layout_and_config(&mut self) {
        let layout = collect_grid_layout();
        self.ctx.config.layout.boxes = layout;
        self.merge_latest_overlay_config();
        if let Err(e) = self.ctx.save_config() {
            self.ctx.set_status(format!("保存失败: {}", e));
        } else {
            self.ctx.set_status(format!(
                "已保存 {} 个分类框",
                self.ctx.config.layout.boxes.len()
            ));
        }
    }

    fn perform_auto_classify(&mut self) {
        self.ensure_desktop_overlay_running();
        let icons = self.ctx.desktop_manager.get_desktop_icons();
        if icons.is_empty() {
            self.ctx.set_status("未找到桌面图标");
            return;
        }
        let mut games: Vec<DesktopIcon> = Vec::new();
        let mut dev_tools: Vec<DesktopIcon> = Vec::new();
        let mut office: Vec<DesktopIcon> = Vec::new();
        for icon in &icons {
            match self.ctx.classifier.classify(icon) {
                Some(cat) if cat.id == "games" => games.push(icon.clone()),
                Some(cat) if cat.id == "dev" => dev_tools.push(icon.clone()),
                _ => office.push(icon.clone()),
            }
        }
        destroy_all_grid_windows();
        let categories: Vec<(&str, &Vec<DesktopIcon>)> = vec![
            ("游戏", &games),
            ("开发工具", &dev_tools),
            ("办公", &office),
        ];
        let (screen_w, screen_h) = get_screen_resolution();
        let box_w = self.ctx.config.box_defaults.default_width;
        let box_h = self.ctx.config.box_defaults.default_height;
        let padding = 20i32;
        let total_w = 3 * box_w + 2 * padding;
        let start_x = ((screen_w - total_w) / 2).max(10);
        let start_y = (screen_h as f32 * 0.08) as i32;
        let mut created_count = 0;
        for (i, (title, icon_list)) in categories.iter().enumerate() {
            if icon_list.is_empty() {
                continue;
            }
            let x = start_x + created_count * (box_w + padding);
            let mut items: Vec<GridFileItem> = Vec::new();
            for icon in icon_list.iter() {
                let path_str = icon.path.to_string_lossy().to_string();
                let hicon = extract_file_icon(&path_str)
                    .map(|fi| fi.hicon)
                    .unwrap_or(std::ptr::null_mut());
                items.push(GridFileItem {
                    file_path: path_str,
                    display_name: icon.display_name.clone(),
                    icon_handle: hicon,
                });
            }
            let params = GridCreateParams {
                id: i,
                title: title.to_string(),
                x,
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
            };
            if create_grid_window(params).is_ok() {
                created_count += 1;
            }
        }

        self.ctx.set_status(format!(
            "分类完成！{} 个图标 → {} 个分类框",
            icons.len(),
            created_count
        ));
    }

    fn clear_and_restore(&mut self) {
        let layout = collect_grid_layout();
        self.ctx.config.layout.boxes = layout;
        self.merge_latest_overlay_config();
        let _ = self.ctx.save_config();
        destroy_all_grid_windows();
        destroy_desktop_overlay();
        if self.ctx.state.icons_hidden {
            let _ = restore_native_icons();
            self.ctx.state.icons_hidden = false;
        }
        self.ctx.set_status("已清除，桌面图标已恢复");
    }

    fn create_empty_box(&mut self, name: String) {
        log::info!("创建分类框: '{}'", name);
        self.ensure_desktop_overlay_running();
        let (screen_w, screen_h) = get_screen_resolution();
        let box_w = self.ctx.config.box_defaults.default_width;
        let box_h = self.ctx.config.box_defaults.default_height;
        let existing = get_grid_count();
        let x = ((screen_w - box_w) / 2 + (existing as i32 * 40) % 200).max(10);
        let y = ((screen_h as f32 * 0.1) as i32 + (existing as i32 * 30) % 150).max(10);
        let params = GridCreateParams {
            id: existing,
            title: name.clone(),
            x,
            y,
            width: box_w,
            height: box_h,
            items: Vec::new(),
            view_mode: self.ctx.config.box_defaults.view_mode.clone(),
            locked: false,
            sort_mode: self.ctx.config.box_defaults.sort_mode.clone(),
            sort_ascending: self.ctx.config.box_defaults.sort_ascending,
            group_by: self.ctx.config.box_defaults.group_by.clone(),
            config_id: uuid::Uuid::new_v4(),
        };
        match create_grid_window(params) {
            Ok(_) => {
                let layout = collect_grid_layout();
                self.ctx.config.layout.boxes = layout;
                self.merge_latest_overlay_config();
                if let Err(e) = self.ctx.save_config() {
                    self.ctx.set_status(format!("创建成功但保存失败: {}", e));
                } else {
                    self.ctx.set_status(format!("已创建分类框「{}」", name));
                }
            }
            Err(e) => self.ctx.set_status(format!("创建失败: {}", e)),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.initialized {
            self.initialized = true;
            self.hide_control_panel(ctx);
            if let Err(e) = setup_system_tray() {
                log::warn!("初始化系统托盘失败: {}", e);
                self.ctx.set_status(format!("初始化系统托盘失败: {}", e));
            }
            self.restore_layout();
            // DesktopIconManager 诊断（验证 LVM 方案可行性）
            #[cfg(windows)]
            crate::platform::windows::icon_manager::diagnose();
            // 先清理旧版残留，再注册新版（退出时清理，保证无残留）
            cleanup_desktop_registry();
            setup_desktop_registry();
            self.ensure_desktop_overlay_running();
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_requested {
            log::info!("控制面板窗口关闭请求已拦截，改为隐藏窗口");
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_control_panel(ctx);
        }

        if let Some(action) = take_pending_tray_action() {
            self.handle_tray_action(action, ctx);
        }

        if should_retry_native_icon_hide(self.ctx.state.icons_hidden, native_icons_hidden()) {
            log::warn!("检测到 Explorer 桌面图标窗口恢复，重新隐藏并保持覆盖窗口接管");
            self.ensure_desktop_overlay_running();
        }

        // 信号文件（桌面右键菜单注册表项触发新实例写入）→ 执行对应操作
        let signal_path = signal_file_path();
        if signal_path.exists() {
            let _ = std::fs::remove_file(&signal_path);
            self.open_create_box_dialog();
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        let open_panel_signal = std::env::temp_dir().join("ikun_open_panel.signal");
        if open_panel_signal.exists() {
            let _ = std::fs::remove_file(&open_panel_signal);
            self.open_control_panel(ctx);
        }

        let ai_classify_signal = std::env::temp_dir().join("ikun_ai_classify.signal");
        if ai_classify_signal.exists() {
            let _ = std::fs::remove_file(&ai_classify_signal);
            log::info!("桌面右键菜单: 一键AI分类暂未实现");
        }

        let quit_signal = std::env::temp_dir().join("ikun_quit.signal");
        if quit_signal.exists() {
            let _ = std::fs::remove_file(&quit_signal);
            log::info!("桌面右键菜单: 退出应用");
            self.quit_requested = true;
            cleanup_system_tray();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // 格子窗口右键菜单 → 打开对话框
        if take_pending_create_box() {
            self.open_create_box_dialog();
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // 每帧渲染对话框 viewport（egui 要求每帧调用 show_viewport_deferred 才能保持打开）
        self.render_create_box_dialog(ctx);

        // 检查对话框结果
        let confirmed_name = if let Ok(mut state) = self.dialog.lock() {
            if state.confirmed {
                let name = state.name.trim().to_string();
                state.confirmed = false;
                if !name.is_empty() && name.chars().count() <= BOX_NAME_MAX_LEN {
                    Some(name)
                } else {
                    None
                }
            } else {
                if state.cancelled {
                    state.cancelled = false;
                }
                None
            }
        } else {
            None
        };
        // 锁已释放，可以安全调用 create_empty_box
        if let Some(name) = confirmed_name {
            self.create_empty_box(name);
        }

        // ===== 控制面板主界面 =====
        if self.ctx.state.panel_visible {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("📦 Desktop Organizer");
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("🔄 一键分类").clicked() {
                        self.perform_auto_classify();
                    }
                    if ui.button("🗑 清除 & 恢复").clicked() {
                        self.clear_and_restore();
                    }
                    if ui.button("💾 保存配置").clicked() {
                        self.save_layout_and_config();
                    }
                    if ui.button("➕ 创建分类框").clicked() {
                        self.open_create_box_dialog();
                    }
                });
                ui.separator();
                if !self.ctx.state.status_message.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(0, 200, 0),
                        &self.ctx.state.status_message,
                    );
                }
                if self.ctx.state.icons_hidden {
                    ui.colored_label(egui::Color32::YELLOW, "⚠ 原生图标已隐藏");
                }
                ui.label(format!("当前格子数: {}", get_grid_count()));
                ui.separator();
                ui.label("• 右键分类框空白处 → 桌面菜单 + 创建分类框");
                ui.label("• 双击图标 → 打开 / 拖动标题栏 → 移动");
            });
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(300));
    }
}

impl Drop for MyApp {
    fn drop(&mut self) {
        let layout = collect_grid_layout();
        if !layout.is_empty() {
            self.ctx.config.layout.boxes = layout;
            self.merge_latest_overlay_config();
            let _ = self.ctx.save_config();
        }
        destroy_all_grid_windows();
        destroy_desktop_overlay();
        if self.ctx.state.icons_hidden {
            let _ = restore_native_icons();
            self.ctx.state.icons_hidden = false;
        }
        // 程序退出时清理注册表，确保不运行时桌面菜单完全原生
        cleanup_desktop_registry();
        cleanup_system_tray();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== 信号文件路径 =====

    #[test]
    fn test_signal_file_path_is_in_temp_dir() {
        let path = signal_file_path();
        let temp = std::env::temp_dir();
        assert_eq!(path.parent().unwrap(), temp.as_path());
        assert_eq!(path.file_name().unwrap(), "ikun_create_box.signal");
    }

    // ===== 分类框名称验证 =====

    #[test]
    fn test_box_name_valid_empty_is_invalid() {
        let name = "";
        assert!(!is_box_name_valid(name));
    }

    #[test]
    fn test_box_name_valid_whitespace_is_invalid() {
        assert!(!is_box_name_valid("   "));
    }

    #[test]
    fn test_box_name_valid_within_limit() {
        assert!(is_box_name_valid("测试框"));
        assert!(is_box_name_valid("a"));
        assert!(is_box_name_valid("1234567890123456")); // 恰好 16 字符
    }

    #[test]
    fn test_box_name_valid_exceeds_limit() {
        assert!(!is_box_name_valid("12345678901234567")); // 17 字符
        assert!(!is_box_name_valid("这是一个超过十六个字符的名称测试用例")); // 19 字符
    }

    // ===== DialogState =====

    #[test]
    fn test_dialog_state_default() {
        let state = DialogState::default();
        assert!(state.name.is_empty());
        assert_eq!(state.open_state, DialogOpenState::Closed);
        assert!(!state.confirmed);
        assert!(!state.cancelled);
    }

    #[test]
    fn test_dialog_state_open_resets_fields() {
        let app = MyApp::default();
        // 先设置一些脏值
        {
            let mut s = app.dialog.lock().unwrap();
            s.name = "脏数据".to_string();
            s.confirmed = true;
            s.cancelled = true;
        }
        // open_create_box_dialog 应重置所有字段
        app.open_create_box_dialog();
        let s = app.dialog.lock().unwrap();
        assert_eq!(s.open_state, DialogOpenState::Open);
        assert!(s.name.is_empty());
        assert!(!s.confirmed);
        assert!(!s.cancelled);
    }

    #[test]
    fn test_dialog_state_not_open_initially() {
        let app = MyApp::default();
        let state = app
            .dialog
            .lock()
            .map(|s| s.open_state)
            .unwrap_or(DialogOpenState::Closed);
        assert_eq!(state, DialogOpenState::Closed);
    }

    // ===== 三态状态机转换 =====

    #[test]
    fn test_dialog_state_machine_closed_to_open() {
        let app = MyApp::default();
        assert_eq!(
            app.dialog.lock().unwrap().open_state,
            DialogOpenState::Closed
        );
        app.open_create_box_dialog();
        assert_eq!(app.dialog.lock().unwrap().open_state, DialogOpenState::Open);
    }

    #[test]
    fn test_dialog_state_machine_open_to_closing() {
        let app = MyApp::default();
        app.open_create_box_dialog();

        // 模拟用户点击取消
        {
            let mut s = app.dialog.lock().unwrap();
            s.open_state = DialogOpenState::Closing;
            s.cancelled = true;
        }

        let s = app.dialog.lock().unwrap();
        assert_eq!(s.open_state, DialogOpenState::Closing);
        assert!(s.cancelled);
    }

    #[test]
    fn test_dialog_open_stores_target_pos() {
        let app = MyApp::default();
        app.open_create_box_dialog();
        let s = app.dialog.lock().unwrap();
        // target_pos 应被设置（具体值取决于鼠标位置，只验证非 None）
        assert!(s.target_pos.is_some());
    }

    // ===== 名称验证边界 =====

    #[test]
    fn test_box_name_valid_exactly_max_len() {
        // 恰好 BOX_NAME_MAX_LEN (16) 个字符
        let name = "1234567890ABCDEF"; // 16 ASCII chars
        assert_eq!(name.chars().count(), BOX_NAME_MAX_LEN);
        assert!(is_box_name_valid(name));
    }

    #[test]
    fn test_box_name_valid_unicode_counts_as_chars() {
        // Unicode 字符按 chars().count() 计算，不是字节数
        let name = "分类框测试用名称一二三四"; // 12 Chinese chars
        assert!(name.chars().count() <= BOX_NAME_MAX_LEN);
        assert!(is_box_name_valid(name));

        let long_name = "分类框测试用名称一二三四五六七八九"; // 17 Chinese chars
        assert!(!is_box_name_valid(long_name));
    }

    #[test]
    fn test_box_name_valid_single_char() {
        assert!(is_box_name_valid("A"));
        assert!(is_box_name_valid("中"));
        assert!(is_box_name_valid("1"));
    }

    // ===== 阶段 6 生命周期判定 =====

    #[test]
    fn test_should_retry_native_icon_hide_when_state_hidden_but_native_visible() {
        assert!(should_retry_native_icon_hide(true, false));
    }

    #[test]
    fn test_should_not_retry_native_icon_hide_when_both_hidden() {
        assert!(!should_retry_native_icon_hide(true, true));
    }

    #[test]
    fn test_should_not_retry_native_icon_hide_when_app_did_not_hide_icons() {
        assert!(!should_retry_native_icon_hide(false, false));
        assert!(!should_retry_native_icon_hide(false, true));
    }

    // ===== 显示名称解析优先级测试 =====

    /// CLSID 路径应始终使用 clsid_display_name 解析，而非 display_names 数组
    #[test]
    fn test_display_name_clsid_uses_mapping_not_array() {
        // Simulate: display_names has wrong value for CLSID path
        let file_path = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
        let display_names = vec!["wrong_name".to_string()];

        // The resolution should use CLSID mapping, not display_names
        let result = if file_path.starts_with("::{") {
            crate::platform::clsid_display_name(file_path)
                .map(|s| s.to_string())
                .unwrap_or_else(|| file_path.to_string())
        } else if !display_names.is_empty() && !display_names[0].is_empty() {
            display_names[0].clone()
        } else {
            file_path.to_string()
        };

        assert_eq!(result, "此电脑");
    }

    // ===== CLSID display name test =====

    #[test]
    fn test_clsid_display_name_from_config() {
        // Test the exact CLSID from the user's config file
        let clsid = "::{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}";
        let result = crate::platform::clsid_display_name(clsid);
        log::info!("CLSID {} -> {:?}", clsid, result);
        assert_eq!(result, Some("控制面板"));

        // Test other CLSIDs
        assert_eq!(
            crate::platform::clsid_display_name("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"),
            Some("此电脑")
        );
        assert_eq!(
            crate::platform::clsid_display_name("::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}"),
            Some("网络")
        );
        assert_eq!(
            crate::platform::clsid_display_name("::{645FF040-5081-101B-9F08-00AA002F954E}"),
            Some("回收站")
        );

        // Test case insensitivity
        assert_eq!(
            crate::platform::clsid_display_name("::{5399e694-6ce5-4d6c-8fce-1d8870fdcba0}"),
            Some("控制面板")
        );

        // Test unknown CLSID
        assert_eq!(
            crate::platform::clsid_display_name("::{00000000-0000-0000-0000-000000000000}"),
            None
        );
    }
}

/// 验证分类框名称是否有效（非空且不超过 BOX_NAME_MAX_LEN 字符）
fn is_box_name_valid(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= BOX_NAME_MAX_LEN
}

/// 判断是否需要重新隐藏原生桌面图标。
///
/// Explorer 重启或桌面列表窗口重新可见时，应用运行态仍认为已接管桌面，
/// 此时需要重新执行隐藏并保持覆盖窗口生命周期。
fn should_retry_native_icon_hide(state_icons_hidden: bool, native_hidden: bool) -> bool {
    state_icons_hidden && !native_hidden
}
