//! GPUI 应用运行时 — 与 UI 框架无关的业务逻辑层

#![allow(dead_code)] // GPUI 应用运行时，部分方法/类型预留
//!
//! 完整移植自 app.rs（eframe 版本）的所有业务逻辑。
//! 使用 Arc<Mutex<>> 保护状态，避免 GPUI RefCell 借用冲突。

use crate::config::app_config::AppConfig;
use crate::context::AppContext;
// 全局编辑分类框回调存储
lazy_static::lazy_static! {
    static ref EDIT_BOX_CALLBACK: Mutex<Option<Box<dyn Fn(uuid::Uuid, &str) + Send + Sync>>> = Mutex::new(None);
    static ref PENDING_EDIT_REQUEST: Mutex<Option<(uuid::Uuid, String)>> = Mutex::new(None);
}

/// 获取待处理的编辑请求（从全局存储中取出）
pub fn take_pending_edit_request() -> Option<(uuid::Uuid, String)> {
    let mut pending = PENDING_EDIT_REQUEST.lock().unwrap();
    pending.take()
}

/// 设置全局编辑分类框回调
pub fn set_edit_box_callback(callback: Box<dyn Fn(uuid::Uuid, &str) + Send + Sync>) {
    let mut cb = EDIT_BOX_CALLBACK.lock().unwrap();
    *cb = Some(callback);
}

/// 触发编辑分类框回调
pub fn trigger_edit_box_callback(config_id: uuid::Uuid, current_name: &str) {
    // 将编辑请求存入全局存储，由 GPUI 轮询定时器检查并打开弹窗
    let mut pending = PENDING_EDIT_REQUEST.lock().unwrap();
    *pending = Some((config_id, current_name.to_string()));
    log::info!(
        "编辑请求已存入全局存储: config_id={}, name={}",
        config_id,
        current_name
    );
}
use crate::platform::{
    cleanup_desktop_registry, cleanup_system_tray, collect_grid_layout, create_grid_window,
    destroy_all_grid_windows, extract_file_icon, get_grid_count, get_primary_monitor_dpi,
    get_screen_resolution, hide_control_panel_window, icon_exists_for_box, resolve_icon_read_path,
    restore_all_hidden_icons, set_tray_auto_start_state, setup_desktop_registry,
    setup_global_hotkey, setup_system_tray, show_control_panel_window, take_hotkey_toggle,
    take_pending_create_box, take_pending_tray_action, update_grid_window_title, DesktopIcon,
    DesktopManagerTrait, GridCreateParams, GridFileItem, SystemTrayAction,
};
use std::sync::{Arc, Mutex};

/// 新建分类框名称最大长度（字符数）
pub const BOX_NAME_MAX_LEN: usize = 16;

/// 控制面板标题（Win32 窗口查找用）
pub const CONTROL_PANEL_TITLE: &str = "Desktop Organizer 控制面板";

/// 分类框名称校验结果。
///
/// GPUI 创建分类框窗口、legacy eframe 回退路径迁移测试均应复用该纯逻辑，
/// 避免 UI 层重复实现 trim / Unicode 字符计数规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxNameValidation {
    /// 去除首尾空白后的名称。
    pub trimmed: String,
    /// 原始输入按 Unicode scalar value 统计的字符数。
    pub char_count: usize,
    /// trim 后是否为空。
    pub is_empty: bool,
    /// 原始输入字符数是否超过上限。
    pub is_too_long: bool,
    /// 是否可用于创建分类框。
    pub is_valid: bool,
}

impl Default for BoxNameValidation {
    fn default() -> Self {
        Self::new("")
    }
}

impl BoxNameValidation {
    /// 根据用户输入构造分类框名称校验结果。
    pub fn new(input: &str) -> Self {
        let trimmed = input.trim().to_string();
        let char_count = input.chars().count();
        let is_empty = trimmed.is_empty();
        let is_too_long = char_count > BOX_NAME_MAX_LEN;
        let is_valid = !is_empty && !is_too_long;
        Self {
            trimmed,
            char_count,
            is_empty,
            is_too_long,
            is_valid,
        }
    }

    /// 返回与旧 eframe 对话框一致的校验提示文案。
    pub fn message(&self) -> String {
        if self.is_empty {
            "名称不能为空".to_string()
        } else if self.is_too_long {
            format!(
                "⚠ 最多 {} 字符（当前 {}）",
                BOX_NAME_MAX_LEN, self.char_count
            )
        } else {
            format!("✓ {}/{}", self.char_count, BOX_NAME_MAX_LEN)
        }
    }
}

/// 线程安全的共享运行时
pub type SharedAppRuntime = Arc<Mutex<AppRuntime>>;

// =====================================================================
// 信号文件路径
// =====================================================================

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

// =====================================================================
// 验证函数
// =====================================================================

/// 验证分类框名称是否有效（非空且不超过 BOX_NAME_MAX_LEN 字符）
pub fn is_box_name_valid(name: &str) -> bool {
    BoxNameValidation::new(name).is_valid
}

/// 标准化分类框名称（去除首尾空白）。
pub fn normalized_box_name(name: &str) -> String {
    BoxNameValidation::new(name).trimmed
}

/// 返回分类框名称校验结果。
pub fn validate_box_name(name: &str) -> BoxNameValidation {
    BoxNameValidation::new(name)
}

/// 判断是否需要重新隐藏原生桌面图标

// =====================================================================
// AppRuntime
// =====================================================================

/// GPUI 应用运行时（UI 框架无关）
///
/// 使用 Arc<Mutex<>> 保护，避免 GPUI RefCell 借用冲突。
pub struct AppRuntime {
    /// 应用上下文（配置、桌面管理器、分类器、状态）
    ctx: AppContext,
    /// 是否已完成首帧初始化
    initialized: bool,
    /// 是否请求退出
    quit_requested: bool,

