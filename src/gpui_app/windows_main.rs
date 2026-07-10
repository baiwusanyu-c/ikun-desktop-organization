//! Windows GPUI 应用入口
//!
//! 完整替代 eframe 的 Windows 平台入口。
//! 使用 GPUI Application 事件循环驱动控制面板和创建分类框对话框。

use crate::gpui_app::{
    app_runtime::{AppRuntime, CONTROL_PANEL_TITLE},
    control_panel::ControlPanelView,
};
use crate::platform::hide_control_panel_window;
use gpui::{AppContext, Application, Timer};
use gpui_component::Root;
use std::time::Duration;

/// 运行 Windows GPUI 应用（完整入口）
pub fn run_gpui_app() {
    log::info!("启动 GPUI 应用 (Windows)");

    Application::new().run(|cx| {
        // 初始化 gpui-component 主题系统
        gpui_component::init(cx);
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

        // 延迟 1 秒后隐藏控制面板（避免启动时的借用冲突）
        cx.spawn(async move |cx| {
            Timer::after(Duration::from_millis(1000)).await;

            let result = cx.update(|_cx| {
                hide_control_panel_window(CONTROL_PANEL_TITLE);
                log::info!("延迟隐藏控制面板完成");
            });

            if result.is_err() {
                log::warn!("延迟隐藏控制面板失败");
            }
        })
        .detach();

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
