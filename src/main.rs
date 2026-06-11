//! Desktop Organizer - 入口

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
    log::info!("启动 Desktop Organizer (重写版)");
    log::info!("========================================");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Desktop Organizer - 控制面板")
            .with_inner_size([320.0, 380.0])
            .with_decorations(true)
            .with_always_on_top()
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "Desktop Organizer",
        options,
        Box::new(|cc| {
            setup_chinese_fonts(&cc.egui_ctx);
            Box::new(app::MyApp::new(cc))
        }),
    )
}

fn setup_chinese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let font_paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyhbd.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];

    for path in &font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            log::info!("加载中文字体: {}", path);
            fonts
                .font_data
                .insert("chinese".to_owned(), egui::FontData::from_owned(font_data));
            if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                f.insert(0, "chinese".to_owned());
            }
            if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                f.insert(0, "chinese".to_owned());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
    log::warn!("未找到中文字体");
}