    // ---- 创建分类框对话框状态 ----
    /// 对话框是否打开
    pub dialog_open: bool,
    /// 对话框输入值
    pub dialog_name: String,
    /// 对话框是否需要在本帧打开
    pub dialog_needs_open: bool,

    // ---- 编辑分类框对话框状态 ----
    /// 编辑对话框是否需要在本帧打开
    pub edit_dialog_needs_open: bool,
    /// 编辑对话框的 config_id
    pub edit_dialog_config_id: Option<uuid::Uuid>,

    // ---- 桌面图标位置快照 ----
    /// 启动时记录的桌面图标位置：名称 → (client_x, client_y)
    desktop_positions_snapshot: std::collections::HashMap<String, (i32, i32)>,
    /// 编辑对话框的初始名称
    pub edit_dialog_initial_name: String,
}

/// 从桌面图标快照中过滤出仓库管理的图标
///
/// 纯逻辑函数，不含平台 API 调用，可独立进行单元测试。
/// 输入完整的桌面图标位置快照和仓库图标显示名称集合，
/// 返回仅包含仓库图标的位置列表（供后续恢复使用）。
///
/// # 参数
/// - snapshot: 启动时记录的桌面图标名称→位置映射
/// - `vault_display_names`: 仓库中所有图标的显示名称（小写）
///
/// # 返回
/// 返回仅包含仓库图标的位置元组列表 (名称引用, x, y)
fn filter_vault_positions<'a>(
    snapshot: &'a std::collections::HashMap<String, (i32, i32)>,
    vault_display_names: &std::collections::HashSet<String>,
) -> Vec<(&'a str, i32, i32)> {
    let mut result = Vec::new();
    for (name, &(x, y)) in snapshot {
        if vault_display_names.contains(&name.to_lowercase()) {
            result.push((name.as_str(), x, y));
        }
    }
    result
}

impl AppRuntime {
    /// 创建新的应用运行时
    pub fn new() -> Self {
        let ctx = AppContext::new().unwrap_or_else(|e| {
            log::warn!("初始化 AppContext 失败: {}，使用默认值", e);
            AppContext::default()
        });
        Self {
            ctx,
            initialized: false,
            quit_requested: false,
            dialog_open: false,
            dialog_name: String::new(),
            dialog_needs_open: false,
            edit_dialog_needs_open: false,
            edit_dialog_config_id: None,
            edit_dialog_initial_name: String::new(),
            desktop_positions_snapshot: std::collections::HashMap::new(),
        }
    }

    /// 创建线程安全的共享运行时
    pub fn new_shared() -> SharedAppRuntime {
        Arc::new(Mutex::new(Self::new()))
    }

    /// 首帧初始化
    pub fn initialize(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        log::info!("GPUI AppRuntime 初始化开始");

        self.ctx.state.panel_visible = false;

        if let Err(e) = setup_system_tray() {
            log::warn!("初始化系统托盘失败: {}", e);
            self.ctx.set_status(format!("初始化系统托盘失败: {}", e));
        }

        self.restore_layout();

        #[cfg(windows)]
        crate::platform::windows::icon_manager::diagnose();

        cleanup_desktop_registry();
        setup_desktop_registry();
        // 注册全局热键
        let hotkey_str = self.ctx.config.hotkeys.toggle_panel.clone();
        if let Err(e) = setup_global_hotkey(&hotkey_str) {
            log::warn!("注册全局热键失败: {} (热键: {})", e, hotkey_str);
        }

        // 同步开机自启状态
        #[cfg(windows)]
        {
            let auto_start = self.ctx.config.general.auto_start;
            set_tray_auto_start_state(auto_start);
            if let Err(e) = crate::utils::platform::set_auto_start(auto_start) {
                log::warn!("同步开机自启状态失败: {}", e);
            }
        }

        log::info!("GPUI AppRuntime 初始化完成");
    }

    /// 定时轮询
    pub fn tick(&mut self) -> bool {
        let mut changed = false;

        // 1. 消费托盘动作
        if let Some(action) = take_pending_tray_action() {
            self.handle_tray_action(action);
            changed = true;
        }

        // 3. 信号文件轮询
        if signal_file_path().exists() {
            let _ = std::fs::remove_file(signal_file_path());
            self.open_create_box_dialog();
            changed = true;
        }
        if open_panel_signal_path().exists() {
            let _ = std::fs::remove_file(open_panel_signal_path());
            self.open_control_panel();
            changed = true;
        }
        if ai_classify_signal_path().exists() {
            let _ = std::fs::remove_file(ai_classify_signal_path());
            self.perform_auto_classify();
            changed = true;
        }
        if quit_signal_path().exists() {
            let _ = std::fs::remove_file(quit_signal_path());
            self.quit();
            changed = true;
        }

        // 4. 全局热键切换面板
        if take_hotkey_toggle() {
            log::info!("全局热键触发: 切换控制面板");
            if self.ctx.state.panel_visible {
                self.hide_control_panel();
            } else {
                self.open_control_panel();
            }
            changed = true;
        }

        if take_pending_create_box() {
            self.open_create_box_dialog();
            changed = true;
        }

        changed
    }

    // =================================================================
    // 控制面板 显示/隐藏
    // =================================================================

    pub fn open_control_panel(&mut self) {
        log::info!("打开控制面板窗口");
        self.ctx.state.panel_visible = true;
        show_control_panel_window(CONTROL_PANEL_TITLE);
    }

    pub fn hide_control_panel(&mut self) {
        log::info!("隐藏控制面板窗口");
        self.ctx.state.panel_visible = false;
        hide_control_panel_window(CONTROL_PANEL_TITLE);
    }

    pub fn panel_visible(&self) -> bool {
        self.ctx.state.panel_visible
    }

