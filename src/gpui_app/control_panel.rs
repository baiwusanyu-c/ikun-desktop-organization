//! GPUI 控制面板视图
//!
//! 完整移植自原 eframe 版本的控制面板 UI。
//! 按钮布局、状态消息、图标隐藏提示完全对齐原版。

use crate::gpui_app::app_runtime::{AppRuntime, CONTROL_PANEL_TITLE};
use crate::gpui_app::create_box_dialog::CreateBoxDialogView;
use gpui::{
    div, px, rgb, size, App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, WindowBounds, WindowOptions,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, Root,
};

/// 控制面板快照数据（避免 render 期间持有借用）
struct PanelSnapshot {
    status_msg: String,
    icons_hidden: bool,
    box_count: usize,
}

/// 控制面板 GPUI 视图
pub struct ControlPanelView {
    runtime: Entity<AppRuntime>,
}

impl ControlPanelView {
    pub fn new(runtime: Entity<AppRuntime>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        log::debug!("[ControlPanelView] new() 创建视图");
        Self { runtime }
    }

    /// 一次性读取所有快照数据（立即释放借用）
    fn read_snapshot(&self, cx: &Context<Self>) -> PanelSnapshot {
        let rt = self.runtime.read(cx);
        PanelSnapshot {
            status_msg: rt.status_message().to_string(),
            icons_hidden: rt.icons_hidden(),
            box_count: rt.box_count(),
        }
    }

    /// 打开控制面板 GPUI 窗口（不使用 Root 包装）
    pub fn open_window(
        runtime: Entity<AppRuntime>,
        cx: &mut App,
    ) -> anyhow::Result<gpui::WindowHandle<Root>> {
        log::debug!("[ControlPanelView] open_window() 开始");
        let bounds = Bounds::centered(None, size(px(320.0), px(380.0)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(SharedString::from(CONTROL_PANEL_TITLE)),
                    ..Default::default()
                }),
                show: false,
                ..Default::default()
            },
            |window, cx| {
                log::debug!("[ControlPanelView] 创建视图");
                let view = cx.new(|cx| Self::new(runtime.clone(), window, cx));
                let rt_for_close = runtime.clone();
                window.on_window_should_close(cx, move |_window, cx| {
                    rt_for_close.update(cx, |rt, cx| {
                        if rt.is_quit_requested() {
                            true
                        } else {
                            rt.hide_control_panel();
                            cx.notify();
                            false
                        }
                    })
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        );
        log::debug!("[ControlPanelView] open_window() 完成");
        result
    }

    /// 打开创建分类框对话框窗口
    pub fn open_create_box_dialog_window(
        runtime: Entity<AppRuntime>,
        cx: &mut App,
    ) -> anyhow::Result<gpui::WindowHandle<Root>> {
        log::debug!("[ControlPanelView] open_create_box_dialog_window() 开始");
        let bounds = Bounds::centered(None, size(px(500.0), px(200.0)), cx);

        let handle = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                is_resizable: true,
                window_min_size: Some(size(px(400.0), px(160.0))),
                ..Default::default()
            },
            |window, cx| {
                log::debug!("[ControlPanelView] 创建对话框视图");
                let view = cx.new(|cx| CreateBoxDialogView::new(runtime.clone(), window, cx));
                let rt_for_close = runtime.clone();
                window.on_window_should_close(cx, move |_window, cx| {
                    rt_for_close.update(cx, |rt, cx| {
                        rt.cancel_create_box_dialog();
                        cx.notify();
                    });
                    true
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        );
        log::debug!("[ControlPanelView] open_create_box_dialog_window() 完成");
        handle
    }
}

impl Render for ControlPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        log::debug!("[ControlPanelView] render() 开始");

        // 一次性读取所有数据，立即释放借用
        let snapshot = self.read_snapshot(cx);

        // 克隆 Entity 用于按钮回调
        let runtime = self.runtime.clone();

        log::debug!("[ControlPanelView] render() 构建 UI");

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(12.0))
            .bg(rgb(0xf5f5f5))
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("📦 Desktop Organizer"),
            )
            .child(div().h(px(1.0)).bg(rgb(0xcccccc)))
            .child(
                h_flex()
                    .gap(px(6.0))
                    .child(
                        Button::new("auto_classify")
                            .primary()
                            .label("🔄 一键分类")
                            .on_click({
                                let rt = runtime.clone();
                                move |_, _window, cx| {
                                    rt.update(cx, |rt, cx| {
                                        rt.perform_auto_classify();
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("clear_restore")
                            .danger()
                            .label("🗑 清除 & 恢复")
                            .on_click({
                                let rt = runtime.clone();
                                move |_, _window, cx| {
                                    rt.update(cx, |rt, cx| {
                                        rt.clear_and_restore();
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap(px(6.0))
                    .child(Button::new("save_config").label("💾 保存配置").on_click({
                        let rt = runtime.clone();
                        move |_, _window, cx| {
                            rt.update(cx, |rt, cx| {
                                rt.save_layout_and_config();
                                cx.notify();
                            });
                        }
                    }))
                    .child(Button::new("create_box").label("➕ 创建分类框").on_click({
                        let rt = runtime.clone();
                        move |_, _window, cx| {
                            rt.update(cx, |rt, cx| {
                                rt.open_create_box_dialog();
                                cx.notify();
                            });
                        }
                    })),
            )
            .child({
                if !snapshot.status_msg.is_empty() {
                    div()
                        .p(px(6.0))
                        .rounded(px(4.0))
                        .bg(rgb(0xd4edda))
                        .text_color(rgb(0x155724))
                        .text_sm()
                        .child(snapshot.status_msg)
                        .into_any_element()
                } else {
                    div().into_any_element()
                }
            })
            .child({
                if snapshot.icons_hidden {
                    div()
                        .p(px(6.0))
                        .rounded(px(4.0))
                        .bg(rgb(0xfff3cd))
                        .text_color(rgb(0x856404))
                        .text_sm()
                        .child("⚠ 原生桌面图标已隐藏，由覆盖窗口接管")
                        .into_any_element()
                } else {
                    div().into_any_element()
                }
            })
            .child(
                div()
                    .mt(px(16.0))
                    .text_xs()
                    .text_color(rgb(0x999999))
                    .child(format!("分类框: {} 个", snapshot.box_count)),
            )
    }
}
