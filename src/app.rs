//! 控制面板 - egui 主窗口
//!
//! 通过 `AppContext` 统一管理配置、平台服务和分类器。
//! 创建分类框使用独立的 egui viewport（独立窗口，非控制面板内弹窗）。
//!
//! 右键菜单触发方式：
//! - 分类框空白区域右键 → IContextMenu 编程式注入 → PENDING_CREATE_BOX 原子标志
//! - 桌面空白区域右键 → 注册表注入（程序退出时自动清理）→ --create-box → 信号文件

use crate::context::AppContext;
use crate::platform::{
    cleanup_desktop_registry, collect_grid_layout, create_grid_window, destroy_all_grid_windows,
    extract_file_icon, get_cursor_monitor_work_area_phys, get_grid_count, get_screen_resolution,
    hide_native_icons, reposition_window_by_title, restore_native_icons, setup_desktop_registry,
    take_pending_create_box, DesktopIcon, DesktopManagerTrait, GridCreateParams, GridFileItem,
};
use eframe::egui;
use std::sync::{Arc, Mutex};

/// 新建分类框名称最大长度（字符数）
const BOX_NAME_MAX_LEN: usize = 16;

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
    /// 创建分类框对话框状态（Arc<Mutex> 用于跨 viewport 共享）
    dialog: Arc<Mutex<DialogState>>,
}

impl Default for MyApp {
    fn default() -> Self {
        let ctx = AppContext::new().unwrap_or_default();
        Self {
            ctx,
            initialized: false,
            dialog: Arc::new(Mutex::new(DialogState::default())),
        }
    }
}

impl MyApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
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
            x, y, work_x, work_y, work_w, work_h
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
        let state = self.dialog.lock().map(|s| s.open_state).unwrap_or(DialogOpenState::Closed);

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
        let target_pos = self
            .dialog
            .lock()
            .map(|s| s.target_pos)
            .unwrap_or(None);
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
            return;
        }
        let boxes = &self.ctx.config.layout.boxes;
        if boxes.is_empty() {
            return;
        }
        log::info!("开始恢复布局: {} 个分类框", boxes.len());

        if !self.ctx.state.icons_hidden {
            if let Err(e) = hide_native_icons() {
                self.ctx.set_status(format!("隐藏图标失败: {}", e));
                return;
            }
            self.ctx.state.icons_hidden = true;
        }
        destroy_all_grid_windows();

        let mut created_count = 0;
        for (i, box_data) in boxes.iter().enumerate() {
            let mut items: Vec<GridFileItem> = Vec::new();
            for file_path in &box_data.icon_names {
                let display_name = std::path::Path::new(file_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(file_path)
                    .to_string();
                let hicon = extract_file_icon(file_path)
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
            };
            if create_grid_window(params).is_ok() {
                created_count += 1;
            }
        }
        self.ctx
            .set_status(format!("恢复了 {} 个分类框", created_count));
    }

    fn save_layout_and_config(&mut self) {
        let layout = collect_grid_layout();
        self.ctx.config.layout.boxes = layout;
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
        if !self.ctx.state.icons_hidden {
            if let Err(e) = hide_native_icons() {
                self.ctx.set_status(format!("隐藏图标失败: {}", e));
                return;
            }
            self.ctx.state.icons_hidden = true;
        }
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
        let _ = self.ctx.save_config();
        destroy_all_grid_windows();
        if self.ctx.state.icons_hidden {
            let _ = restore_native_icons();
            self.ctx.state.icons_hidden = false;
        }
        self.ctx.set_status("已清除，桌面图标已恢复");
    }

    fn create_empty_box(&mut self, name: String) {
        log::info!("创建分类框: '{}'", name);
        if !self.ctx.state.icons_hidden {
            if let Err(e) = hide_native_icons() {
                self.ctx.set_status(format!("隐藏图标失败: {}", e));
                return;
            }
            self.ctx.state.icons_hidden = true;
        }
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
        };
        match create_grid_window(params) {
            Ok(_) => {
                let layout = collect_grid_layout();
                self.ctx.config.layout.boxes = layout;
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
            self.restore_layout();
            // 先清理旧版残留，再注册新版（退出时清理，保证无残留）
            cleanup_desktop_registry();
            setup_desktop_registry();
        }

        // 信号文件（--create-box 从注册表菜单写入）→ 打开对话框
        let signal_path = signal_file_path();
        if signal_path.exists() {
            let _ = std::fs::remove_file(&signal_path);
            self.open_create_box_dialog();
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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

        ctx.request_repaint_after(std::time::Duration::from_millis(300));
    }
}

impl Drop for MyApp {
    fn drop(&mut self) {
        let layout = collect_grid_layout();
        if !layout.is_empty() {
            self.ctx.config.layout.boxes = layout;
            let _ = self.ctx.save_config();
        }
        destroy_all_grid_windows();
        if self.ctx.state.icons_hidden {
            let _ = restore_native_icons();
        }
        // 程序退出时清理注册表，确保不运行时桌面菜单完全原生
        cleanup_desktop_registry();
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
}

/// 验证分类框名称是否有效（非空且不超过 BOX_NAME_MAX_LEN 字符）
fn is_box_name_valid(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= BOX_NAME_MAX_LEN
}
