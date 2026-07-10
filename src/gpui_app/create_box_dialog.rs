//! GPUI 创建分类框对话框视图
//!
//! 完整移植自原 eframe 版本的创建分类框对话框。
//! 输入验证、键盘快捷键（Enter/Escape）、按钮状态完全对齐原版。

use crate::gpui_app::app_runtime::{is_box_name_valid, AppRuntime, BOX_NAME_MAX_LEN};
use gpui::{
    div, px, rgb, AppContext, Context, Entity, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    Disableable,
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
                    let name = input.read(cx).value().trim().to_string();
                    if is_box_name_valid(&name) {
                        runtime_for_enter.update(cx, |rt, cx| {
                            rt.dialog_name = name;
                            rt.confirm_create_box();
                            cx.notify();
                        });
                        window.remove_window();
                    }
                }
                _ => {}
            },
        );
        // 观察 runtime 变化 → 刷新
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
        // 读取输入值
        let value = self.input_state.read(cx).value().to_string();
        let char_count = value.chars().count();
        let is_valid = is_box_name_valid(&value);
        let is_empty = value.trim().is_empty();

        let runtime = self.runtime.clone();
        let input_state = self.input_state.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(12.0))
            .bg(rgb(0xf5f5f5))
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
            // 标题
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("创建分类框"),
            )
            // 输入行
            .child(
                h_flex()
                    .gap(px(6.0))
                    .items_center()
                    .child(div().text_sm().child("名称："))
                    .child(div().flex_1().child(Input::new(&input_state.clone()))),
            )
            // 验证标签
            .child(render_validation(char_count, is_valid, is_empty))
            // 分割线
            .child(div().h(px(1.0)).bg(rgb(0xcccccc)))
            // 按钮行
            .child(
                h_flex()
                    .gap(px(8.0))
                    .child(
                        Button::new("confirm")
                            .primary()
                            .label("✓ 确定")
                            .disabled(!is_valid)
                            .on_click({
                                let rt = runtime.clone();
                                let inp = input_state.clone();
                                move |_, win, cx| {
                                    let name = inp.read(cx).value().to_string();
                                    let trimmed = name.trim().to_string();
                                    if is_box_name_valid(&trimmed) {
                                        rt.update(cx, |rt, cx| {
                                            rt.dialog_name = trimmed;
                                            rt.confirm_create_box();
                                            cx.notify();
                                        });
                                        win.remove_window();
                                    }
                                }
                            }),
                    )
                    .child(Button::new("cancel").label("✗ 取消").on_click({
                        let rt = runtime.clone();
                        move |_, win, cx| {
                            rt.update(cx, |rt, cx| {
                                rt.cancel_create_box_dialog();
                                cx.notify();
                            });
                            win.remove_window();
                        }
                    })),
            )
    }
}

/// 渲染验证标签
fn render_validation(char_count: usize, is_valid: bool, is_empty: bool) -> impl IntoElement {
    let (text, color) = if is_empty {
        ("名称不能为空".to_string(), rgb(0x888888))
    } else if !is_valid {
        if char_count > BOX_NAME_MAX_LEN {
            (
                format!("⚠ 最多 {} 字符（当前 {}）", BOX_NAME_MAX_LEN, char_count),
                rgb(0xff5050),
            )
        } else {
            ("名称无效".to_string(), rgb(0xff5050))
        }
    } else {
        (
            format!("✓ {}/{}", char_count, BOX_NAME_MAX_LEN),
            rgb(0x64c864),
        )
    };

    div().text_sm().text_color(color).child(text)
}
