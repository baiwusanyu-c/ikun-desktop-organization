//! Windows GPUI 应用入口
//!
//! 完整替代 eframe 的 Windows 平台入口。
//! 使用 GPUI Application 事件循环驱动控制面板和创建分类框对话框。

use crate::gpui_app::{app_runtime::AppRuntime, control_panel::ControlPanelView};
use gpui::{AppContext, Application, Timer};
use gpui_component::Root;
use std::panic::{catch_unwind, AssertUnwindSafe};
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

            // 启动轮询定时器（每 100ms 检查信号文件和托盘动作，降低与渲染帧的 RefCell 冲突频率）
            let runtime_for_timer = runtime.clone();
            cx.spawn(async move |cx| {
                let mut dialog_handle: Option<gpui::WindowHandle<Root>> = None;
                loop {
                    Timer::after(Duration::from_millis(100)).await;

                    // 第一步：检查状态并执行 tick
                    let (should_quit, changed, needs_open_dialog) = match safe_gpui_update(
                        &cx,
                        "tick",
                        |cx| {
                            runtime_for_timer.update(cx, |rt, _cx| {
                                let changed = rt.tick();
                                (rt.is_quit_requested(), changed, rt.dialog_needs_open)
                            })
                        },
                    ) {
                        Ok(Ok(result)) => result,
                        Ok(Err(_)) => {
                            log::warn!("轮询定时器更新失败");
                            break;
                        }
                        Err(_) => continue, // RefCell 借用冲突，跳过本次
                    };

                    // 第二步：处理退出
                    if should_quit {
                        let _ = safe_gpui_update(&cx, "quit", |cx| cx.quit());
                        break;
                    }

                    // 第三步：刷新窗口
                    if changed {
                        let _ = safe_gpui_update(&cx, "refresh", |cx| cx.refresh_windows());
                    }

                    // 第四步：如果需要打开对话框，在独立的 update 中执行
                    if needs_open_dialog {
                        // 先清除标志
                        let _ = safe_gpui_update(&cx, "clear_dialog_flag", |cx| {
                            runtime_for_timer.update(cx, |rt, _cx| {
                                rt.dialog_needs_open = false;
                            });
                        });

                        // 然后在新的 update 中打开或复用窗口（避免嵌套借用）
                        let open_result = match safe_gpui_update(&cx, "open_create_dialog", |cx| {
                            if let Some(handle) = dialog_handle {
                                if handle.update(cx, |_root, window, _cx| {
                                    window.remove_window();
                                }).is_ok() {
                                    log::debug!("已关闭旧的创建分类框窗口");
                                }
                                dialog_handle = None;
                            }

                            ControlPanelView::open_create_box_dialog_window(
                                runtime_for_timer.clone(),
                                cx,
                            )
                        }) {
                            Ok(result) => result,
                            Err(_) => Err(anyhow::anyhow!("RefCell 借用冲突，跳过本帧")),
                        };

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

                    // 第五步：检查是否需要打开编辑分类框对话框
                    // 合并所有编辑对话框相关操作到一个 cx.update() 调用中，避免多次借用
                    let pending_edit = crate::gpui_app::app_runtime::take_pending_edit_request();
                    let edit_dialog_info = match safe_gpui_update(&cx, "edit_dialog_info", |cx| {
                        runtime_for_timer.update(cx, |rt, _cx| {
                            // 如果有待处理的编辑请求，设置标志
                            if let Some((config_id, current_name)) = pending_edit {
                                log::debug!("[Timer] 设置编辑对话框标志: config_id={}, name={}", config_id, current_name);
                                rt.edit_dialog_config_id = Some(config_id);
                                rt.edit_dialog_initial_name = current_name;
                                rt.edit_dialog_needs_open = true;
                            }

                            // 读取并清除标志（在同一借用中完成）
                            let needs_open = rt.edit_dialog_needs_open;
                            let config_id = rt.edit_dialog_config_id;
                            let initial_name = rt.edit_dialog_initial_name.clone();

                            if needs_open {
                                rt.edit_dialog_needs_open = false;
                                log::debug!("[Timer] 读取编辑对话框标志并清除: needs_open={}, config_id={:?}", needs_open, config_id);
                            }

                            (needs_open, config_id, initial_name)
                        })
                    }) {
                        Ok(Ok(result)) => result,
                        Ok(Err(_)) => {
                            log::warn!("[Timer] 更新失败（编辑对话框）");
                            break;
                        }
                        Err(_) => continue, // RefCell 借用冲突，跳过本次
                    };

                    if edit_dialog_info.0 {
                        if let (Some(config_id), initial_name) = (edit_dialog_info.1, edit_dialog_info.2) {
                            log::info!("[Timer] 准备打开编辑对话框: config_id={}, initial_name={}", config_id, initial_name);

                            let open_result = match safe_gpui_update(&cx, "open_edit_dialog", |cx| {
                                // 关闭已存在的对话框窗口
                                if let Some(handle) = dialog_handle {
                                    if handle.update(cx, |_root, window, _cx| {
                                            window.remove_window();
                                        }).is_ok() {
                                            log::debug!("已关闭旧的编辑分类框窗口");
                                        }
                                    dialog_handle = None;
                                }

                                ControlPanelView::open_edit_box_dialog_window(
                                    runtime_for_timer.clone(),
                                    cx,
                                    config_id,
                                    &initial_name,
                                )
                            }) {
                                Ok(result) => result,
                                Err(_) => Err(anyhow::anyhow!("RefCell 借用冲突，跳过本帧")),
                            };

                            match open_result {
                                Ok(Ok(handle)) => {
                                    log::info!("[Timer] 编辑对话框窗口已打开");
                                    dialog_handle = Some(handle);
                                }
                                Ok(Err(e)) => {
                                    log::warn!("打开编辑分类框窗口失败: {}", e);
                                }
                                Err(_) => {
                                    log::warn!("打开编辑分类框窗口时更新 GPUI 失败");
                                    break;
                                }
                            }
                        }
                    }
                }
            })
            .detach();

            // ?????????????????????
            crate::platform::windows::alignment_guide::guide_overlay::init_guide_color();

            log::info!("GPUI 应用初始化完成，进入事件循环");
        });

    log::info!("GPUI 事件循环已退出");
}

