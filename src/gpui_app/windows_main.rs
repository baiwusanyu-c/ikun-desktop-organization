//! Windows GPUI 应用入口
//!
//! 完整替代 eframe 的 Windows 平台入口。
//! 使用 GPUI Application 事件循环驱动控制面板和创建分类框对话框。

use crate::gpui_app::{app_runtime::AppRuntime, control_panel::ControlPanelView};
use gpui::{AppContext, Application, Timer};
use gpui_component::Root;
use std::time::Duration;

/// 运行 Windows GPUI 应用（完整入口）
pub fn run_gpui_app() {
    log::info!("启动 GPUI 应用 (Windows)");

    Application::new()
        .with_assets(crate::gpui_app::assets::EmbeddedAssets)
        .run(|cx| {
            // 初始化 gpui-component 主题系统
            gpui_component::init(cx);

            // 设置标题栏颜色与背景色一致（让原生标题栏融入窗口背景）
            {
                let theme = gpui_component::Theme::global_mut(cx);
                theme.title_bar = theme.background;
                theme.title_bar_border = theme.border;
            }

            // 加载配置并应用自定义主题
            if let Ok(config) = crate::config::app_config::AppConfig::load() {
                apply_ui_theme(&config.ui_theme, cx);
                log::info!("已应用 UI 主题配置");
            }

            log::info!("gpui-component 已初始化");

            // 在 App 级别创建 AppRuntime 实体
            let runtime = cx.new(|_cx| AppRuntime::new());

            // 执行首帧初始化
            runtime.update(cx, |rt, _cx| {
                rt.initialize();
            });

            // 打开控制面板窗口
            if let Err(e) = ControlPanelView::open_window(runtime.clone(), cx) {
                log::error!("打开控制面板窗口失败: {}", e);
                cx.quit();
                return;
            }

            // 激活应用
            cx.activate(true);

            // 启动轮询定时器（每 50ms 检查信号文件和托盘动作）
            let runtime_for_timer = runtime.clone();
            cx.spawn(async move |cx| {
                let mut dialog_handle: Option<gpui::WindowHandle<Root>> = None;
                loop {
                    Timer::after(Duration::from_millis(50)).await;

                    // 第一步：检查状态并执行 tick
                    let (should_quit, changed, needs_open_dialog) = match cx.update(|cx| {
                        runtime_for_timer.update(cx, |rt, _cx| {
                            let changed = rt.tick();
                            (rt.is_quit_requested(), changed, rt.dialog_needs_open)
                        })
                    }) {
                        Ok(result) => result,
                        Err(_) => {
                            log::warn!("轮询定时器更新失败");
                            break;
                        }
                    };

                    // 第二步：处理退出
                    if should_quit {
                        cx.update(|cx| cx.quit()).ok();
                        break;
                    }

                    // 第三步：刷新窗口
                    if changed {
                        cx.update(|cx| cx.refresh_windows()).ok();
                    }

                    // 第四步：如果需要打开对话框，在独立的 update 中执行
                    if needs_open_dialog {
                        // 先清除标志
                        cx.update(|cx| {
                            runtime_for_timer.update(cx, |rt, _cx| {
                                rt.dialog_needs_open = false;
                            });
                        })
                        .ok();

                        // 然后在新的 update 中打开或复用窗口（避免嵌套借用）
                        let open_result = cx.update(|cx| {
                            if let Some(handle) = dialog_handle {
                                if handle
                                    .update(cx, |_root, window, _cx| {
                                        window.remove_window();
                                    })
                                    .is_err()
                                {
                                    log::debug!("创建分类框窗口句柄已失效，将重新创建");
                                }
                                dialog_handle = None;
                            }

                            ControlPanelView::open_create_box_dialog_window(
                                runtime_for_timer.clone(),
                                cx,
                            )
                        });

                        match open_result {
                            Ok(Ok(handle)) => {
                                dialog_handle = Some(handle);
                            }
                            Ok(Err(e)) => {
                                log::warn!("打开创建分类框窗口失败: {}", e);
                            }
                            Err(_) => {
                                log::warn!("打开创建分类框窗口时更新 GPUI 失败");
                                break;
                            }
                        }
                    }
                }
            })
            .detach();

            log::info!("GPUI 应用初始化完成，进入事件循环");
        });

    log::info!("GPUI 事件循环已退出");
}

/// 从配置文件应用 UI 主题颜色
///
/// 解析十六进制颜色字符串并应用到 gpui-component 主题。
/// 支持 #RRGGBB 和 #RRGGBBAA 格式。
fn apply_ui_theme(ui_theme: &crate::config::app_config::UiThemeConfig, cx: &mut gpui::App) {
    use gpui::hsla;

    fn parse_hex(hex: &str) -> Option<gpui::Hsla> {
        let hex = hex.trim_start_matches('#');
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(hsla(
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    1.0,
                ))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(hsla(
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                ))
            }
            _ => None,
        }
    }

    let theme = gpui_component::Theme::global_mut(cx);

    // 应用亮色主题（当前只应用亮色，暗色留待系统切换支持）
    let colors = &ui_theme.light;
    if let Some(ref bg) = colors.background {
        if let Some(c) = parse_hex(bg) {
            theme.background = c;
            theme.title_bar = c;
        }
    }
    if let Some(ref fg) = colors.foreground {
        if let Some(c) = parse_hex(fg) {
            theme.foreground = c;
        }
    }
    if let Some(ref border) = colors.border {
        if let Some(c) = parse_hex(border) {
            theme.border = c;
            theme.title_bar_border = c;
        }
    }
    if let Some(ref primary) = colors.primary {
        if let Some(c) = parse_hex(primary) {
            theme.primary = c;
        }
    }
    if let Some(ref primary_fg) = colors.primary_foreground {
        if let Some(c) = parse_hex(primary_fg) {
            theme.primary_foreground = c;
        }
    }
    if let Some(ref success) = colors.success {
        if let Some(c) = parse_hex(success) {
            theme.success = c;
        }
    }
    if let Some(ref danger) = colors.danger {
        if let Some(c) = parse_hex(danger) {
            theme.danger = c;
        }
    }
    if let Some(ref muted_fg) = colors.muted_foreground {
        if let Some(c) = parse_hex(muted_fg) {
            theme.muted_foreground = c;
        }
    }

    log::debug!("已应用亮色主题配置");
}
