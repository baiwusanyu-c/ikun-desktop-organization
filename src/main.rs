//! 桌面图标智能分类管理应用
//!
//! Desktop Organizer - 桌面图标智能分类管理工具
//! 支持平台: Windows 10/11, macOS

mod app;
mod desktop;
mod classification;
mod ui;
mod config;
mod utils;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // 初始化日志
    env_logger::init();

    log::info!("启动 Desktop Organizer...");

    // 窗口配置
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Desktop Organizer")
            .with_inner_size([800.0, 600.0])
            .with_decorations(false)     // 无边框
            .with_transparent(true),     // 透明背景
        ..Default::default()
    };

    // 启动应用
    eframe::run_native(
        "Desktop Organizer",
        options,
        Box::new(|cc| {
            app::MyApp::new(cc)
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}