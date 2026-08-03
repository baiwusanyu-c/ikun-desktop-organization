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
    #[cfg(windows)]
    ui.collapsing("💎 液态玻璃效果", |ui| {
        let glass = &mut ctx.config.box_style.liquid_glass;
        let mut changed = false;
        changed |= ui.checkbox(&mut glass.enabled, "启用液态玻璃").changed();
        ui.add_enabled_ui(glass.enabled, |ui| {
            changed |= ui
                .add(egui::Slider::new(&mut glass.displacement_scale, 0.0..=200.0).text("位移强度"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut glass.blur_amount, 0.0..=1.0).text("模糊程度"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut glass.saturation, 1.0..=3.0).text("饱和度"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut glass.chromatic_offset, 0.0..=20.0).text("色差偏移"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut glass.elasticity, 0.0..=1.0).text("弹性形变"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut glass.corner_radius, 0.0..=100.0).text("圆角半径"))
                .changed();
            ui.horizontal(|ui| {
                ui.label("折射贴图:");
                for (mode, label) in [(0, "标准"), (1, "极坐标"), (2, "凸起"), (3, "程序化")]
                {
                    if ui
                        .selectable_label(glass.refraction_mode == mode, label)
                        .clicked()
                    {
                        glass.refraction_mode = mode;
                        changed = true;
                    }
                }
            });
            changed |= ui.checkbox(&mut glass.over_light, "亮区减弱折射").changed();
        });
        if ui.button("🔄 重置默认").clicked() {
            *glass = crate::config::LiquidGlassConfig::default();
            changed = true;
        }

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
