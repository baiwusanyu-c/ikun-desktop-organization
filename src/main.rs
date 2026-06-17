//! Desktop Organizer - 入口

// 发布构建使用 windows 子系统（无控制台窗口），调试构建保留控制台用于日志
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod box_core;
mod classification;
mod config;
mod context;
#[allow(dead_code)]
mod ipc;
mod platform;
mod plugin;
mod ui;
mod utils;

use eframe::egui;

fn main() -> eframe::Result<()> {
    utils::init_logger();
    log::info!("========================================");
    log::info!("启动 Desktop Organizer (重写版)");
    log::info!("========================================");

    // 设置 Per-Monitor DPI V2 感知（运行时设置，作为 manifest 的补充保障）
    // 防止 Windows 对窗口做 bitmap-stretch 导致文字和图标模糊
    #[cfg(target_os = "windows")]
    {
        set_per_monitor_dpi_v2();
    }

    // --create-box: 桌面右键菜单注册表项触发
    // 立即释放控制台窗口（避免黑框闪烁），写入信号文件后退出
    if std::env::args().any(|a| a == "--create-box") {
        #[cfg(target_os = "windows")]
        {
            extern "system" {
                fn FreeConsole() -> i32;
            }
            unsafe { FreeConsole(); }
        }
        let signal = std::env::temp_dir().join("ikun_create_box.signal");
        let _ = std::fs::write(&signal, "1");
        return Ok(());
    }

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

/// 设置 Per-Monitor DPI V2 感知（运行时调用，作为 manifest 的补充保障）
///
/// 确保 Windows 不对格子窗口做 bitmap-stretch，保持文字和图标清晰。
/// DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
#[cfg(target_os = "windows")]
fn set_per_monitor_dpi_v2() {
    use std::ffi::c_void;

    #[allow(non_camel_case_types)]
    type DPI_AWARENESS_CONTEXT = *mut c_void;
    const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: DPI_AWARENESS_CONTEXT =
        -4isize as *mut c_void;

    extern "system" {
        fn SetProcessDpiAwarenessContext(value: DPI_AWARENESS_CONTEXT) -> i32;
    }

    let result =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if result != 0 {
        log::info!("已设置 Per-Monitor DPI V2 感知（运行时）");
    } else {
        log::warn!("设置 Per-Monitor DPI V2 失败（可能系统版本不支持），依赖 manifest 配置");
    }
}
