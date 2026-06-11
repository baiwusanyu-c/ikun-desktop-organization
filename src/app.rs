//! 控制面板 - egui 主窗口

use crate::classification::AppClassifier;
use crate::config::DesktopLayout;
use crate::desktop::{
    create_grid_window, destroy_all_grid_windows, extract_file_icon, get_grid_count,
    hide_native_icons, restore_native_icons, DesktopIcon, DesktopManager, DesktopManagerTrait,
    GridCreateParams, GridFileItem,
};
use eframe::egui;

/// 主应用
pub struct MyApp {
    desktop_manager: DesktopManager,
    classifier: AppClassifier,
    layout: DesktopLayout,
    initialized: bool,
    status_message: String,
    icons_hidden: bool,
}

impl Default for MyApp {
    fn default() -> Self {
        let layout = DesktopLayout::load().unwrap_or_default();
        let desktop_manager = DesktopManager::new().unwrap_or_default();
        let classifier = AppClassifier::new().unwrap_or_default();

        Self {
            desktop_manager,
            classifier,
            layout,
            initialized: false,
            status_message: String::new(),
            icons_hidden: false,
        }
    }
}

impl MyApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    /// 一键分类：隐藏原生图标 → 固定 3 个分类框（游戏/开发工具/办公）
    fn perform_auto_classify(&mut self) {
        log::info!("=== 一键分类开始 ===");

        // 1. 隐藏原生图标
        if !self.icons_hidden {
            if let Err(e) = hide_native_icons() {
                self.status_message = format!("隐藏图标失败: {}", e);
                log::error!("{}", self.status_message);
                return;
            }
            self.icons_hidden = true;
        }

        // 2. 获取桌面图标
        let icons = self.desktop_manager.get_desktop_icons();
        if icons.is_empty() {
            self.status_message = "未找到桌面图标".to_string();
            return;
        }

        // 3. 固定分为 3 类：游戏、开发工具、办公（剩余全部归入办公）
        let mut games: Vec<DesktopIcon> = Vec::new();
        let mut dev_tools: Vec<DesktopIcon> = Vec::new();
        let mut office: Vec<DesktopIcon> = Vec::new();

        for icon in &icons {
            match self.classifier.classify(icon) {
                Some(cat) if cat.id == "games" => games.push(icon.clone()),
                Some(cat) if cat.id == "dev" => dev_tools.push(icon.clone()),
                _ => office.push(icon.clone()),
            }
        }

        log::info!(
            "分类结果: 游戏={}, 开发工具={}, 办公={}",
            games.len(),
            dev_tools.len(),
            office.len()
        );

        // 4. 销毁旧格子
        destroy_all_grid_windows();

        // 5. 创建固定 3 个格子窗口
        let categories: Vec<(&str, &Vec<DesktopIcon>)> = vec![
            ("游戏", &games),
            ("开发工具", &dev_tools),
            ("办公", &office),
        ];

        let box_w: i32 = 280;
        let box_h: i32 = 400;
        let padding: i32 = 20;
        let start_x: i32 = 30;
        let start_y: i32 = 80;

        let mut created_count = 0;
        for (i, (title, icon_list)) in categories.iter().enumerate() {
            if icon_list.is_empty() {
                log::info!("分类 '{}' 为空，跳过", title);
                continue;
            }

            let x = start_x + (created_count as i32) * (box_w + padding);
            let y = start_y;

            // 提取文件图标并构建 GridFileItem 列表
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
                y,
                width: box_w,
                height: box_h,
                items,
            };

            if let Err(e) = create_grid_window(params) {
                log::error!("创建格子窗口失败: {}", e);
            } else {
                created_count += 1;
            }
        }

        self.status_message = format!(
            "分类完成！隐藏了 {} 个图标，创建了 {} 个格子",
            icons.len(),
            get_grid_count()
        );
        log::info!("{}", self.status_message);
    }

    /// 清除所有格子，恢复原生图标
    fn clear_and_restore(&mut self) {
        destroy_all_grid_windows();

        if self.icons_hidden {
            if let Err(e) = restore_native_icons() {
                self.status_message = format!("恢复图标失败: {}", e);
                return;
            }
            self.icons_hidden = false;
        }

        self.status_message = "已清除所有格子，桌面图标已恢复".to_string();
        log::info!("{}", self.status_message);
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.initialized = true;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("📦 Desktop Organizer");
            ui.separator();

            // 操作按钮
            ui.horizontal(|ui| {
                if ui.button("🔄 一键分类").clicked() {
                    self.perform_auto_classify();
                }
                if ui.button("🗑 清除 & 恢复").clicked() {
                    self.clear_and_restore();
                }
                if ui.button("💾 保存布局").clicked() {
                    if let Err(e) = self.layout.save() {
                        log::error!("保存失败: {}", e);
                    }
                }
            });

            ui.separator();

            // 状态
            if !self.status_message.is_empty() {
                ui.colored_label(egui::Color32::from_rgb(0, 200, 0), &self.status_message);
            }

            if self.icons_hidden {
                ui.colored_label(egui::Color32::YELLOW, "⚠ 原生图标已隐藏");
            }

            ui.label(format!("当前格子数: {}", get_grid_count()));

            ui.separator();
            ui.label("操作说明:");
            ui.label("• 点击「一键分类」→ 自动隐藏桌面图标 + 创建 3 个分类格子");
            ui.label("• 拖动格子标题栏 → 整体移动");
            ui.label("• 拖动格子边缘 → 调整大小");
            ui.label("• 双击格子内图标 → 打开应用");
            ui.label("• 右键格子内图标 → 弹出菜单");
            ui.label("• 点击「清除 & 恢复」→ 恢复原始桌面");
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

impl Drop for MyApp {
    fn drop(&mut self) {
        // 程序退出时恢复图标
        destroy_all_grid_windows();
        if self.icons_hidden {
            let _ = restore_native_icons();
        }
    }
}
