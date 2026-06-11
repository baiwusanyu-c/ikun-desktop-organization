//! 控制面板 - egui 主窗口

use crate::classification::AppClassifier;
use crate::config::DesktopLayout;
use crate::desktop::{
    create_grid_window, destroy_all_grid_windows, extract_file_icon, get_grid_count,
    hide_native_icons, restore_native_icons, DesktopIcon, DesktopManager, DesktopManagerTrait,
    GridCreateParams, GridFileItem,
};
use eframe::egui;

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

    /// 一键分类：隐藏原生图标 → 分类 → 创建格子窗口
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

        // 3. 分类
        let mut classified: std::collections::HashMap<String, Vec<DesktopIcon>> =
            std::collections::HashMap::new();

        for icon in &icons {
            if let Some(category) = self.classifier.classify(icon) {
                classified
                    .entry(category.id.clone())
                    .or_default()
                    .push(icon.clone());
            }
        }

        // 未分类的图标放入"其他"
        let mut others = Vec::new();
        for icon in &icons {
            let dominated = classified
                .values()
                .any(|v| v.iter().any(|i| i.id == icon.id));
            if !dominated {
                others.push(icon.clone());
            }
        }
        if !others.is_empty() {
            classified.insert("_other".to_string(), others);
        }

        // 4. 销毁旧格子
        destroy_all_grid_windows();

        // 5. 创建格子窗口
        let box_w: i32 = 240;
        let box_h: i32 = 300;
        let padding: i32 = 20;
        let start_x: i32 = 30;
        let start_y: i32 = 80;
        let icon_cols: i32 = 3;
        let icon_cell_w: i32 = 72;
        let icon_cell_h: i32 = 68;
        let title_h: i32 = 36;

        for (i, (category_id, icon_list)) in classified.iter().enumerate() {
            let title = if category_id == "_other" {
                "其他".to_string()
            } else {
                self.classifier
                    .get_category(category_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| category_id.clone())
            };

            let x = start_x + (i as i32) * (box_w + padding);
            let y = start_y;

            // 计算需要的行数以确定窗口高度
            let rows = ((icon_list.len() as i32) + icon_cols - 1) / icon_cols;
            let needed_h = title_h + 10 + rows * icon_cell_h + 20;
            let final_h = needed_h.max(box_h);

            // 提取文件图标
            let mut items: Vec<GridFileItem> = Vec::new();
            for (j, icon) in icon_list.iter().enumerate() {
                let col = (j as i32) % icon_cols;
                let row = (j as i32) / icon_cols;

                let client_x = 10 + col * icon_cell_w;
                let client_y = title_h + 8 + row * icon_cell_h;

                // 提取系统图标
                let path_str = icon.path.to_string_lossy().to_string();
                let hicon = extract_file_icon(&path_str)
                    .map(|fi| fi.hicon)
                    .unwrap_or(std::ptr::null_mut());

                items.push(GridFileItem {
                    file_path: path_str,
                    display_name: icon.display_name.clone(),
                    icon_handle: hicon,
                    client_x,
                    client_y,
                });
            }

            let params = GridCreateParams {
                id: i,
                title,
                x,
                y,
                width: box_w,
                height: final_h,
                items,
            };

            if let Err(e) = create_grid_window(params) {
                log::error!("创建格子窗口失败: {}", e);
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
            ui.label("• 点击「一键分类」→ 自动隐藏桌面图标 + 创建分类格子");
            ui.label("• 拖动格子标题栏 → 整体移动，图标跟随");
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
