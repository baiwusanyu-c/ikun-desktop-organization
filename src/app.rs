//! egui 应用实现

use eframe::egui;
use crate::desktop::{DesktopIcon, DesktopManager, DesktopManagerTrait};
use crate::classification::AppClassifier;
use crate::ui::ClassificationBoxWidget;
use crate::config::DesktopLayout;

pub struct MyApp {
    /// 桌面管理器
    desktop_manager: DesktopManager,
    /// 应用分类器
    classifier: AppClassifier,
    /// 分类框列表
    classification_boxes: Vec<ClassificationBoxWidget>,
    /// 布局配置
    layout: DesktopLayout,
    /// 是否已初始化
    initialized: bool,
}

impl Default for MyApp {
    fn default() -> Self {
        // 加载布局配置（使用默认值如果失败）
        let layout = DesktopLayout::load()
            .unwrap_or_else(|e| {
                log::warn!("加载布局失败: {}, 使用默认布局", e);
                DesktopLayout::default()
            });

        // 初始化桌面管理器
        let desktop_manager = DesktopManager::new()
            .unwrap_or_else(|e| {
                log::warn!("初始化桌面管理器失败: {}", e);
                DesktopManager::default()
            });

        // 初始化分类器
        let classifier = AppClassifier::new()
            .unwrap_or_else(|e| {
                log::warn!("初始化分类器失败: {}", e);
                AppClassifier::default()
            });

        Self {
            desktop_manager,
            classifier,
            classification_boxes: Vec::new(),
            layout,
            initialized: false,
        }
    }
}

impl MyApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    /// 执行一键智能分类
    fn perform_auto_classification(&mut self) {
        log::info!("执行一键智能分类...");

        // 获取桌面图标
        let icons = self.desktop_manager.get_desktop_icons();

        // 分类每个图标
        let mut classified: std::collections::HashMap<String, Vec<DesktopIcon>> =
            std::collections::HashMap::new();

        for icon in icons {
            if let Some(category) = self.classifier.classify(&icon) {
                classified.entry(category.id.clone())
                    .or_default()
                    .push(icon);
            }
        }

        // 创建分类框
        self.classification_boxes.clear();
        for (category_id, icons) in classified {
            if let Some(category) = self.classifier.get_category(&category_id) {
                let box_widget = ClassificationBoxWidget::new(
                    category.id,
                    category.name,
                    icons,
                );
                self.classification_boxes.push(box_widget);
            }
        }

        log::info!("分类完成，创建了 {} 个分类框", self.classification_boxes.len());
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 初始化（首次运行）
        if !self.initialized {
            self.perform_auto_classification();
            self.initialized = true;
        }

        // 透明背景面板
        egui::CentralPanel::default().show(ctx, |ui| {
            // 控制栏
            ui.horizontal(|ui| {
                ui.heading("Desktop Organizer MVP");
                if ui.button("🔄 一键分类").clicked() {
                    self.perform_auto_classification();
                }
                if ui.button("💾 保存布局").clicked() {
                    if let Err(e) = self.layout.save() {
                        log::error!("保存布局失败: {}", e);
                    }
                }
            });

            ui.separator();

            // 显示分类框
            for box_widget in &mut self.classification_boxes {
                box_widget.show(ctx);
            }
        });

        // 每秒刷新一次（用于动态更新）
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
}