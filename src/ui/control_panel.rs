//! 控制面板主界面
//!
//! 从 app.rs 拆分的控制面板 UI 组件。
//! TODO: 后续从 app.rs 迁移控制面板渲染逻辑到此处。

use crate::context::AppContext;

/// 渲染控制面板
pub fn render_control_panel(ctx: &mut AppContext, ui: &mut egui::Ui) {
    ui.heading("📦 Desktop Organizer");
    ui.separator();

    // 操作按钮
    ui.horizontal(|ui| {
        if ui.button("📦 创建分类框").clicked() {
            ctx.set_status("创建分类框功能待实现");
        }
        if ui.button("🤖 一键 AI 分类").clicked() {
            ctx.set_status("一键 AI 分类功能待实现");
        }
    });

    ui.separator();

    // 状态消息
    if !ctx.state.status_message.is_empty() {
        ui.colored_label(
            egui::Color32::from_rgb(0, 200, 0),
            &ctx.state.status_message,
        );
    }

    if ctx.state.icons_hidden {
        ui.colored_label(egui::Color32::YELLOW, "⚠ 原生图标已隐藏");
    }

    ui.separator();
    ui.label("分类框风格设置：");
    ui.horizontal(|ui| {
        if ui.button("🎨 背景色").clicked() {
            ctx.set_status("背景色设置待实现");
        }
        if ui.button("⭕ 圆角").clicked() {
            ctx.set_status("圆角设置待实现");
        }
        if ui.button("🖼️ 图标大小").clicked() {
            ctx.set_status("图标大小设置待实现");
        }
        if ui.button("🔤 文字大小").clicked() {
            ctx.set_status("文字大小设置待实现");
        }
    });

    ui.separator();
    ui.label("AI 设置：");
    ui.horizontal(|ui| {
        ui.label("Base URL:");
        ui.text_edit_singleline(&mut ctx.config.ai.base_url);
    });
    ui.horizontal(|ui| {
        ui.label("API Key:");
        ui.text_edit_singleline(&mut ctx.config.ai.api_key);
    });

    ui.separator();
    if ui.button("💾 保存配置").clicked() {
        if let Err(e) = ctx.save_config() {
            ctx.set_status(format!("保存失败: {}", e));
        } else {
            ctx.set_status("配置已保存");
        }
    }
    if ui.button("📤 导出配置").clicked() {
        ctx.set_status("导出配置功能待实现");
    }
    if ui.button("📥 导入配置").clicked() {
        ctx.set_status("导入配置功能待实现");
    }
}
