//! 分类框组件
//!
//! 可拖拽、可调整尺寸的分类框容器

use crate::desktop::DesktopIcon;
use crate::ui::IconWidget;
use eframe::egui;
use serde::{Deserialize, Serialize};

/// 分类框样式
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BoxStyle {
    /// 背景颜色 (R, G, B, A)
    pub background_color: (u8, u8, u8, u8),
    /// 圆角半径
    pub border_radius: f32,
    /// 边框颜色 (R, G, B, A)
    pub border_color: (u8, u8, u8, u8),
    /// 边框宽度
    pub border_width: f32,
}

impl BoxStyle {
    /// 获取 egui Color32
    pub fn get_background_color(&self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(
            self.background_color.0,
            self.background_color.1,
            self.background_color.2,
            self.background_color.3,
        )
    }

    /// 获取 egui Color32
    pub fn get_border_color(&self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(
            self.border_color.0,
            self.border_color.1,
            self.border_color.2,
            self.border_color.3,
        )
    }
}

impl Default for BoxStyle {
    fn default() -> Self {
        Self {
            background_color: (200, 200, 220, 150),
            border_radius: 12.0,
            border_color: (255, 255, 255, 80),
            border_width: 1.0,
        }
    }
}

/// 分类框组件
pub struct ClassificationBoxWidget {
    /// 框 ID
    pub id: uuid::Uuid,
    /// 框名称
    pub name: String,
    /// 框图标
    pub icon: String,
    /// 位置
    pub position: egui::Pos2,
    /// 尺寸
    pub size: egui::Vec2,
    /// 样式
    pub style: BoxStyle,
    /// 图标列表
    pub icons: Vec<IconWidget>,
    /// 是否锁定
    pub locked: bool,
}

impl ClassificationBoxWidget {
    pub fn new(id: String, name: String, desktop_icons: Vec<DesktopIcon>) -> Self {
        let icons = desktop_icons
            .into_iter()
            .map(|di| IconWidget::new(di))
            .collect();

        Self {
            id: uuid::Uuid::new_v4(),
            name,
            icon: "📁".to_string(),
            position: egui::Pos2::new(50.0, 100.0),
            size: egui::Vec2::new(300.0, 200.0),
            style: BoxStyle::default(),
            icons,
            locked: false,
        }
    }

    /// 显示分类框
    pub fn show(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new(self.id))
            .pivot(egui::Align2::LEFT_TOP)
            .current_pos(self.position)
            .movable(!self.locked)
            .show(ctx, |ui| {
                // 分类框容器
                egui::Frame::none()
                    .fill(self.style.get_background_color())
                    .rounding(egui::Rounding::same(self.style.border_radius))
                    .stroke(egui::Stroke::new(
                        self.style.border_width,
                        self.style.get_border_color(),
                    ))
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        // 标题栏
                        ui.horizontal(|ui| {
                            ui.label(&self.icon);
                            ui.label(&self.name);
                            if self.locked {
                                ui.label("🔒");
                            }
                            if ui.button("⚙").clicked() {
                                // TODO: 打开设置菜单
                            }
                        });

                        ui.separator();

                        // 图标网格
                        egui::Grid::new(egui::Id::new(format!("grid_{}", self.id)))
                            .num_columns(4)
                            .spacing(egui::vec2(10.0, 10.0))
                            .show(ui, |ui| {
                                for icon in &self.icons {
                                    icon.show(ui);
                                    ui.end_row();
                                }
                            });
                    });

                // 更新位置（拖拽后）
                self.position = ui.max_rect().min;
            });

        // 尺寸调整（边缘拖拽）
        // TODO: 实现边缘拖拽调整尺寸
    }

    /// 锁定分类框
    pub fn lock(&mut self) {
        self.locked = true;
    }

    /// 解锁分类框
    pub fn unlock(&mut self) {
        self.locked = false;
    }
}
