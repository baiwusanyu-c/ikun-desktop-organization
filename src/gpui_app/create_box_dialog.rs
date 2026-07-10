//! GPUI 创建分类框对话框视图
//!
//! 完整移植自原 eframe 版本的创建分类框对话框。
//! 输入验证、键盘快捷键（Enter/Escape）、按钮状态完全对齐原版。
//! 样式使用 gpui-component 主题系统，与项目设计风格保持一致。

use crate::gpui_app::app_runtime::{validate_box_name, AppRuntime};
use gpui::{
    div, px, AppContext, Context, Entity, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    ActiveTheme, Disableable, TitleBar,
};

/// 创建分类框对话框 GPUI 视图
pub struct CreateBoxDialogView {
    runtime: Entity<AppRuntime>,
    input_state: Entity<InputState>,
    _input_subscription: gpui::Subscription,
}

impl CreateBoxDialogView {
    pub fn new(runtime: Entity<AppRuntime>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_state = cx.new(|cx| InputState::new(window, cx).placeholder("必填，最多16字符"));
        let runtime_for_enter = runtime.clone();
        let _input_subscription = cx.subscribe_in(
            &input_state,
            window,
            move |_this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => {
                    let name = input.read(cx).value().to_string();
                    let validation = validate_box_name(&name);
                    if validation.is_valid {
                        runtime_for_enter.update(cx, |rt, cx| {
                            rt.dialog_name = validation.trimmed;
                            rt.confirm_create_box();
                            cx.notify();
                        });
                        window.remove_window();
                    }
                }
                _ => {}
            },
        );
        cx.observe(&runtime, |_, _, cx| cx.notify()).detach();
        Self {
            runtime,
            input_state,
            _input_subscription,
        }
    }
}

impl Render for CreateBoxDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let value = self.input_state.read(cx).value().to_string();
        let validation = validate_box_name(&value);
        let runtime = self.runtime.clone();
        let input_state = self.input_state.clone();

        // 主题色
        let bg = cx.theme().background;
        let fg = cx.theme().foreground;
        let border = cx.theme().border;
        let muted_fg = cx.theme().muted_foreground;
        let success = cx.theme().success;
        let danger = cx.theme().danger;

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .on_key_down({
                let runtime = runtime.clone();
                move |event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        runtime.update(cx, |rt, cx| {
                            rt.cancel_create_box_dialog();
                            cx.notify();
                        });
                        window.remove_window();
                    }
                }
            })
            // 自定义标题栏（紧贴窗口边缘）
            .child(
                TitleBar::new().bg(bg).border_color(bg).child(
                    div()
                        .flex()
                        .items_center()
                        .text_base()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(fg)
                        .child("创建分类框"),
                ),
            )
            // 内容区域
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .px(px(16.0))
                    .pb(px(16.0))
                    .pt(px(8.0))
                    // 输入行
                    .child(
                        h_flex()
                            .gap(px(8.0))
                            .items_center()
                            .child(div().text_sm().text_color(fg).child("名称："))
                            .child(div().flex_1().child(Input::new(&input_state.clone()))),
                    )
                    // 验证标签
                    .child(render_validation(&validation, muted_fg, danger, success))
                    // 分割线
                    .child(div().h(px(1.0)).bg(border))
                    // 按钮行（水平居中，自动推到底部）
                    .child(
                        div().flex_1().flex().items_center().justify_center().child(
                            h_flex()
                                .gap(px(12.0))
                                .child(
                                    Button::new("confirm")
                                        .primary()
                                        .label("确定")
                                        .disabled(!validation.is_valid)
                                        .on_click({
                                            let rt = runtime.clone();
                                            let inp = input_state.clone();
                                            move |_, win, cx| {
                                                let name = inp.read(cx).value().to_string();
                                                let validation = validate_box_name(&name);
                                                if validation.is_valid {
                                                    rt.update(cx, |rt, cx| {
                                                        rt.dialog_name = validation.trimmed;
                                                        rt.confirm_create_box();
                                                        cx.notify();
                                                    });
                                                    win.remove_window();
                                                }
                                            }
                                        }),
                                )
                                .child(Button::new("cancel").label("取消").on_click({
                                    let rt = runtime.clone();
                                    move |_, win, cx| {
                                        rt.update(cx, |rt, cx| {
                                            rt.cancel_create_box_dialog();
                                            cx.notify();
                                        });
                                        win.remove_window();
                                    }
                                })),
                        ),
                    ),
            )
    }
}

/// 渲染验证标签
///
/// 使用主题色：空输入显示 muted_foreground，超长显示 danger，合法显示 success。
fn render_validation(
    validation: &crate::gpui_app::app_runtime::BoxNameValidation,
    muted_fg: Hsla,
    danger: Hsla,
    success: Hsla,
) -> impl IntoElement {
    let color = if validation.is_empty {
        muted_fg
    } else if validation.is_too_long {
        danger
    } else {
        success
    };

    div()
        .text_sm()
        .text_color(color)
        .child(validation.message())
}
