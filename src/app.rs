//! 控制面板 - egui 主窗口
//!
//! 通过 `AppContext` 统一管理配置、平台服务和分类器。

use crate::context::AppContext;
use crate::platform::{
    collect_grid_layout, create_grid_window, destroy_all_grid_windows, extract_file_icon,
    get_grid_count, get_screen_resolution, hide_native_icons, restore_native_icons, DesktopIcon,
    DesktopManagerTrait, GridCreateParams, GridFileItem,
};
use eframe::egui;

/// 主应用
pub struct MyApp {
    /// 应用上下文（统一管理所有状态）
    ctx: AppContext,
    /// 是否已完成首次初始化（恢复布局等）
    initialized: bool,
}

impl Default for MyApp {
    fn default() -> Self {
        let ctx = AppContext::new().unwrap_or_default();
        Self {
            ctx,
            initialized: false,
        }
    }
}

impl MyApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    /// 根据配置文件恢复分类框布局
    ///
    /// 读取 `config.layout.boxes`，为每个 BoxLayoutData 创建格子窗口。
    /// 仅在 `config.general.restore_layout_on_start` 为 true 时执行。
    fn restore_layout(&mut self) {
        if !self.ctx.config.general.restore_layout_on_start {
            log::info!("配置 disable 了启动恢复布局，跳过");
            return;
        }

        let boxes = &self.ctx.config.layout.boxes;
        if boxes.is_empty() {
            log::info!("配置中无布局数据，跳过恢复");
            return;
        }

        log::info!("开始恢复布局: {} 个分类框", boxes.len());

        // 先隐藏原生图标
        if !self.ctx.state.icons_hidden {
            if let Err(e) = hide_native_icons() {
                self.ctx.set_status(format!("隐藏图标失败: {}", e));
                log::error!("{}", self.ctx.state.status_message);
                return;
            }
            self.ctx.state.icons_hidden = true;
        }

        // 销毁已有格子（防止重复创建）
        destroy_all_grid_windows();

        // 逐个恢复分类框
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

            if items.is_empty() {
                log::info!("分类框 '{}' 无有效文件，跳过", box_data.name);
                continue;
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

            if let Err(e) = create_grid_window(params) {
                log::error!("恢复分类框 '{}' 失败: {}", box_data.name, e);
            } else {
                created_count += 1;
            }
        }

        self.ctx
            .set_status(format!("布局恢复完成：恢复了 {} 个分类框", created_count));
        log::info!("{}", self.ctx.state.status_message);
    }

    /// 保存当前布局到配置并写入文件
    fn save_layout_and_config(&mut self) {
        // 1. 从运行中的格子窗口收集布局数据
        let layout = collect_grid_layout();
        self.ctx.config.layout.boxes = layout;

        // 2. 保存配置到 ~/.ikun_organizer.json
        if let Err(e) = self.ctx.save_config() {
            log::error!("保存配置失败: {}", e);
            self.ctx.set_status(format!("保存失败: {}", e));
        } else {
            let box_count = self.ctx.config.layout.boxes.len();
            self.ctx
                .set_status(format!("配置已保存（含 {} 个分类框布局）", box_count));
        }
    }

    /// 一键分类：隐藏原生图标 → 固定 3 个分类框（游戏/开发工具/办公）
    fn perform_auto_classify(&mut self) {
        log::info!("=== 一键分类开始 ===");

        // 1. 隐藏原生图标
        if !self.ctx.state.icons_hidden {
            if let Err(e) = hide_native_icons() {
                self.ctx.set_status(format!("隐藏图标失败: {}", e));
                log::error!("{}", self.ctx.state.status_message);
                return;
            }
            self.ctx.state.icons_hidden = true;
        }

        // 2. 获取桌面图标
        let icons = self.ctx.desktop_manager.get_desktop_icons();
        if icons.is_empty() {
            self.ctx.set_status("未找到桌面图标");
            return;
        }

        // 3. 固定分为 3 类：游戏、开发工具、办公（剩余全部归入办公）
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

        let (screen_w, screen_h) = get_screen_resolution();
        log::info!("屏幕分辨率: {}x{}", screen_w, screen_h);

        let num_categories: i32 = 3;
        let box_w: i32 = self.ctx.config.box_defaults.default_width;
        let box_h: i32 = self.ctx.config.box_defaults.default_height;
        let padding: i32 = 20;
        let total_w = num_categories * box_w + (num_categories - 1) * padding;
        let start_x: i32 = ((screen_w - total_w) / 2).max(10);
        let start_y: i32 = (screen_h as f32 * 0.08) as i32;

        log::info!(
            "框尺寸: {}x{}, padding={}, start=({}, {})",
            box_w,
            box_h,
            padding,
            start_x,
            start_y
        );

        let mut created_count = 0;
        for (i, (title, icon_list)) in categories.iter().enumerate() {
            if icon_list.is_empty() {
                log::info!("分类 '{}' 为空，跳过", title);
                continue;
            }

            let x = start_x + created_count * (box_w + padding);
            let y = start_y;

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
                view_mode: self.ctx.config.box_defaults.view_mode.clone(),
            };

            if let Err(e) = create_grid_window(params) {
                log::error!("创建格子窗口失败: {}", e);
            } else {
                created_count += 1;
            }
        }

        self.ctx.set_status(format!(
            "分类完成！隐藏了 {} 个图标，创建了 {} 个格子",
            icons.len(),
            get_grid_count()
        ));
        log::info!("{}", self.ctx.state.status_message);
    }

    /// 清除所有格子，恢复原生图标
    fn clear_and_restore(&mut self) {
        // 先收集当前布局保存到配置（保留最后一次布局记录）
        let layout = collect_grid_layout();
        self.ctx.config.layout.boxes = layout;
        let _ = self.ctx.save_config();

        destroy_all_grid_windows();

        if self.ctx.state.icons_hidden {
            if let Err(e) = restore_native_icons() {
                self.ctx.set_status(format!("恢复图标失败: {}", e));
                return;
            }
            self.ctx.state.icons_hidden = false;
        }

        self.ctx.set_status("已清除所有格子，桌面图标已恢复");
        log::info!("{}", self.ctx.state.status_message);
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 首次渲染时恢复布局
        if !self.initialized {
            self.initialized = true;
            self.restore_layout();
        }

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
                if ui.button("💾 保存配置").clicked() {
                    self.save_layout_and_config();
                }
            });

            ui.separator();

            // 状态
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
            ui.label("操作说明:");
            ui.label("• 点击「一键分类」→ 自动隐藏桌面图标 + 创建 3 个分类格子");
            ui.label("• 拖动格子标题栏 → 整体移动");
            ui.label("• 拖动格子边缘 → 调整大小");
            ui.label("• 双击格子内图标 → 打开应用");
            ui.label("• 右键格子内图标 → 弹出菜单");
            ui.label("• 点击「保存配置」→ 保存当前布局到 ~/.ikun_organizer.json");
            ui.label("• 点击「清除 & 恢复」→ 恢复原始桌面");
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

impl Drop for MyApp {
    fn drop(&mut self) {
        // 退出前自动保存布局
        let layout = collect_grid_layout();
        if !layout.is_empty() {
            self.ctx.config.layout.boxes = layout;
            let _ = self.ctx.save_config();
            log::info!("退出前自动保存布局");
        }

        destroy_all_grid_windows();
        if self.ctx.state.icons_hidden {
            let _ = restore_native_icons();
        }
    }
}
