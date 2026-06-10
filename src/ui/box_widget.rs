//! 分类框组件
//!
//! 在全屏透明 overlay（鼠标穿透）上渲染分类框的视觉边框。
//! 由于 overlay 完全穿透，分类框只做视觉渲染，不可交互。
//! 交互通过独立的控制面板窗口完成。

use crate::desktop::DesktopIcon;
use eframe::egui;
use serde::{Deserialize, Serialize};

/// 分类框样式
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BoxStyle {
    pub background_color: (u8, u8, u8, u8),
    pub border_radius: f32,
    pub border_color: (u8, u8, u8, u8),
    pub border_width: f32,
}

impl BoxStyle {
    pub fn get_background_color(&self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(
            self.background_color.0,
            self.background_color.1,
            self.background_color.2,
            self.background_color.3,
        )
    }

    pub fn get_border_color(&self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(
            self.border_color.0,
            self.border_color.1,
            self.border_color.2,
            self.border_color.3,
        )
    }

    pub fn get_title_bar_color(&self) -> egui::Color32 {
        let (r, g, b, _) = self.background_color;
        egui::Color32::from_rgba_unmultiplied(
            r.saturating_add(20),
            g.saturating_add(20),
            b.saturating_add(30),
            200,
        )
    }
}

impl Default for BoxStyle {
    fn default() -> Self {
        Self {
            background_color: (30, 30, 50, 60),
            border_radius: 10.0,
            border_color: (120, 120, 180, 120),
            border_width: 1.5,
        }
    }
}

/// 分类框组件（纯视觉渲染，不可交互）
pub struct ClassificationBoxWidget {
    pub id: uuid::Uuid,
    pub name: String,
    pub icon: String,
    /// 位置（屏幕坐标）
    pub position: egui::Pos2,
    /// 尺寸
    pub size: egui::Vec2,
    pub style: BoxStyle,
    /// 图标列表（显示名称用）
    pub icons: Vec<DesktopIcon>,
    /// 框内图标在 SysListView32 中的 indices
    pub icon_indices: Vec<i32>,
}

impl ClassificationBoxWidget {
    pub fn new(_id: String, name: String, desktop_icons: Vec<DesktopIcon>) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            name,
            icon: "📁".to_string(),
            position: egui::Pos2::new(50.0, 100.0),
            size: egui::Vec2::new(200.0, 250.0),
            style: BoxStyle::default(),
            icons: desktop_icons,
            icon_indices: Vec::new(),
        }
    }

    /// 在全屏 overlay 上渲染分类框（纯视觉，不捕获鼠标）
    pub fn show_on_overlay(&mut self, ctx: &egui::Context) {
        let box_rect = egui::Rect::from_min_size(self.position, self.size);

        // 使用 layer_painter 在最底层绘制（不阻挡任何鼠标事件）
        let painter = ctx.layer_painter(egui::layers::LayerId::background());

        // === 半透明背景 ===
        painter.rect_filled(
            box_rect,
            egui::Rounding::same(self.style.border_radius),
            self.style.get_background_color(),
        );

        // === 边框 ===
        painter.rect_stroke(
            box_rect,
            egui::Rounding::same(self.style.border_radius),
            egui::Stroke::new(self.style.border_width, self.style.get_border_color()),
        );

        // === 标题栏背景 ===
        let title_height = 28.0;
        let title_rect = egui::Rect::from_min_size(
            box_rect.min,
            egui::Vec2::new(self.size.x, title_height),
        );
        painter.rect_filled(
            title_rect,
            egui::Rounding {
                nw: self.style.border_radius,
                ne: self.style.border_radius,
                sw: 0.0,
                se: 0.0,
            },
            self.style.get_title_bar_color(),
        );

        // === 标题文字 ===
        painter.text(
            egui::Pos2::new(title_rect.center().x, title_rect.center().y),
            egui::Align2::CENTER_CENTER,
            format!("{} {}", self.icon, self.name),
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );

        // === 图标数量 ===
        painter.text(
            egui::Pos2::new(title_rect.max.x - 8.0, title_rect.center().y),
            egui::Align2::RIGHT_CENTER,
            format!("{}", self.icons.len()),
            egui::FontId::proportional(10.0),
            egui::Color32::from_gray(180),
        );

        // === 内容区域：图标名称列表 ===
        let content_top = box_rect.min.y + title_height + 6.0;
        let line_height = 18.0;
        let max_lines = ((box_rect.max.y - content_top - 5.0) / line_height) as usize;

        for (i, icon) in self.icons.iter().take(max_lines).enumerate() {
            let y = content_top + (i as f32) * line_height;
            painter.text(
                egui::Pos2::new(box_rect.min.x + 8.0, y),
                egui::Align2::LEFT_TOP,
                &icon.display_name,
                egui::FontId::proportional(11.0),
                egui::Color32::from_rgba_unmultiplied(200, 200, 220, 180),
            );
        }

        if self.icons.len() > max_lines {
            let y = content_top + (max_lines as f32) * line_height;
            painter.text(
                egui::Pos2::new(box_rect.min.x + 8.0, y),
                egui::Align2::LEFT_TOP,
                format!("+{} 更多", self.icons.len() - max_lines),
                egui::FontId::proportional(10.0),
                egui::Color32::from_rgba_unmultiplied(150, 150, 170, 150),
            );
        }
    }
}