/// 从配置文件应用 UI 主题颜色
///
/// 解析十六进制颜色字符串并应用到 gpui-component 主题。

/// 安全执行 GPUI update 操作，捕获 RefCell 借用冲突 panic
///
/// GPUI 的 Entity::update() 和 App::update() 内部使用 RefCell 借用检查，
/// 当定时器的 orrow_mut 与渲染循环的 orrow 同时发生时，会触发 double_lease_panic!
/// 导致进程崩溃。此辅助函数通过 catch_unwind 将 panic 降级为可恢复的跳过操作。
///
/// # 返回值
/// - Ok(Ok(T)) — update 成功执行
/// - Ok(Err(anyhow::Error)) — GPUI AsyncApp 已释放（应用正在退出）
/// - Err(RefCellBorrowConflict) — RefCell 借用冲突，应跳过本次操作
fn safe_gpui_update<T>(
    cx: &gpui::AsyncApp,
    operation_name: &str,
    f: impl FnOnce(&mut gpui::App) -> T,
) -> Result<anyhow::Result<T>, RefCellBorrowConflict> {
    match catch_unwind(AssertUnwindSafe(|| cx.update(f))) {
        Ok(result) => Ok(result),
        Err(_) => {
            log::debug!("定时器跳过 {}: RefCell 借用冲突", operation_name);
            Err(RefCellBorrowConflict)
        }
    }
}