    /// 切换开机自启状态。
    ///
    /// 翻转配置中的 `general.auto_start`，同步写入注册表并更新托盘菜单勾选。
    fn toggle_auto_start(&mut self) {
        let new_state = !self.ctx.config.general.auto_start;
        self.ctx.config.general.auto_start = new_state;

        // 持久化配置
        if let Err(e) = self.ctx.config.save() {
            log::warn!("保存开机自启配置失败: {}", e);
        }

        // 同步注册表
        #[cfg(windows)]
        {
            if let Err(e) = crate::utils::platform::set_auto_start(new_state) {
                log::warn!("切换开机自启失败: {}", e);
            }
        }

        // 更新托盘菜单勾选
        set_tray_auto_start_state(new_state);
        log::info!("开机自启已切换: {}", new_state);
    }

    // =================================================================
    // 托盘动作分发
    // =================================================================

    pub fn handle_tray_action(&mut self, action: SystemTrayAction) {
        log::info!("消费系统托盘动作: {:?}", action);
        match action {
            SystemTrayAction::OpenControlPanel => self.open_control_panel(),
            SystemTrayAction::CreateBox => {
                log::info!("系统托盘菜单: 创建分类框");
                self.open_create_box_dialog();
            }
            SystemTrayAction::AiClassify => {
                log::info!("托盘菜单: 一键AI分类");
                self.perform_auto_classify();
            }
            SystemTrayAction::ToggleAutoStart => {
                self.toggle_auto_start();
            }
            SystemTrayAction::Quit => {
                log::info!("托盘菜单: 退出");
                self.quit();
            }
        }
    }

    // =================================================================
    // 创建分类框对话框
    // =================================================================

    pub fn open_create_box_dialog(&mut self) {
        self.dialog_name.clear();
        self.dialog_open = true;
        self.dialog_needs_open = true;
        log::info!("打开创建分类框对话框");
    }

    pub fn confirm_create_box(&mut self) {
        let name = self.dialog_name.trim().to_string();
        if !is_box_name_valid(&name) {
            log::warn!("分类框名称无效: {:?}", name);
            return;
        }
        self.dialog_open = false;
        self.dialog_name.clear();
        self.create_empty_box(name);
    }

    pub fn cancel_create_box_dialog(&mut self) {
        self.dialog_open = false;
        self.dialog_name.clear();
    }

    /// 重命名分类框
    pub fn rename_box(&mut self, config_id: uuid::Uuid, new_name: &str) {
        log::info!(
            "[rename_box] 开始: config_id={}, new_name={:?}",
            config_id,
            new_name
        );
        let name = new_name.trim().to_string();
        if !is_box_name_valid(&name) {
            log::warn!("[rename_box] 分类框名称无效: {:?}", name);
            return;
        }

        // 查找对应的 Box 并更新名称
        if let Some(box_item) = self
            .ctx
            .config
            .layout
            .boxes
            .iter_mut()
            .find(|b| b.id == config_id)
        {
            let old_name = box_item.name.clone();
            box_item.name = name.clone();
            log::info!(
                "[rename_box] 重命名分类框: '{}' -> '{}' (config_id={})",
                old_name,
                name,
                config_id
            );

            // 保存配置
            if let Err(e) = self.ctx.save_config() {
                log::error!("[rename_box] 保存配置失败: {}", e);
            } else {
                log::info!("[rename_box] 配置保存成功");
            }

            // 更新对应的格子窗口显示
            log::debug!("[rename_box] 调用 update_grid_window_title");
            update_grid_window_title(config_id, &name);
            log::info!("[rename_box] 完成");
        } else {
            log::warn!("[rename_box] 未找到 config_id: {} 对应的分类框", config_id);
            log::debug!("[rename_box] 当前已有的 config_id 列表:");
            for b in &self.ctx.config.layout.boxes {
                log::debug!("  - config_id={}, name={}", b.id, b.name);
            }
        }
    }

    // =================================================================
    // 业务逻辑
    // =================================================================

