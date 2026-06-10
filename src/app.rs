//! egui 应用实现
//!
//! 架构：
//! - 主窗口 = 控制面板（egui，正常窗口）
//! - 分类框 = Win32 原生 overlay（GDI 绘制，鼠标穿透，在桌面上方）
//! - 桌面图标 = SysListView32 控制移动

use crate::classification::AppClassifier;
use crate::config::DesktopLayout;
use crate::desktop::{
    create_shared_box_data, start_overlay, BoxDrawData, DesktopIcon, DesktopIconInfo,
    DesktopIconMover, DesktopManager, DesktopManagerTrait, SharedBoxData,
};
use eframe::egui;

pub struct MyApp {
    desktop_manager: DesktopManager,
    icon_mover: Option<DesktopIconMover>,
    classifier: AppClassifier,
    layout: DesktopLayout,
    initialized: bool,
    status_message: String,
    selected_box: Option<usize>,

    /// Win32 overlay 共享数据
    overlay_data: SharedBoxData,

    /// 分类框数据（与 overlay 同步）
    boxes: Vec<BoxInfo>,
}

/// 分类框信息
struct BoxInfo {
    name: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    icon_names: Vec<String>,
    icon_indices: Vec<i32>, // SysListView32 indices
}

impl Default for MyApp {
    fn default() -> Self {
        let layout = DesktopLayout::load().unwrap_or_else(|e| {
            log::warn!("加载布局失败: {}, 使用默认布局", e);
            DesktopLayout::default()
        });

        let desktop_manager = DesktopManager::new().unwrap_or_else(|e| {
            log::warn!("初始化桌面管理器失败: {}", e);
            DesktopManager::default()
        });

        let classifier = AppClassifier::new().unwrap_or_else(|e| {
            log::warn!("初始化分类器失败: {}", e);
            AppClassifier::default()
        });

        let icon_mover = DesktopIconMover::new()
            .ok()
            .or_else(|| {
                log::warn!("DesktopIconMover 初始化失败");
                None
            });

        // 创建 overlay 共享数据并启动 Win32 overlay
        let overlay_data = create_shared_box_data();
        if let Err(e) = start_overlay(overlay_data.clone()) {
            log::error!("启动 overlay 失败: {}", e);
        }

        Self {
            desktop_manager,
            icon_mover,
            classifier,
            layout,
            initialized: false,
            status_message: String::new(),
            selected_box: None,
            overlay_data,
            boxes: Vec::new(),
        }
    }
}

