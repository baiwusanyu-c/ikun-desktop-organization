//! GPUI 应用运行时 — 与 UI 框架无关的业务逻辑层

#![allow(dead_code)] // GPUI 应用运行时，部分方法/类型预留
//!
//! 完整移植自 app.rs（eframe 版本）的所有业务逻辑。
//! 使用 Arc<Mutex<>> 保护状态，避免 GPUI RefCell 借用冲突。

use crate::config::app_config::AppConfig;
use crate::context::AppContext;
use crate::platform::{
    cleanup_desktop_registry, cleanup_system_tray, collect_grid_layout, create_desktop_overlay,
    create_grid_window, destroy_all_grid_windows, destroy_desktop_overlay, extract_file_icon,
    get_grid_count, get_primary_monitor_dpi, get_screen_resolution, hide_control_panel_window,
    hide_native_icons, icon_exists_for_box, native_icons_hidden, resolve_icon_read_path,
    restore_all_hidden_icons, restore_native_icons, setup_desktop_registry, setup_system_tray,
    show_control_panel_window, take_pending_create_box, take_pending_tray_action, DesktopIcon,
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
pub fn should_retry_native_icon_hide(state_icons_hidden: bool, native_hidden: bool) -> bool {
    state_icons_hidden && !native_hidden
}

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

        self.ensure_desktop_overlay_running();

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

        // 2. 检查是否需要重新隐藏原生图标
        if should_retry_native_icon_hide(self.ctx.state.icons_hidden, native_icons_hidden()) {
            log::warn!("检测到 Explorer 桌面图标窗口恢复，重新隐藏");
            self.ensure_desktop_overlay_running();
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

        // 4. Shell 右键菜单待创建分类框
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

    // =================================================================
    // 业务逻辑
    // =================================================================

    pub fn perform_auto_classify(&mut self) {
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

    pub fn clear_and_restore(&mut self) {
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
        self.ensure_desktop_overlay_running();

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

    fn restore_layout(&mut self) {
        if !self.ctx.config.general.restore_layout_on_start {
            log::info!("启动恢复布局已禁用");
            return;
        }

        restore_all_hidden_icons();
        self.ensure_native_icons_hidden();
        if let Err(e) = create_desktop_overlay() {
            log::warn!("创建桌面覆盖窗口失败: {}", e);
        }

        let boxes = self.ctx.config.layout.boxes.clone();
        if boxes.is_empty() {
            log::info!("启动布局为空");
            self.ctx.set_status("桌面覆盖窗口已启用");
            return;
        }
        log::info!("开始恢复布局: {} 个分类框", boxes.len());

        destroy_all_grid_windows();

        let mut created_count = 0;
        for (i, box_data) in boxes.iter().enumerate() {
            let mut items: Vec<GridFileItem> = Vec::new();
            for file_path in &box_data.icon_names {
                if !icon_exists_for_box(file_path) {
                    log::info!("跳过已删除的图标: {}", file_path);
                    continue;
                }
                let read_path = resolve_icon_read_path(file_path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.clone());
                let display_name = std::path::Path::new(file_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(file_path)
                    .to_string();
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

    fn ensure_native_icons_hidden(&mut self) -> bool {
        if self.ctx.state.icons_hidden && native_icons_hidden() {
            return true;
        }
        if self.ctx.state.icons_hidden {
            log::warn!("原生桌面图标窗口已恢复，尝试重新隐藏");
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

    fn merge_latest_overlay_config(&mut self) {
        match AppConfig::load() {
            Ok(config) => {
                self.ctx.config.overlay = config.overlay;
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

    fn cleanup(&mut self) {
        log::info!("执行清理");

        let layout = collect_grid_layout();
        if !layout.is_empty() {
            self.ctx.config.layout.boxes = layout;
            self.merge_latest_overlay_config();
            if let Err(e) = self.ctx.save_config() {
                log::warn!("退出前保存布局失败: {}", e);
            }
        }

        destroy_all_grid_windows();
        destroy_desktop_overlay();
        cleanup_system_tray();
        cleanup_desktop_registry();

        if self.ctx.state.icons_hidden {
            if let Err(e) = restore_native_icons() {
                log::warn!("退出时恢复原生桌面图标失败: {}", e);
            }
            self.ctx.state.icons_hidden = false;
        }
        restore_all_hidden_icons();
    }

    // =================================================================
    // 状态读取（UI 层用）
    // =================================================================

    pub fn status_message(&self) -> &str {
        &self.ctx.state.status_message
    }

    pub fn icons_hidden(&self) -> bool {
        self.ctx.state.icons_hidden
    }

    pub fn box_count(&self) -> usize {
        get_grid_count()
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
    fn test_should_retry_native_icon_hide() {
        assert!(should_retry_native_icon_hide(true, false));
        assert!(!should_retry_native_icon_hide(true, true));
        assert!(!should_retry_native_icon_hide(false, false));
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