    pub fn perform_auto_classify(&mut self) {
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

        let (screen_w_phys, screen_h_phys) = get_screen_resolution();
        let box_w = self.ctx.config.box_defaults.default_width;
        let box_h = self.ctx.config.box_defaults.default_height;
        let padding = 20i32;
        let total_w = 3 * box_w + 2 * padding;
        // GridCreateParams 的 x/y 是逻辑像素（96 DPI 基准），需要物理→逻辑转换
        let primary_dpi = get_primary_monitor_dpi();
        let screen_w = (screen_w_phys * 96) / primary_dpi as i32;
        let screen_h = (screen_h_phys * 96) / primary_dpi as i32;
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
                    grid_col: -1,
                    grid_row: -1,
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
                edit_box_callback: Some(Box::new(|config_id, name| {
                    trigger_edit_box_callback(config_id, name);
                })),
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

    pub fn clear_and_restore(&mut self) {
        let layout = collect_grid_layout();
        self.ctx.config.layout.boxes = layout;
        self.merge_latest_overlay_config();
        let _ = self.ctx.save_config();

        destroy_all_grid_windows();
        if self.ctx.state.icons_hidden {
            self.ctx.state.icons_hidden = false;
        }

        self.ctx.set_status("已清除，桌面图标已恢复");
    }

    pub fn save_layout_and_config(&mut self) {
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

    fn create_empty_box(&mut self, name: String) {
        log::info!("创建分类框: '{}'", name);
        let (screen_w_phys, screen_h_phys) = get_screen_resolution();
        let box_w = 400;
        let box_h = 400;
        let existing = get_grid_count();
        // GridCreateParams 的 x/y 是逻辑像素（96 DPI 基准），create_grid_window 会按实际 DPI 缩放。
        // 将物理像素屏幕尺寸转换为逻辑像素，再计算居中位置。
        let primary_dpi = get_primary_monitor_dpi();
        let screen_w = (screen_w_phys * 96) / primary_dpi as i32;
        let screen_h = (screen_h_phys * 96) / primary_dpi as i32;
        // 新建分类框默认出现在主屏幕中央（与'创建分类框'对话框行为一致）
        // 多个框通过小偏移量级联避免完全重叠
        let x = ((screen_w - box_w) / 2 + (existing as i32 * 40) % 200).max(10);
        let y = ((screen_h - box_h) / 2 + (existing as i32 * 30) % 150).max(10);

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
            edit_box_callback: Some(Box::new(|config_id, name| {
                trigger_edit_box_callback(config_id, name);
            })),
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

    /// 快照当前桌面图标位置
    ///
    /// 在程序启动时调用，记录所有桌面图标的当前位置，
    /// 以便在退出时恢复到这些位置。
    fn snapshot_desktop_positions(&mut self) {
        log::info!("开始快照桌面图标位置");

        self.desktop_positions_snapshot =
            crate::platform::windows::icon_manager::snapshot_all_desktop_positions();

        log::info!(
            "快照完成，记录了 {} 个图标位置",
            self.desktop_positions_snapshot.len()
        );
    }

    fn restore_layout(&mut self) {
        if !self.ctx.config.general.restore_layout_on_start {
            log::info!("启动恢复布局已禁用");
            return;
        }

        restore_all_hidden_icons();

        // 在隐藏 SysListView32 之前，快照所有桌面图标位置
        // 必须在 restore_all_hidden_icons 之后，确保仓库图标已还原到桌面
        self.snapshot_desktop_positions();

        // 不再创建覆盖区域窗口 — 使用原生桌面渲染未分类图标

        let boxes = self.ctx.config.layout.boxes.clone();
        if boxes.is_empty() {
            log::info!("启动布局为空");
            self.ctx.set_status("桌面覆盖窗口已启用");
            return;
        }
        log::info!("开始恢复布局: {} 个分类框", boxes.len());

        // 先隐藏已分类图标，再创建带 Desktop Duplication 快照的分类框。
        // 首个分类框创建时会建立桌面快照，若顺序相反，玻璃背景会重复显示这些图标。
        {
            let mut classified_paths: Vec<String> = Vec::new();
            for box_data in &boxes {
                classified_paths.extend(box_data.icon_names.iter().cloned());
            }
            if !classified_paths.is_empty() {
                log::info!("启动: 隐藏 {} 个已分类图标", classified_paths.len());
                for path in &classified_paths {
                    match crate::platform::windows::icon_hider::hide_single_icon(path) {
                        Ok(()) => log::debug!("启动隐藏: '{}'", path),
                        Err(e) => log::warn!("启动隐藏失败: '{}': {}", path, e),
                    }
                }
                let classified_set: std::collections::HashSet<String> = classified_paths
                    .iter()
                    .map(|p| p.replace('/', "\\").to_lowercase())
                    .collect();
                self.desktop_positions_snapshot.retain(|name, _| {
                    let lower = name.to_lowercase();
                    !classified_set.iter().any(|c| {
                        lower == *c
                            || lower.ends_with(&format!("\\{}", c.split('\\').last().unwrap_or("")))
                    })
                });
                log::info!(
                    "启动隐藏完成，快照剩余 {} 个未分类图标",
                    self.desktop_positions_snapshot.len()
                );
            }
        }

        destroy_all_grid_windows();

        let mut created_count = 0;
        for (i, box_data) in boxes.iter().enumerate() {
            let mut items: Vec<GridFileItem> = Vec::new();
            for (idx, file_path) in box_data.icon_names.iter().enumerate() {
                if !icon_exists_for_box(file_path) {
                    log::info!("跳过已删除的图标: {}", file_path);
                    continue;
                }
                let read_path = resolve_icon_read_path(file_path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.clone());
                // CLSID 路径始终从路径本身解析显示名称
                let display_name = if file_path.starts_with("::{") {
                    crate::platform::clsid_display_name(file_path)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| file_path.clone())
                } else if idx < box_data.display_names.len()
                    && !box_data.display_names[idx].is_empty()
                {
                    // 普通文件路径优先使用持久化的 display_name
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
                // Restore grid position if saved
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
                edit_box_callback: Some(Box::new(|config_id, name| {
                    trigger_edit_box_callback(config_id, name);
                })),
            };
            if create_grid_window(params).is_ok() {
                created_count += 1;
            }
        }

        self.ctx
            .set_status(format!("恢复了 {} 个分类框", created_count));
    }

    fn ensure_native_icons_hidden(&mut self) -> bool {
        // 不再隐藏原生桌面图标 — 使用原生桌面渲染未分类图标
        true
    }

    fn ensure_desktop_overlay_running(&mut self) {
        // 不再创建覆盖区域窗口 — 使用原生桌面渲染未分类图标
    }

    fn merge_latest_overlay_config(&mut self) {
        log::info!("开始合并覆盖窗口配置");
        match AppConfig::load() {
            Ok(config) => {
                log::info!(
                    "从磁盘加载的 icon_positions 数={}",
                    config.overlay.icon_positions.len()
                );
                self.ctx.config.overlay = config.overlay;
                log::info!(
                    "合并后 icon_positions 数={}",
                    self.ctx.config.overlay.icon_positions.len()
                );
            }
            Err(e) => {
                log::warn!("保存配置前读取覆盖窗口配置失败: {}", e);
            }
        }
    }

    // =================================================================
    // 退出与清理
    // =================================================================

    pub fn quit(&mut self) {
        log::info!("退出应用");
        self.quit_requested = true;
        self.cleanup();
    }

    pub fn is_quit_requested(&self) -> bool {
        self.quit_requested
    }

    fn restore_desktop_positions(&self, vault_display_names: &std::collections::HashSet<String>) {
        log::info!("开始恢复桌面图标位置");

        let positions = &self.desktop_positions_snapshot;

        if positions.is_empty() {
            log::info!("没有快照位置，跳过恢复");
            return;
        }

        // 使用纯逻辑函数过滤出仓库图标（跳过用户在运行时拖拽的其他图标）
        let total = positions.len();
        let vault_icons = filter_vault_positions(positions, vault_display_names);
        let filtered_count = total - vault_icons.len();
        let mut pending: Vec<(&str, i32, i32)> = Vec::new();
        let mut success_count = 0;

        for &(name, x, y) in &vault_icons {
            match crate::platform::windows::icon_manager::set_icon_position_by_name(name, x, y) {
                Ok(()) => success_count += 1,
                Err(e) => {
                    let msg = format!("{}", e);
                    if msg.contains("未找到图标") {
                        log::info!("图标 '{}' 尚未出现在 SysListView32，加入重试队列", name);
                        pending.push((name, x, y));
                    } else {
                        log::debug!("恢复图标位置失败: '{}': {}", name, e);
                    }
                }
            }
        }

        log::info!(
            "跳过 {} 个非仓库图标，恢复 {} 个仓库图标",
            filtered_count,
            success_count + pending.len()
        );

        // 重试轮次：等待 Windows 将刚还原的图标加入 SysListView32
        let retry_delays = [500u64, 1000, 1500, 2000];
        for (attempt, &delay_ms) in retry_delays.iter().enumerate() {
            if pending.is_empty() {
                break;
            }
            log::info!(
                "等待 {}ms 后重试（第 {} 次），剩余 {} 个图标未找到",
                delay_ms,
                attempt + 1,
                pending.len()
            );
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));

            let mut still_pending = Vec::new();
            for &(name, x, y) in &pending {
                match crate::platform::windows::icon_manager::set_icon_position_by_name(name, x, y)
                {
                    Ok(()) => {
                        log::info!("重试成功: '{}' 已找到并恢复位置", name);
                        success_count += 1;
                    }
                    Err(e) => {
                        let msg = format!("{}", e);
                        if msg.contains("未找到图标") {
                            still_pending.push((name, x, y));
                        } else {
                            log::debug!("重试时恢复图标位置失败: '{}': {}", name, e);
                        }
                    }
                }
            }
            pending = still_pending;
        }

        if !pending.is_empty() {
            log::warn!(
                "以下 {} 个图标始终未在 SysListView32 中找到，无法恢复位置: {}",
                pending.len(),
                pending
                    .iter()
                    .map(|(n, _, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        log::info!("位置恢复完成，成功设置 {}/{} 个图标", success_count, total);
    }

    fn cleanup(&mut self) {
        log::info!("开始执行清理");

        let layout = collect_grid_layout();
        log::info!("收集到 {} 个分类框", layout.len());
        if !layout.is_empty() {
            self.ctx.config.layout.boxes = layout;
            log::info!("调用 merge_latest_overlay_config");
            self.merge_latest_overlay_config();
            log::info!(
                "merge 完成, overlay.icon_positions 数={}",
                self.ctx.config.overlay.icon_positions.len()
            );
            if let Err(e) = self.ctx.save_config() {
                log::warn!("退出前保存布局失败: {}", e);
            }
        }

        destroy_all_grid_windows();
        cleanup_system_tray();
        cleanup_desktop_registry();

        // 在还原图标之前，获取仓库中的图标显示名称
        // 只恢复这些图标的位置，避免覆盖用户在运行时拖拽的其他图标位置
        let vault_display_names = crate::platform::windows::icon_hider::get_vault_display_names();
        log::info!(
            "仓库图标显示名称: {} 个，只恢复这些图标的位置",
            vault_display_names.len()
        );

        restore_all_hidden_icons();

        // 等待 Windows 处理图标还原
        std::thread::sleep(std::time::Duration::from_millis(500));

        // 只恢复仓库图标到快照的位置（用户在运行时拖拽的其他图标位置不受影响）
        self.restore_desktop_positions(&vault_display_names);
    }

    // =================================================================
    // 状态读取（UI 层用）
    // =================================================================

    pub fn status_message(&self) -> &str {
        &self.ctx.state.status_message
    }

    pub fn icons_hidden(&self) -> bool {
        false
    }

    pub fn box_count(&self) -> usize {
        get_grid_count()
    }

    // =================================================================
    // Liquid Glass parameter accessors (UI layer)
    // =================================================================

    pub fn liquid_glass(&self) -> &crate::config::LiquidGlassConfig {
        &self.ctx.config.box_style.liquid_glass
    }

    pub fn set_liquid_glass_param(&mut self, param: crate::config::LiquidGlassParam, value: f32) {
        self.ctx
            .config
            .box_style
            .liquid_glass
            .set_param(param, value);
        self.apply_liquid_glass_config_change();
    }

    pub fn get_liquid_glass_param(&self, param: crate::config::LiquidGlassParam) -> f32 {
        self.ctx.config.box_style.liquid_glass.get_param(param)
    }

    pub fn reset_liquid_glass_defaults(&mut self) {
        self.ctx.config.box_style.liquid_glass = crate::config::LiquidGlassConfig::default();
        self.apply_liquid_glass_config_change();
    }

    pub fn set_liquid_glass_enabled(&mut self, enabled: bool) {
        self.ctx.config.box_style.liquid_glass.enabled = enabled;
        self.apply_liquid_glass_config_change();
    }

    /// 同步液态玻璃配置到渲染器、现有分类框和持久化文件。
    fn apply_liquid_glass_config_change(&mut self) {
        crate::platform::windows::liquid_glass::update_liquid_glass_config(
            &self.ctx.config.box_style.liquid_glass,
        );
        crate::platform::windows::grid_window::refresh_all_grid_windows();
        if let Err(error) = self.ctx.save_config() {
            log::error!("保存液态玻璃配置失败: {error}");
        }
    }

    pub fn get_liquid_glass_enabled(&self) -> bool {
        self.ctx.config.box_style.liquid_glass.enabled
    }

    pub fn set_liquid_glass_over_light(&mut self, val: bool) {
        self.ctx.config.box_style.liquid_glass.over_light = val;
        self.apply_liquid_glass_config_change();
    }

    pub fn get_liquid_glass_over_light(&self) -> bool {
        self.ctx.config.box_style.liquid_glass.over_light
    }
}

impl Drop for AppRuntime {
    fn drop(&mut self) {
        if !self.quit_requested {
            log::info!("AppRuntime 被 drop，执行紧急清理");
            self.cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_box_name_valid_basic() {
        assert!(is_box_name_valid("测试"));
        assert!(is_box_name_valid("abc"));
    }

    #[test]
    fn test_box_name_validation_trims_and_counts_original_input() {
        let validation = validate_box_name("  测试框  ");
        assert_eq!(validation.trimmed, "测试框");
        assert_eq!(validation.char_count, 7);
        assert!(!validation.is_empty);
        assert!(!validation.is_too_long);
        assert!(validation.is_valid);
        assert_eq!(normalized_box_name("  测试框  "), "测试框");
    }

    #[test]
    fn test_box_name_validation_empty_and_whitespace() {
        let empty = validate_box_name("");
        assert!(empty.is_empty);
        assert!(!empty.is_too_long);
        assert!(!empty.is_valid);
        assert_eq!(empty.message(), "名称不能为空");

        let spaces = validate_box_name("   ");
        assert!(spaces.is_empty);
        assert!(!spaces.is_valid);
        assert_eq!(spaces.message(), "名称不能为空");
    }

    #[test]
    fn test_box_name_validation_ascii_limit() {
        let valid = validate_box_name("1234567890123456");
        assert_eq!(valid.char_count, BOX_NAME_MAX_LEN);
        assert!(valid.is_valid);
        assert_eq!(valid.message(), "✓ 16/16");

        let invalid = validate_box_name("12345678901234567");
        assert_eq!(invalid.char_count, BOX_NAME_MAX_LEN + 1);
        assert!(invalid.is_too_long);
        assert!(!invalid.is_valid);
        assert_eq!(invalid.message(), "⚠ 最多 16 字符（当前 17）");
    }

    #[test]
    fn test_box_name_validation_unicode_limit() {
        let valid = validate_box_name("一二三四五六七八九十甲乙丙丁戊己");
        assert_eq!(valid.char_count, BOX_NAME_MAX_LEN);
        assert!(valid.is_valid);

        let invalid = validate_box_name("一二三四五六七八九十甲乙丙丁戊己庚");
        assert_eq!(invalid.char_count, BOX_NAME_MAX_LEN + 1);
        assert!(!invalid.is_valid);
    }

    #[test]
    fn test_box_name_validation_default_is_invalid_empty() {
        let validation = BoxNameValidation::default();
        assert!(validation.is_empty);
        assert!(!validation.is_valid);
    }

    #[test]
    fn test_box_name_valid_empty() {
        assert!(!is_box_name_valid(""));
        assert!(!is_box_name_valid("   "));
    }

    #[test]
    fn test_box_name_valid_max_len() {
        assert!(is_box_name_valid("1234567890123456"));
        assert!(!is_box_name_valid("12345678901234567"));
    }
    #[test]
    fn test_signal_file_paths() {
        assert!(signal_file_path()
            .to_string_lossy()
            .contains("ikun_create_box"));
        assert!(open_panel_signal_path()
            .to_string_lossy()
            .contains("ikun_open_panel"));
    }
}

// ============================================================================
// 纯逻辑函数（用于单元测试）
// ============================================================================

/// 物理像素转逻辑像素（96 DPI 基准）
///
/// # 参数
/// - `physical_px`: 物理像素值
/// - `dpi`: 显示器 DPI
///
/// # 返回
/// 逻辑像素值（96 DPI 基准）
fn physical_to_logical_px(physical_px: i32, dpi: u32) -> i32 {
    (physical_px * 96) / dpi.max(1) as i32
}

/// 计算分类框居中位置（带级联偏移）
///
/// # 参数
/// - `screen_size`: 屏幕逻辑像素尺寸
/// - `box_size`: 分类框逻辑像素尺寸
/// - `existing_count`: 已存在的分类框数量
/// - `offset_multiplier`: 偏移乘数（x 用 40，y 用 30）
/// - `offset_mod`: 偏移取模值（x 用 200，y 用 150）
///
/// # 返回
/// 居中位置坐标（最小值 10）
fn calculate_centered_position(
    screen_size: i32,
    box_size: i32,
    existing_count: usize,
    offset_multiplier: i32,
    offset_mod: i32,
) -> i32 {
    let offset = (existing_count as i32 * offset_multiplier) % offset_mod;
    ((screen_size - box_size) / 2 + offset).max(10)
}

#[cfg(test)]
mod pixel_conversion_tests {
    use super::*;

    #[test]
    fn test_physical_to_logical_at_96dpi() {
        assert_eq!(physical_to_logical_px(1920, 96), 1920);
        assert_eq!(physical_to_logical_px(1080, 96), 1080);
        assert_eq!(physical_to_logical_px(400, 96), 400);
    }

    #[test]
    fn test_physical_to_logical_at_144dpi() {
        assert_eq!(physical_to_logical_px(1920, 144), 1280);
        assert_eq!(physical_to_logical_px(1080, 144), 720);
        assert_eq!(physical_to_logical_px(600, 144), 400);
    }

    #[test]
    fn test_physical_to_logical_at_192dpi() {
        assert_eq!(physical_to_logical_px(1920, 192), 960);
        assert_eq!(physical_to_logical_px(1080, 192), 540);
        assert_eq!(physical_to_logical_px(800, 192), 400);
    }

    #[test]
    fn test_physical_to_logical_at_120dpi() {
        assert_eq!(physical_to_logical_px(1920, 120), 1536);
        assert_eq!(physical_to_logical_px(1080, 120), 864);
    }

    #[test]
    fn test_physical_to_logical_zero_dpi() {
        assert_eq!(physical_to_logical_px(1920, 0), 1920 * 96);
    }

    #[test]
    fn test_physical_to_logical_zero_pixels() {
        assert_eq!(physical_to_logical_px(0, 96), 0);
        assert_eq!(physical_to_logical_px(0, 144), 0);
    }

    #[test]
    fn test_physical_to_logical_roundtrip() {
        let logical = 400;
        let dpi = 144;
        let physical = logical * dpi as i32 / 96;
        let restored = physical_to_logical_px(physical, dpi);
        assert_eq!(restored, logical);
    }
}

#[cfg(test)]
mod positioning_tests {
    use super::*;

    #[test]
    fn test_centered_position_first_box() {
        let screen_w = 1920;
        let box_w = 400;
        let x = calculate_centered_position(screen_w, box_w, 0, 40, 200);
        assert_eq!(x, 760);
    }

    #[test]
    fn test_centered_position_with_offset() {
        let screen_w = 1920;
        let box_w = 400;
        let x = calculate_centered_position(screen_w, box_w, 1, 40, 200);
        assert_eq!(x, 800);
    }

    #[test]
    fn test_centered_position_offset_wraps() {
        let screen_w = 1920;
        let box_w = 400;
        let x = calculate_centered_position(screen_w, box_w, 5, 40, 200);
        assert_eq!(x, 760);
    }

    #[test]
    fn test_centered_position_minimum_10() {
        let screen_w = 300;
        let box_w = 400;
        let x = calculate_centered_position(screen_w, box_w, 0, 40, 200);
        assert_eq!(x, 10);
    }

    #[test]
    fn test_centered_position_y_axis() {
        let screen_h = 1080;
        let box_h = 400;
        let y = calculate_centered_position(screen_h, box_h, 0, 30, 150);
        assert_eq!(y, 340);
        let y2 = calculate_centered_position(screen_h, box_h, 2, 30, 150);
        assert_eq!(y2, 400);
    }

    #[test]
    fn test_centered_position_high_dpi_screen() {
        let screen_w = 1280;
        let box_w = 400;
        let x = calculate_centered_position(screen_w, box_w, 0, 40, 200);
        assert_eq!(x, 440);
    }

    #[test]
    fn test_centered_position_multiple_boxes() {
        let screen_w = 1920;
        let box_w = 400;
        let base = 760;
        assert_eq!(
            calculate_centered_position(screen_w, box_w, 0, 40, 200),
            base
        );
        assert_eq!(
            calculate_centered_position(screen_w, box_w, 1, 40, 200),
            base + 40
        );
        assert_eq!(
            calculate_centered_position(screen_w, box_w, 2, 40, 200),
            base + 80
        );
        assert_eq!(
            calculate_centered_position(screen_w, box_w, 3, 40, 200),
            base + 120
        );
        assert_eq!(
            calculate_centered_position(screen_w, box_w, 4, 40, 200),
            base + 160
        );
        assert_eq!(
            calculate_centered_position(screen_w, box_w, 5, 40, 200),
            base
        );
        assert_eq!(
            calculate_centered_position(screen_w, box_w, 6, 40, 200),
            base + 40
        );
    }
}

#[cfg(test)]
mod edit_dialog_tests {
    use super::*;
    use std::sync::Mutex;

    // 测试锁，防止测试间状态冲突
    lazy_static::lazy_static! {
        static ref TEST_LOCK: Mutex<()> = Mutex::new(());
    }

    #[test]
    fn test_take_pending_edit_request_empty() {
        let _guard = TEST_LOCK.lock().unwrap();
        // 清空待处理请求
        let _ = take_pending_edit_request();

        // 初始状态应该返回 None
        assert!(take_pending_edit_request().is_none());
    }

    #[test]
    fn test_trigger_and_take_edit_request() {
        let _guard = TEST_LOCK.lock().unwrap();
        // 清空待处理请求
        let _ = take_pending_edit_request();

        let test_id = uuid::Uuid::new_v4();
        let test_name = "测试分类框";

        // 触发编辑回调
        trigger_edit_box_callback(test_id, test_name);

        // 应该能取到编辑请求
        let request = take_pending_edit_request();
        assert!(request.is_some());
        let (id, name) = request.unwrap();
        assert_eq!(id, test_id);
        assert_eq!(name, test_name);

        // 第二次取应该返回 None（已消费）
        assert!(take_pending_edit_request().is_none());
    }

    #[test]
    fn test_edit_request_overwrite() {
        let _guard = TEST_LOCK.lock().unwrap();
        // 清空待处理请求
        let _ = take_pending_edit_request();

        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();

        // 触发两次编辑回调
        trigger_edit_box_callback(id1, "第一个");
        trigger_edit_box_callback(id2, "第二个");

        // 应该只取到最后一个
        let request = take_pending_edit_request();
        assert!(request.is_some());
        let (id, name) = request.unwrap();
        assert_eq!(id, id2);
        assert_eq!(name, "第二个");
    }

    #[test]
    fn test_edit_request_with_chinese_name() {
        let _guard = TEST_LOCK.lock().unwrap();
        let _ = take_pending_edit_request();

        let test_id = uuid::Uuid::new_v4();
        let test_name = "办公文档分类框";

        trigger_edit_box_callback(test_id, test_name);

        let request = take_pending_edit_request();
        assert!(request.is_some());
        let (_, name) = request.unwrap();
        assert_eq!(name, test_name);
    }

    #[test]
    fn test_edit_request_with_max_length_name() {
        let _guard = TEST_LOCK.lock().unwrap();
        let _ = take_pending_edit_request();

        let test_id = uuid::Uuid::new_v4();
        let test_name = "1234567890123456"; // 16 characters

        trigger_edit_box_callback(test_id, test_name);

        let request = take_pending_edit_request();
        assert!(request.is_some());
        let (_, name) = request.unwrap();
        assert_eq!(name, test_name);
        assert_eq!(name.chars().count(), 16);
    }
}

#[cfg(test)]
mod rename_validation_tests {
    use super::*;
    use std::sync::Mutex;

    lazy_static::lazy_static! {
        static ref TEST_LOCK: Mutex<()> = Mutex::new(());
    }

    #[test]
    fn test_rename_validation_rejects_empty() {
        // 空名称不应通过验证
        assert!(!is_box_name_valid(""));
        assert!(!is_box_name_valid("   "));
        assert!(!is_box_name_valid("\t\n"));
    }

    #[test]
    fn test_rename_validation_accepts_valid_names() {
        assert!(is_box_name_valid("办公"));
        assert!(is_box_name_valid("Work"));
        assert!(is_box_name_valid("工具-1"));
        assert!(is_box_name_valid("a"));
    }

    #[test]
    fn test_rename_validation_rejects_too_long() {
        let max = "a".repeat(BOX_NAME_MAX_LEN);
        assert!(is_box_name_valid(&max));

        let over = "a".repeat(BOX_NAME_MAX_LEN + 1);
        assert!(!is_box_name_valid(&over));
    }

    #[test]
    fn test_validate_box_name_trims_whitespace() {
        let v = validate_box_name("  hello  ");
        assert_eq!(v.trimmed, "hello");
        assert!(v.is_valid);
    }

    #[test]
    fn test_edit_request_chinese_name_preserved() {
        let _guard = TEST_LOCK.lock().unwrap();
        let _ = take_pending_edit_request();

        let id = uuid::Uuid::new_v4();
        let name = "中文名称测试框";
        trigger_edit_box_callback(id, name);

        let req = take_pending_edit_request().unwrap();
        assert_eq!(req.0, id);
        assert_eq!(req.1, name);
        // 确保消费后为空
        assert!(take_pending_edit_request().is_none());
    }

    #[test]
    fn test_edit_request_latest_wins() {
        let _guard = TEST_LOCK.lock().unwrap();
        let _ = take_pending_edit_request();

        // 快速连续触发 3 次
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();
        let id3 = uuid::Uuid::new_v4();
        trigger_edit_box_callback(id1, "第一次");
        trigger_edit_box_callback(id2, "第二次");
        trigger_edit_box_callback(id3, "第三次");

        // 只保留最后一个
        let req = take_pending_edit_request().unwrap();
        assert_eq!(req.0, id3);
        assert_eq!(req.1, "第三次");
        assert!(take_pending_edit_request().is_none());
    }
}

// ===== filter_vault_positions ???? =====

#[cfg(test)]
mod filter_vault_positions_tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn make_vault_names(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_lowercase().to_string()).collect()
    }

    #[test]
    fn test_empty_snapshot_returns_empty() {
        let snapshot = HashMap::new();
        let vault = make_vault_names(&["Chrome"]);
        let result = filter_vault_positions(&snapshot, &vault);
        assert!(result.is_empty());
    }

    #[test]
    fn test_empty_vault_returns_empty() {
        let mut snapshot = HashMap::new();
        snapshot.insert("Chrome".to_string(), (100, 200));
        snapshot.insert("Firefox".to_string(), (300, 400));
        let vault = HashSet::new();
        let result = filter_vault_positions(&snapshot, &vault);
        assert!(result.is_empty());
    }

    #[test]
    fn test_filters_only_vault_icons() {
        let mut snapshot = HashMap::new();
        snapshot.insert("Chrome".to_string(), (100, 200));
        snapshot.insert("Firefox".to_string(), (300, 400));
        snapshot.insert("Notepad".to_string(), (500, 600));

        let vault = make_vault_names(&["Chrome", "Firefox"]);
        let result = filter_vault_positions(&snapshot, &vault);

        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|(n, _, _)| *n).collect();
        assert!(names.contains(&"Chrome"));
        assert!(names.contains(&"Firefox"));
        assert!(!names.contains(&"Notepad"));
    }

    #[test]
    fn test_case_insensitive_matching() {
        let mut snapshot = HashMap::new();
        snapshot.insert("CHROME".to_string(), (100, 200));
        snapshot.insert("firefox".to_string(), (300, 400));

        // Vault names are stored lowercase
        let vault = make_vault_names(&["chrome", "FIREFOX"]);
        let result = filter_vault_positions(&snapshot, &vault);

        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_preserves_coordinates() {
        let mut snapshot = HashMap::new();
        snapshot.insert("Chrome".to_string(), (42, 84));

        let vault = make_vault_names(&["Chrome"]);
        let result = filter_vault_positions(&snapshot, &vault);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "Chrome");
        assert_eq!(result[0].1, 42);
        assert_eq!(result[0].2, 84);
    }

    #[test]
    fn test_no_match_returns_empty() {
        let mut snapshot = HashMap::new();
        snapshot.insert("Notepad".to_string(), (100, 200));
        snapshot.insert("Calc".to_string(), (300, 400));

        let vault = make_vault_names(&["Chrome", "Firefox"]);
        let result = filter_vault_positions(&snapshot, &vault);

        assert!(result.is_empty());
    }

    #[test]
    fn test_chinese_icon_names() {
        let mut snapshot = HashMap::new();
        snapshot.insert("æ­¤çµè".to_string(), (100, 200));
        snapshot.insert("åæ¶ç«".to_string(), (300, 400));
        snapshot.insert("Chrome".to_string(), (500, 600));

        let vault = make_vault_names(&["æ­¤çµè"]);
        let result = filter_vault_positions(&snapshot, &vault);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, 100);
        assert_eq!(result[0].2, 200);
    }

    #[test]
    fn test_all_icons_in_vault() {
        let mut snapshot = HashMap::new();
        snapshot.insert("A".to_string(), (10, 20));
        snapshot.insert("B".to_string(), (30, 40));
        snapshot.insert("C".to_string(), (50, 60));

        let vault = make_vault_names(&["A", "B", "C"]);
        let result = filter_vault_positions(&snapshot, &vault);

        assert_eq!(result.len(), 3);
    }
}