/// RefCell 借用冲突标记类型
///
/// 表示 GPUI 的 RefCell 内部借用检查检测到冲突，
/// 通常在定时器回调与渲染回调同时访问同一 Entity 时发生。
struct RefCellBorrowConflict;


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

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// 测试 RefCellBorrowConflict 类型可以正常构造
    #[test]
    fn test_refcell_borrow_conflict_is_constructible() {
        let _conflict = RefCellBorrowConflict;
    }

    /// 测试 safe_gpui_update 的核心逻辑：正常情况返回 Ok(Ok(value))
    ///
    /// 由于 GPUI 的 AsyncApp 无法在测试环境中构造，
    /// 这里通过模拟 catch_unwind 的行为来验证模式正确性。
    #[test]
    fn test_catch_unwind_no_panic_returns_ok() {
        let result: Result<i32, _> = catch_unwind(AssertUnwindSafe(|| 42));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    /// 测试 catch_unwind 在 panic 时返回 Err
    ///
    /// 模拟 GPUI double_lease_panic 场景：
    /// 当 RefCell 借用冲突发生时，catch_unwind 应捕获 panic 并返回 Err。
    #[test]
    fn test_catch_unwind_panic_returns_err() {
        let result: Result<i32, _> = catch_unwind(AssertUnwindSafe(|| {
            panic!("simulated RefCell borrow conflict");
        }));
        assert!(result.is_err(), "catch_unwind 应捕获 panic 并返回 Err");
    }

    /// 测试 catch_unwind 嵌套 Result 模式：
    /// 模拟 safe_gpui_update 的三层 match 逻辑
    #[test]
    fn test_safe_update_pattern_success() {
        // 模拟 cx.update() 返回 Ok(42)
        let inner = || -> Result<i32, anyhow::Error> { Ok(42) };
        let result = catch_unwind(AssertUnwindSafe(|| inner()));

        match result {
            Ok(Ok(value)) => assert_eq!(value, 42),
            Ok(Err(_)) => panic!("不应到达: update 返回 Ok"),
            Err(_) => panic!("不应到达: 无 panic"),
        }
    }

    /// 测试 safe_gpui_update 模式：GPUI 已释放（App 退出）
    #[test]
    fn test_safe_update_pattern_app_released() {
        // 模拟 cx.update() 返回 Err（AsyncApp 已释放）
        let inner = || -> Result<i32, anyhow::Error> {
            Err(anyhow::anyhow!("app was released"))
        };
        let result = catch_unwind(AssertUnwindSafe(|| inner()));

        match result {
            Ok(Ok(_)) => panic!("不应到达: update 返回 Err"),
            Ok(Err(e)) => assert!(e.to_string().contains("app was released")),
            Err(_) => panic!("不应到达: 无 panic"),
        }
    }

    /// 测试 safe_gpui_update 模式：RefCell 借用冲突
    #[test]
    fn test_safe_update_pattern_refcell_conflict() {
        // 模拟 cx.update() 内部 panic（double_lease_panic）
        let inner = || -> Result<i32, anyhow::Error> {
            panic!("cannot update AppRuntime while it is already being updated");
        };
        let result = catch_unwind(AssertUnwindSafe(|| inner()));

        match result {
            Ok(Ok(_)) => panic!("不应到达: panic 发生"),
            Ok(Err(_)) => panic!("不应到达: panic 发生"),
            Err(_) => {
                // 正确路径：RefCell 借用冲突被捕获，应跳过本次 tick
            }
        }
    }

    /// 验证 parse_hex 函数（从 apply_ui_theme 中提取的纯逻辑）
    #[test]
    fn test_parse_hex_6_digit() {
        fn parse_hex(hex: &str) -> Option<(f32, f32, f32, f32)> {
            let hex = hex.trim_start_matches('#');
            match hex.len() {
                6 => {
                    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                    Some((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0))
                }
                8 => {
                    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                    let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                    Some((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0))
                }
                _ => None,
            }
        }

        // 6 位 RGB
        let (r, g, b, a) = parse_hex("#FF0080").unwrap();
        assert!((r - 1.0).abs() < 0.01);
        assert!((g - 0.0).abs() < 0.01);
        assert!((b - 0.502).abs() < 0.01);
        assert!((a - 1.0).abs() < 0.01);

        // 8 位 RGBA
        let (r, _g, _b, a) = parse_hex("#FF008080").unwrap();
        assert!((r - 1.0).abs() < 0.01);
        assert!((a - 0.502).abs() < 0.01);

        // 无效输入
        assert!(parse_hex("#FFF").is_none());
        assert!(parse_hex("invalid").is_none());
        assert!(parse_hex("#GGGGGG").is_none());
    }
}