impl MyApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    /// 同步分类框数据到 Win32 overlay
    fn sync_overlay(&self) {
        let draw_data: Vec<BoxDrawData> = self
            .boxes
            .iter()
            .map(|b| BoxDrawData {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
                title: b.name.clone(),
                icon_count: b.icon_names.len(),
                icon_names: b.icon_names.clone(),
            })
            .collect();

        if let Ok(mut data) = self.overlay_data.lock() {
            *data = draw_data;
        }
    }

    /// 执行一键智能分类
    fn perform_auto_classification(&mut self) {
        log::info!("执行一键智能分类...");

        let icons = self.desktop_manager.get_desktop_icons();
        if icons.is_empty() {
            self.status_message = "未找到桌面图标".to_string();
            return;
        }

        let listview_icons = self
            .icon_mover
            .as_ref()
            .and_then(|m| m.get_all_icons().ok())
            .unwrap_or_default();

        log::info!(
            "文件系统: {} 个图标, SysListView32: {} 个图标",
            icons.len(),
            listview_icons.len()
        );

        // 分类
        let mut classified: std::collections::HashMap<String, Vec<(DesktopIcon, Option<i32>)>> =
            std::collections::HashMap::new();

        for icon in &icons {
            if let Some(category) = self.classifier.classify(icon) {
                let lv_index = find_listview_index(&icon.display_name, &listview_icons);
                classified
                    .entry(category.id.clone())
                    .or_default()
                    .push((icon.clone(), lv_index));
            }
        }

        // 创建分类框
        self.boxes.clear();

        let box_width: i32 = 200;
        let box_height: i32 = 250;
        let padding: i32 = 30;
        let start_x: i32 = 50;
        let start_y: i32 = 100;

        for (i, (category_id, icon_pairs)) in classified.iter().enumerate() {
            if let Some(category) = self.classifier.get_category(category_id) {
                let x = start_x + (i as i32) * (box_width + padding);
                let y = start_y;

                let icon_names: Vec<String> = icon_pairs
                    .iter()
                    .map(|(icon, _)| icon.display_name.clone())
                    .collect();
                let icon_indices: Vec<i32> = icon_pairs
                    .iter()
                    .filter_map(|(_, idx)| *idx)
                    .collect();

                self.boxes.push(BoxInfo {
                    name: category.name.clone(),
                    x,
                    y,
                    width: box_width,
                    height: box_height,
                    icon_names,
                    icon_indices,
                });
            }
        }

        // 移动真实桌面图标到框内
        self.move_icons_into_boxes();

        // 同步到 overlay
        self.sync_overlay();

        self.status_message = format!("分类完成，创建了 {} 个分类框", self.boxes.len());
        log::info!("{}", self.status_message);
    }

    /// 将图标移动到对应框的区域
    fn move_icons_into_boxes(&self) {
        let mover = match &self.icon_mover {
            Some(m) => m,
            None => return,
        };

        for box_info in &self.boxes {
            if box_info.icon_indices.is_empty() {
                continue;
            }

            let content_top = box_info.y + 40;
            let icon_spacing: i32 = 80;
            let cols: i32 = 3;

            let mut moves: Vec<(i32, i32, i32)> = Vec::new();
            for (i, &index) in box_info.icon_indices.iter().enumerate() {
                let col = (i as i32) % cols;
                let row = (i as i32) / cols;
                let target_x = box_info.x + 15 + col * icon_spacing;
                let target_y = content_top + 10 + row * icon_spacing;
                moves.push((index, target_x, target_y));
            }

            if let Err(e) = mover.move_icons_to_positions(&moves) {
                log::error!("移动图标到框 [{}] 失败: {}", box_info.name, e);
            }
        }
    }

    /// 移动选中的框
    fn move_selected_box(&mut self, dx: i32, dy: i32) {
        if let Some(idx) = self.selected_box {
            if idx < self.boxes.len() {
                self.boxes[idx].x += dx;
                self.boxes[idx].y += dy;

                // 移动真实图标
                if let Some(mover) = &self.icon_mover {
                    let indices = &self.boxes[idx].icon_indices;
                    if !indices.is_empty() {
                        let _ = mover.move_icons_by_offset(indices, dx, dy);
                    }
                }

                // 同步 overlay
                self.sync_overlay();
            }
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.initialized {
            self.initialized = true;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("📦 Desktop Organizer");
            ui.separator();

            // === 操作按钮 ===
            ui.horizontal(|ui| {
                if ui.button("🔄 一键分类").clicked() {
                    self.perform_auto_classification();
                }
                if ui.button("💾 保存布局").clicked() {
                    if let Err(e) = self.layout.save() {
                        log::error!("保存布局失败: {}", e);
                    }
                }
                if ui.button("🗑 清除框").clicked() {
                    self.boxes.clear();
                    self.sync_overlay();
                    self.status_message = "已清除所有分类框".to_string();
                }
            });

            // === 状态 ===
            if !self.status_message.is_empty() {
                ui.colored_label(egui::Color32::from_rgb(0, 200, 0), &self.status_message);
            }

            ui.horizontal(|ui| {
                if self.icon_mover.is_some() {
                    ui.colored_label(egui::Color32::from_rgb(0, 180, 0), "✅ 图标控制已就绪");
                } else {
                    ui.colored_label(egui::Color32::YELLOW, "⚠ 图标控制不可用");
                }
            });

            ui.separator();

            // === 分类框列表 ===
            ui.heading(format!("分类框 ({})", self.boxes.len()));

            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for (i, box_info) in self.boxes.iter().enumerate() {
                    let selected = self.selected_box == Some(i);
                    let text = format!(
                        "📁 {} ({} 个图标) @ ({},{})",
                        box_info.name,
                        box_info.icon_names.len(),
                        box_info.x,
                        box_info.y,
                    );
                    if ui.selectable_label(selected, &text).clicked() {
                        self.selected_box = Some(i);
                    }
                }
            });

            ui.separator();

            // === 移动控制 ===
            if let Some(idx) = self.selected_box {
                if idx < self.boxes.len() {
                    ui.heading(format!("移动: {}", self.boxes[idx].name));

                    let step = 30;
                    ui.horizontal(|ui| {
                        if ui.button("⬅ 左").clicked() {
                            self.move_selected_box(-step, 0);
                        }
                        if ui.button("➡ 右").clicked() {
                            self.move_selected_box(step, 0);
                        }
                        if ui.button("⬆ 上").clicked() {
                            self.move_selected_box(0, -step);
                        }
                        if ui.button("⬇ 下").clicked() {
                            self.move_selected_box(0, step);
                        }
                    });

                    ui.horizontal(|ui| {
                        if ui.button("⬅⬅ 大步左").clicked() {
                            self.move_selected_box(-step * 5, 0);
                        }
                        if ui.button("➡➡ 大步右").clicked() {
                            self.move_selected_box(step * 5, 0);
                        }
                        if ui.button("⬆⬆ 大步上").clicked() {
                            self.move_selected_box(0, -step * 5);
                        }
                        if ui.button("⬇⬇ 大步下").clicked() {
                            self.move_selected_box(0, step * 5);
                        }
                    });
                }
            } else if !self.boxes.is_empty() {
                ui.label("👆 选择一个框以移动它");
            } else {
                ui.label("点击「一键分类」创建分类框");
            }
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}

/// 在 SysListView32 中查找匹配名称的图标 index
fn find_listview_index(name: &str, listview_icons: &[DesktopIconInfo]) -> Option<i32> {
    if let Some(icon) = listview_icons.iter().find(|i| i.name == name) {
        return Some(icon.index);
    }

    let name_lower = name.to_lowercase();
    if let Some(icon) = listview_icons.iter().find(|i| {
        let stem = i
            .name
            .strip_suffix(".lnk")
            .or_else(|| i.name.strip_suffix(".url"))
            .or_else(|| i.name.strip_suffix(".exe"))
            .unwrap_or(&i.name);
        stem.to_lowercase() == name_lower
    }) {
        return Some(icon.index);
    }

    if let Some(icon) = listview_icons.iter().find(|i| {
        i.name.to_lowercase().contains(&name_lower)
            || name_lower.contains(&i.name.to_lowercase())
    }) {
        return Some(icon.index);
    }

    None
}
