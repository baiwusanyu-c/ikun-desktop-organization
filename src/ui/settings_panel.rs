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

    // ===== 液态玻璃效果 =====
    ui.collapsing("💎 液态玻璃效果", |ui| {
        let glass = &mut ctx.config.box_style.liquid_glass;
        let mut changed = false;

        ui.label("🔵 核心参数");
        changed |= ui.add(egui::Slider::new(&mut glass.blur_radius, 0.5..=8.0)
            .text("模糊半径")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.refraction_strength, 0.0..=20.0)
            .text("折射强度")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.glass_alpha, 0.0..=1.0)
            .text("玻璃透明度")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.edge_effect_width, 4.0..=60.0)
            .text("边缘过渡宽度")).changed();

        ui.separator();
        ui.label("🌈 光学效果");
        changed |= ui.add(egui::Slider::new(&mut glass.chromatic_offset, 0.0..=5.0)
            .text("色差偏移")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.contrast, 0.5..=2.0)
            .text("对比度")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.brightness, 0.0..=0.2)
            .text("亮度提升")).changed();

        ui.separator();
        ui.label("💡 光影效果");
        changed |= ui.add(egui::Slider::new(&mut glass.shadow_offset_y, 0.0..=20.0)
            .text("阴影偏移 Y")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.shadow_softness, 1.0..=30.0)
            .text("阴影柔和度")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.shadow_opacity, 0.0..=1.0)
            .text("阴影不透明度")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.top_light_position, 0.0..=1.0)
            .text("顶部高光位置")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.top_light_strength, 0.0..=0.5)
            .text("顶部高光强度")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.lower_shadow_position, 0.0..=1.0)
            .text("底部阴影位置")).changed();
        changed |= ui.add(egui::Slider::new(&mut glass.lower_shadow_strength, 0.0..=0.5)
            .text("底部阴影强度")).changed();

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("🔄 重置默认").clicked() {
                *glass = crate::config::LiquidGlassConfig::default();
                changed = true;
            }
        });

        // 实时更新：同步到全局配置、触发重绘并自动保存
        if changed {
            crate::platform::windows::liquid_glass::update_liquid_glass_config(glass);
            crate::platform::windows::grid_window::refresh_all_grid_windows();
            // 自动保存配置到文件
            if let Err(e) = ctx.save_config() {
                log::error!("自动保存液态玻璃配置失败: {}", e);
            }
        }
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
