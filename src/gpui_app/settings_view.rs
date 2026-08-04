//! 液态玻璃效果设置面板（对齐 liquid-glass-rs FrostedLiquid EffectStyle）

use crate::gpui_app::app_runtime::AppRuntime;
use gpui::{
    div, px, rgb, size, App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, WindowBounds, WindowOptions,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    slider::{Slider, SliderEvent, SliderState},
    Root,
};

struct ParamDef {
    param: crate::config::LiquidGlassParam,
    label: &'static str,
    min: f32,
    max: f32,
    step: f32,
    format: ParamFormat,
}

enum ParamFormat {
    Number,
    Percent,
}

const PARAMS: &[ParamDef] = &[
    ParamDef {
        param: crate::config::LiquidGlassParam::DisplacementScale,
        label: "位移强度",
        min: 0.0,
        max: 200.0,
        step: 1.0,
        format: ParamFormat::Number,
    },
    ParamDef {
        param: crate::config::LiquidGlassParam::BlurAmount,
        label: "模糊程度",
        min: 0.0,
        max: 1.0,
        step: 0.01,
        format: ParamFormat::Number,
    },
    ParamDef {
        param: crate::config::LiquidGlassParam::Saturation,
        label: "饱和度",
        min: 1.0,
        max: 3.0,
        step: 0.01,
        format: ParamFormat::Percent,
    },
    ParamDef {
        param: crate::config::LiquidGlassParam::ChromaticOffset,
        label: "色差偏移",
        min: 0.0,
        max: 20.0,
        step: 1.0,
        format: ParamFormat::Number,
    },
    ParamDef {
        param: crate::config::LiquidGlassParam::CornerRadius,
        label: "圆角半径",
        min: 0.0,
        max: crate::config::LIQUID_GLASS_CORNER_RADIUS_MAX,
        step: 1.0,
        format: ParamFormat::Number,
    },
    ParamDef {
        param: crate::config::LiquidGlassParam::Elasticity,
        label: "弹性形变",
        min: 0.0,
        max: crate::config::LIQUID_GLASS_ELASTICITY_MAX,
        step: 0.01,
        format: ParamFormat::Number,
    },
];

