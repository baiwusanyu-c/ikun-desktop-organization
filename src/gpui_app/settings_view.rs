//! Liquid Glass settings panel with GPUI Sliders.

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
}

const PARAMS: &[ParamDef] = &[
    ParamDef { param: crate::config::LiquidGlassParam::BlurRadius, label: "模糊半径", min: 0.5, max: 8.0, step: 0.1 },
    ParamDef { param: crate::config::LiquidGlassParam::RefractionStrength, label: "折射强度", min: 0.0, max: 30.0, step: 0.1 },
    ParamDef { param: crate::config::LiquidGlassParam::GlassAlpha, label: "玻璃透明度", min: 0.0, max: 1.0, step: 0.01 },
    ParamDef { param: crate::config::LiquidGlassParam::EdgeEffectWidth, label: "边缘过渡宽度", min: 4.0, max: 60.0, step: 1.0 },
    ParamDef { param: crate::config::LiquidGlassParam::ChromaticOffset, label: "色差偏移", min: 0.0, max: 5.0, step: 0.05 },
    ParamDef { param: crate::config::LiquidGlassParam::Contrast, label: "对比度", min: 0.5, max: 2.0, step: 0.01 },
    ParamDef { param: crate::config::LiquidGlassParam::Brightness, label: "亮度提升", min: 0.0, max: 0.2, step: 0.005 },
    ParamDef { param: crate::config::LiquidGlassParam::ColorGain, label: "颜色增益", min: 0.8, max: 1.5, step: 0.005 },
    ParamDef { param: crate::config::LiquidGlassParam::ColorLift, label: "颜色提升", min: 0.0, max: 0.1, step: 0.002 },
    ParamDef { param: crate::config::LiquidGlassParam::ShadowOffsetY, label: "阴影偏移 Y", min: 0.0, max: 20.0, step: 0.5 },
    ParamDef { param: crate::config::LiquidGlassParam::ShadowSoftness, label: "阴影柔和度", min: 1.0, max: 30.0, step: 0.5 },
    ParamDef { param: crate::config::LiquidGlassParam::ShadowOpacity, label: "阴影不透明度", min: 0.0, max: 1.0, step: 0.01 },
    ParamDef { param: crate::config::LiquidGlassParam::TopLightPosition, label: "顶部高光位置", min: 0.0, max: 1.0, step: 0.01 },
    ParamDef { param: crate::config::LiquidGlassParam::TopLightStrength, label: "顶部高光强度", min: 0.0, max: 0.5, step: 0.01 },
    ParamDef { param: crate::config::LiquidGlassParam::LowerShadowPosition, label: "底部阴影位置", min: 0.0, max: 1.0, step: 0.01 },
    ParamDef { param: crate::config::LiquidGlassParam::LowerShadowStrength, label: "底部阴影强度", min: 0.0, max: 0.5, step: 0.01 },
    ParamDef { param: crate::config::LiquidGlassParam::CornerRadius, label: "圆角半径", min: 0.0, max: 50.0, step: 1.0 },
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
        let values: Vec<f32> = PARAMS.iter()
            .map(|p| glass.get_param(p.param))
            .collect();

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
            let sub = cx.subscribe_in(&slider, window, move |_this, _state, event: &SliderEvent, _w, cx| {
                let SliderEvent::Change(val) = event;
                let v = val.start();
                rt.update(cx, |rt, cx| {
                    rt.set_liquid_glass_param(param_id, v);
                    cx.notify();
                });
            });

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
        let bounds = Bounds::centered(None, size(px(480.0), px(720.0)), cx);
        let result = cx.open_window(
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
        );
        result
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let runtime = self.runtime.clone();

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
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x666666))
                    .mb(px(8.0))
                    .child("拖动滑块实时调节参数，自动保存"),
            )
            .child({
                let mut container = div().flex().flex_col().gap(px(4.0));
                for (i, param) in PARAMS.iter().enumerate() {
                    let slider = self.sliders[i].clone();
                    let value = slider.read(cx).value().start();
                    container = container.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .w(px(110.0))
                                    .text_xs()
                                    .text_color(rgb(0x333333))
                                    .child(param.label),
                            )
                            .child(
                                div().flex_1().child(Slider::new(&slider)),
                            )
                            .child(
                                div()
                                    .w(px(50.0))
                                    .text_xs()
                                    .text_color(rgb(0x666666))
                                    .child(format!("{:.2}", value)),
                            ),
                    );
                }
                container
            })
            .child(
                div().h(px(1.0)).mt(px(8.0)).bg(rgb(0xcccccc)),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .child(
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
