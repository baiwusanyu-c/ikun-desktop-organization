//! 设置面板
//!
//! 分类框样式、AI 配置、热键配置等详细设置界面。
//! TODO: 后续实现完整的设置面板 UI。

use crate::context::AppContext;

/// 渲染设置面板
pub fn render_settings_panel(ctx: &mut AppContext, ui: &mut egui::Ui) {
    ui.heading("⚙️ 设置");
    ui.separator();

    // ===== 分类框样式 =====
    ui.collapsing("🎨 分类框样式", |ui| {
        let style = &mut ctx.config.box_style;

        ui.horizontal(|ui| {
            ui.label("背景透明度:");
            ui.add(egui::Slider::new(&mut style.background_alpha, 0..=255));
        });

        ui.horizontal(|ui| {
            ui.label("圆角半径:");
            ui.add(egui::Slider::new(&mut style.border_radius, 0.0..=50.0));
        });

        ui.horizontal(|ui| {
            ui.label("图标大小:");
            ui.add(egui::Slider::new(&mut style.icon_size, 16..=128));
        });

        ui.horizontal(|ui| {
            ui.label("标题字号:");
            ui.add(egui::Slider::new(&mut style.title_font_size, 8.0..=32.0));
        });

        ui.horizontal(|ui| {
            ui.label("标签字号:");
            ui.add(egui::Slider::new(&mut style.label_font_size, 8.0..=24.0));
        });
    });

    // ===== AI 配置 =====
    ui.collapsing("🤖 AI 分类", |ui| {
        let ai = &mut ctx.config.ai;

        ui.checkbox(&mut ai.enabled, "启用 AI 分类");

        ui.horizontal(|ui| {
            ui.label("Base URL:");
            ui.text_edit_singleline(&mut ai.base_url);
        });

        ui.horizontal(|ui| {
            ui.label("API Key:");
            ui.add(egui::TextEdit::singleline(&mut ai.api_key).password(true));
        });

        ui.horizontal(|ui| {
            ui.label("模型:");
            ui.text_edit_singleline(&mut ai.model);
        });

        ui.horizontal(|ui| {
            ui.label("Temperature:");
            ui.add(egui::Slider::new(&mut ai.temperature, 0.0..=2.0));
        });
    });

    // ===== 热键配置 =====
    ui.collapsing("⌨️ 热键", |ui| {
        let hotkeys = &mut ctx.config.hotkeys;

        ui.horizontal(|ui| {
            ui.label("显示/隐藏面板:");
            ui.text_edit_singleline(&mut hotkeys.toggle_panel);
        });

        ui.horizontal(|ui| {
            ui.label("一键分类:");
            ui.text_edit_singleline(&mut hotkeys.auto_classify);
        });
    });

    // ===== 调试配置 =====
    ui.collapsing("🔧 调试", |ui| {
        let debug = &mut ctx.config.debug;

        ui.horizontal(|ui| {
            ui.label("日志级别:");
            egui::ComboBox::from_id_source("log_level")
                .selected_text(&debug.log_level)
                .show_ui(ui, |ui| {
                    for level in ["error", "warn", "info", "debug", "trace"] {
                        ui.selectable_value(&mut debug.log_level, level.to_string(), level);
                    }
                });
        });

        ui.checkbox(&mut debug.log_to_file, "输出日志到文件");
        ui.checkbox(&mut debug.show_fps, "显示 FPS");
    });

    ui.separator();
    if ui.button("💾 保存配置").clicked() {
        if let Err(e) = ctx.save_config() {
            log::error!("保存配置失败: {}", e);
        }
    }
}