pub struct SettingsView {
    runtime: Entity<AppRuntime>,
    sliders: Vec<Entity<SliderState>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl SettingsView {
    pub fn new(runtime: Entity<AppRuntime>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut sliders = Vec::with_capacity(PARAMS.len());
        let mut subscriptions = Vec::with_capacity(PARAMS.len());

        let glass = runtime.read(cx).liquid_glass().clone();
        let values: Vec<f32> = PARAMS.iter().map(|p| glass.get_param(p.param)).collect();

        for (i, param) in PARAMS.iter().enumerate() {
            let slider = cx.new(|_cx| {
                SliderState::new()
                    .min(param.min)
                    .max(param.max)
                    .step(param.step)
                    .default_value(values[i])
            });

            let rt = runtime.clone();
            let param_id = param.param;
            let sub = cx.subscribe_in(
                &slider,
                window,
                move |_this, _state, event: &SliderEvent, _w, cx| {
                    let SliderEvent::Change(val) = event;
                    let v = val.start();
                    let rt = rt.clone();
                    rt.update(cx, |rt, cx| {
                        rt.set_liquid_glass_param(param_id, v);
                        cx.notify();
                    });
                },
            );

            sliders.push(slider);
            subscriptions.push(sub);
        }

        Self {
            runtime,
            sliders,
            _subscriptions: subscriptions,
        }
    }

    pub fn open_window(
        runtime: Entity<AppRuntime>,
        cx: &mut App,
    ) -> anyhow::Result<gpui::WindowHandle<Root>> {
        let bounds = Bounds::centered(None, size(px(480.0), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(SharedString::from("液态玻璃效果设置")),
                    ..Default::default()
                }),
                is_resizable: true,
                show: true,
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| Self::new(runtime, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let runtime = self.runtime.clone();
        let enabled = runtime.read(cx).get_liquid_glass_enabled();
        let over_light = runtime.read(cx).get_liquid_glass_over_light();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .p(px(12.0))
            .bg(rgb(0xf5f5f5))
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::BOLD)
                    .mb(px(8.0))
                    .child("💎 液态玻璃效果调节"),
            )
            // ===== enabled toggle =====
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .mb(px(8.0))
                    .child(
                        Button::new("toggle_enabled")
                            .primary()
                            .label(if enabled {
                                "✅ 液态玻璃已开启"
                            } else {
                                "⭕ 液态玻璃已关闭"
                            })
                            .on_click({
                                let rt = runtime.clone();
                                move |_, _window, cx| {
                                    rt.update(cx, |rt, cx| {
                                        rt.set_liquid_glass_enabled(!rt.get_liquid_glass_enabled());
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x666666))
                    .mb(px(8.0))
                    .child(if enabled {
                        "拖动滑块实时调节参数，自动保存"
                    } else {
                        "液态玻璃效果已关闭，分类框使用半透明背景"
                    }),
            )
            // ===== sliders (only when enabled) =====
            .child({
                let mut container = div().flex().flex_col().gap(px(4.0));
                if enabled {
                    for (i, param) in PARAMS.iter().enumerate() {
                        let slider = self.sliders[i].clone();
                        let value = slider.read(cx).value().start();
                        let display_value = match param.format {
                            ParamFormat::Number => format!("{:.2}", value),
                            ParamFormat::Percent => format!("{:.0}%", value * 100.0),
                        };
                        container = container.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .w(px(90.0))
                                        .text_xs()
                                        .text_color(rgb(0x333333))
                                        .child(param.label),
                                )
                                .child(div().flex_1().child(Slider::new(&slider)))
                                .child(
                                    div()
                                        .w(px(50.0))
                                        .text_xs()
                                        .text_color(rgb(0x666666))
                                        .child(display_value),
                                ),
                        );
                    }
                    let mode = runtime
                        .read(cx)
                        .get_liquid_glass_param(crate::config::LiquidGlassParam::RefractionMode)
                        as u32;
                    let mode_names = ["标准", "极坐标", "凸起", "程序化"];
                    let mut mode_row = div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .mt(px(8.0))
                        .child(
                            div()
                                .w(px(90.0))
                                .text_xs()
                                .text_color(rgb(0x333333))
                                .child("折射贴图"),
                        );
                    for (index, name) in mode_names.iter().enumerate() {
                        let rt = runtime.clone();
                        let label = if mode == index as u32 {
                            format!("✓ {name}")
                        } else {
                            (*name).to_string()
                        };
                        mode_row = mode_row.child(
                            Button::new(("refraction_mode", index))
                                .label(label)
                                .on_click(move |_, _window, cx| {
                                    rt.update(cx, |rt, cx| {
                                        rt.set_liquid_glass_param(
                                            crate::config::LiquidGlassParam::RefractionMode,
                                            index as f32,
                                        );
                                        cx.notify();
                                    });
                                }),
                        );
                    }
                    container = container.child(mode_row);
                    // over_light toggle
                    container = container.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .mt(px(8.0))
                            .child(
                                div()
                                    .w(px(130.0))
                                    .text_xs()
                                    .text_color(rgb(0x333333))
                                    .child("亮区减弱折射:"),
                            )
                            .child(
                                Button::new("toggle_over_light")
                                    .label(if over_light { "开" } else { "关" })
                                    .on_click({
                                        let rt = runtime.clone();
                                        move |_, _window, cx| {
                                            rt.update(cx, |rt, cx| {
                                                rt.set_liquid_glass_over_light(
                                                    !rt.get_liquid_glass_over_light(),
                                                );
                                                cx.notify();
                                            });
                                        }
                                    }),
                            ),
                    );
                }
                container
            })
            .child(div().h(px(1.0)).mt(px(8.0)).bg(rgb(0xcccccc)))
            .child(
                div().mt(px(8.0)).child(
                    Button::new("reset_defaults")
                        .primary()
                        .label("🔄 重置默认值")
                        .on_click({
                            let rt = runtime.clone();
                            move |_, _window, cx| {
                                rt.update(cx, |rt, cx| {
                                    rt.reset_liquid_glass_defaults();
                                    cx.notify();
                                });
                            }
                        }),
                ),
            )
    }
}
