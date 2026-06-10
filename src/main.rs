//! 桌面图标智能分类管理应用

mod app;
mod classification;
mod config;
mod desktop;
mod ui;
mod utils;

use eframe::egui;

fn main() -> eframe::Result<()> {
    utils::init_logger();

    log::info!("========================================");
    log::info!("启动 Desktop Organizer...");
    log::info!("========================================");

    // 控制面板窗口（正常窗口，非全屏）
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Desktop Organizer - 控制面板")
            .with_inner_size([320.0, 500.0])
            .with_decorations(true)
            .with_always_on_top()
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "Desktop Organizer",
        options,
        Box::new(|cc| {
            // 加载中文字体
            setup_chinese_fonts(&cc.egui_ctx);
            Box::new(app::MyApp::new(cc))
        }),
    )
}

/// 加载系统中文字体（微软雅黑）
fn setup_chinese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // 尝试加载微软雅黑字体（Windows 系统自带中文字体）
    let font_paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyhbd.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];

    for path in &font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            log::info!("加载中文字体: {}", path);
            fonts.font_data.insert(
                "chinese_font".to_owned(),
                egui::FontData::from_owned(font_data),
            );

            // 将中文字体插入到比例字体族的最前面
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "chinese_font".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.insert(0, "chinese_font".to_owned());
            }

            ctx.set_fonts(fonts);
            return;
        }
    }

    log::warn!("未找到中文字体，界面中文可能显示为方块");
}
