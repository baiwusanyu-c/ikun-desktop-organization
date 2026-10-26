//! 液态玻璃引擎适配层（liquid-verre-rs）
//!
//! 本模块是分类框平台层使用 `liquid-verre-rs` 引擎的唯一入口，负责：
//! - `Engine` 生命周期（启动 / 关闭 / 引擎级配置变化时的自动重建）；
//! - 分类框 `box_id` ↔ `GlassWindow` 句柄注册表（含玻璃块几何的最后已知值）；
//! - 物理像素（窗口矩形）↔ 逻辑像素（库坐标系）的换算；
//! - 项目配置 `LiquidGlassConfig` → 库 `GlassStyle` / `ResizeOptions` / `EngineConfig` 的映射；
//! - `EngineEvent` 转化为项目日志系统的中文日志。
//!
//! # 交互分工（与 `wnd_proc` / `grid_window` 的契约）
//! - 玻璃底板（液态或透明模式）的绘制、边缘/对角缩放拖拽与悬停加粗 border 反馈
//!   由库画布（每屏一个，`WindowLevel::AlwaysOnBottom`）承担；
//! - 标题栏拖拽移动仍走分类框自身的原生移动循环（保留双击编辑标题等交互），
//!   在 `WM_MOVING` 中把新位置同步给玻璃块（`push_box_rect`）；
//! - 分类框窗口是「内容层」：只在玻璃之上绘制图标、栅格、标题与按钮，
//!   在库激活时把边缘把手带与阴影留白的命中让给库画布（返回 `HTTRANSPARENT`）。
//!
//! # Z 序不变量
//! 库画布必须始终位于所有分类框内容窗口之下、桌面之上。分类框被推到底部
//! （`zorder::push_other_grids_below` / `WM_WINDOWPOSCHANGING` 干预）之后，
//! 调用方必须跟进 `reassert_canvas_bottom()`，把画布重新压回 Z 序最底部。

use std::collections::HashMap;
use std::sync::Mutex;

use uuid::Uuid;

use liquid_verre::{
    CaptureBackend, CaptureConfig, DiagnosticsConfig, Engine, EngineConfig, EngineEvent,
    GlassStyle, LiquidParams, LogLevel, Profile as VerreProfile, ResizeFeedbackOptions,
    ResizeOptions, Shape as VerreShape, TransparentParams, WindowConfig, WindowLevel,
};

use crate::config::app_config::{
    AppConfig, GlassCaptureBackend, GlassEngineConfig, GlassLiquidParams, GlassProfile,
    GlassResizeConfig, GlassShape, LiquidGlassConfig,
};

use super::ffi_types::RECT;
use super::monitor_utils::get_primary_monitor_dpi;

/// 单个分类框的玻璃窗口登记项。
struct GlassBoxEntry {
    /// 库玻璃窗口句柄（命令队列 + 快照，跨线程安全）。
    window: GlassWindow,
    /// 内容层分类框窗口句柄（isize 形式，用于引擎事件回调线程 PostMessage）。
    hwnd: isize,
    /// 最后一次推送给库的内容矩形（逻辑像素），用于去重。
    last_logical: [f32; 4],
    /// 内容矩形所在屏的 DPI 比例（该屏 DPI / 主屏 DPI），用于回读换算。
    ratio: f32,
    /// 当前是否处于锁定（固定）状态。
    locked: bool,
}

/// 引擎持有者（进程内单份）。
struct GlassEngineHolder {
    engine: Engine,
    /// box_id → 玻璃窗口登记项。
    boxes: HashMap<Uuid, GlassBoxEntry>,
    /// 引擎级配置快照，用于判断是否需要重建。
    engine_cfg: GlassEngineConfig,
    /// 外观配置快照（含模式开关），用于新窗口创建与样式刷新。
    style_cfg: LiquidGlassConfig,
}

/// 全局引擎状态。`None` 表示引擎未启动或已永久失败（平台层回退旧渲染路径）。
static HOLDER: Mutex<Option<GlassEngineHolder>> = Mutex::new(None);

/// 库玻璃 id → 分类框窗口句柄的独立登记表（缩放会话事件路由专用）。
///
/// 引擎事件回调在渲染线程触发，且发生在库内部 `handler` 读锁持有期间；
/// 若回调再去锁 `HOLDER`，会与「主线程持 `HOLDER` 等 `handler` 写锁」构成
/// 三方锁序环（实测启动即死锁）。因此回调路径只允许碰这张极轻量的表：
/// 表操作 microsecond 级，且持本表锁时绝不调用任何引擎 API。
lazy_static::lazy_static! {
    static ref SESSION_REGISTRY: Mutex<HashMap<u32, isize>> = Mutex::new(HashMap::new());
}

fn registry_register(glass_id: u32, hwnd: isize) {
    if let Ok(mut map) = SESSION_REGISTRY.lock() {
        map.insert(glass_id, hwnd);
    }
}

fn registry_unregister(glass_id: u32) {
    if let Ok(mut map) = SESSION_REGISTRY.lock() {
        map.remove(&glass_id);
    }
}

fn registry_clear() {
    if let Ok(mut map) = SESSION_REGISTRY.lock() {
        map.clear();
    }
}

// =============================================================================
// 纯换算函数（可单测）
// =============================================================================

/// 计算目标屏相对主屏的 DPI 比例（`dpi / 主屏 dpi`），非法输入回退 1.0。
#[must_use]
pub(crate) fn monitor_dpi_ratio(dpi: u32) -> f32 {
    let primary = get_primary_monitor_dpi().max(96) as f32;
    let ratio = dpi as f32 / primary;
    if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    }
}

/// 物理内容矩形 → 库逻辑矩形。
///
/// 库语义：`x/y` 直接使用虚拟屏物理像素；`w/h` 为逻辑像素（主屏物理像素单位），
/// 块物理尺寸 = 逻辑尺寸 × 所在屏 DPI 比例（见库 `Command::SetRect`）。
#[must_use]
pub(crate) fn phys_to_logical_rect(phys: &RECT, ratio: f32) -> [f32; 4] {
    let ratio = if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    };
    let w = (phys.right - phys.left) as f32 / ratio;
    let h = (phys.bottom - phys.top) as f32 / ratio;
    [phys.left as f32, phys.top as f32, w, h]
}

/// 库逻辑矩形 → 物理矩形（分类框内容窗口应占位的区域）。
#[must_use]
pub(crate) fn logical_to_phys_rect(logical: &[f32; 4], ratio: f32) -> RECT {
    let ratio = if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    };
    RECT {
        left: logical[0].round() as i32,
        top: logical[1].round() as i32,
        right: (logical[0] + logical[2] * ratio).round() as i32,
        bottom: (logical[1] + logical[3] * ratio).round() as i32,
    }
}

/// 项目液态参数 → 库 `LiquidParams`。
#[must_use]
pub(crate) fn liquid_params_from(params: &GlassLiquidParams) -> LiquidParams {
    LiquidParams {
        corner_radius: params.corner_radius,
        lens_coverage: params.lens_coverage,
        bevel_width: params.bevel_width,
        shape: match params.shape {
            GlassShape::RoundedRect => VerreShape::RoundedRect,
            GlassShape::Squircle => VerreShape::Squircle,
        },
        super_n: params.super_n,
        profile: match params.profile {
            GlassProfile::Circular => VerreProfile::Circular,
            GlassProfile::Parabolic => VerreProfile::Parabolic,
            GlassProfile::Sinusoidal => VerreProfile::Sinusoidal,
        },
        ior: params.ior,
        thickness: params.thickness,
        refr_strength: params.refr_strength,
        dispersion: params.dispersion,
        absorption: params.absorption,
        clarity: params.clarity,
        blur_max: params.blur_max,
        blur_min: params.blur_min,
        progressive_curve: params.progressive_curve,
        edge_always: params.edge_always,
        tint_opacity: params.tint_opacity,
        adaptive_tint: params.adaptive_tint,
        base_alpha: params.base_alpha,
        tint_color: params.tint_color,
        env_tint_mix: params.env_tint_mix,
        light_dir: params.light_dir,
        light_intensity: params.light_intensity,
        spec_intensity: params.spec_intensity,
        spec_power: params.spec_power,
        spec_spread: params.spec_spread,
        fresnel_f0: params.fresnel_f0,
        rim_width: params.rim_width,
        caustic_gain: params.caustic_gain,
        mouse_radius: params.mouse_radius,
        mouse_intensity: params.mouse_intensity,
        mouse_spread: params.mouse_spread,
        mouse_color: params.mouse_color,
        proximity_glow: params.proximity_glow,
        shadow_opacity: params.shadow_opacity,
        shadow_margin: params.shadow_margin,
        lag_seconds: params.lag_seconds,
    }
}

/// 项目缩放交互配置 → 库 `ResizeOptions`。
#[must_use]
pub(crate) fn resize_options_from(cfg: &GlassResizeConfig) -> ResizeOptions {
    ResizeOptions {
        enabled: cfg.enabled,
        edges: cfg.edges,
        corners: cfg.corners,
        grip_px: cfg.grip_px,
        min_width: cfg.min_width,
        min_height: cfg.min_height,
        feedback: ResizeFeedbackOptions {
            enabled: cfg.feedback.enabled,
            border_width: cfg.feedback.border_width,
            alpha: cfg.feedback.alpha,
            color: cfg.feedback.color,
        },
    }
}

/// 项目外观配置 → 库 `GlassStyle`（液态开关决定两种模式）。
#[must_use]
pub(crate) fn glass_style_from(cfg: &LiquidGlassConfig) -> GlassStyle {
    if cfg.enabled {
        GlassStyle::Liquid(liquid_params_from(&cfg.liquid))
    } else {
        GlassStyle::Transparent(TransparentParams {
            corner_radius: cfg.transparent.corner_radius,
            opacity: cfg.transparent.opacity,
            color: cfg.transparent.color,
        })
    }
}

/// 采集后端映射。
#[must_use]
fn capture_backend_from(backend: GlassCaptureBackend) -> CaptureBackend {
    match backend {
        GlassCaptureBackend::Auto => CaptureBackend::Auto,
        GlassCaptureBackend::ForceDda => CaptureBackend::ForceDda,
        GlassCaptureBackend::ForceWgc => CaptureBackend::ForceWgc,
    }
}

/// 项目配置 → 库 `EngineConfig`。
///
/// 层级固定 `AlwaysOnBottom`：分类框是桌面收纳件，必须低于其他应用窗口、
/// 高于桌面本身，与重构前 `HWND_BOTTOM` 的窗口语义一致。
///
/// `log_level` 固定 `Trace`：库构造时会用它调 `log::set_max_level`（全局门闩）。
/// 若按玻璃配置收紧，会把**整个项目**的低级别日志一并掐死；过滤权必须留给
/// 项目自己的 env_logger 过滤器。
#[must_use]
pub(crate) fn engine_config_from(app: &AppConfig) -> EngineConfig {
    let glass = &app.box_style.liquid_glass;
    EngineConfig {
        default_style: glass_style_from(glass),
        window_level: WindowLevel::AlwaysOnBottom,
        default_resize: resize_options_from(&glass.resize),
        capture: CaptureConfig {
            backend: capture_backend_from(glass.engine.capture_backend),
            ..CaptureConfig::default()
        },
        adaptive_quality: glass.engine.adaptive_quality,
        occlusion_culling: glass.engine.occlusion_culling,
        fullscreen_hide: glass.engine.fullscreen_hide,
        fps_cap: if glass.engine.fps_cap == 0 {
            None
        } else {
            Some(glass.engine.fps_cap)
        },
        monitor_limit: if glass.engine.monitor_limit == 0 {
            None
        } else {
            Some(glass.engine.monitor_limit as usize)
        },
        log_level: LogLevel::Trace,
        diagnostics: DiagnosticsConfig::default(),
    }
}

// 引入句柄类型（避免在顶部 import 列表里淹没类型名）。
use liquid_verre::GlassWindow;

// =============================================================================
// 引擎生命周期
// =============================================================================

/// 在渲染线程存在之前装好事件回调，再启动线程。
///
/// `Engine::spawn` 等价于 new+start 一步到位：渲染线程可能在几毫秒内 emit
/// （Ready / DPI 告警）并在库内持 `handler` 读锁执行回调；若主线程此间还在
/// `HOLDER` 临界区内 `set_event_handler` 等写锁，即构成三方锁环（实测启动死锁）。
/// `new` 之后渲染线程尚不存在，写 handler 无竞争；`start` 后主线程不再触碰写锁。
fn start_engine_with_handler(mut engine: Engine) -> Result<Engine, liquid_verre::Error> {
    engine.set_event_handler(Some(std::sync::Arc::new(|event| match event {
        EngineEvent::ResizeStarted { id, .. } => {
            dispatch_resize_session(*id, true);
        }
        EngineEvent::ResizeEnded { id, .. } => {
            dispatch_resize_session(*id, false);
        }
        _ => {}
    })));
    engine.start()?;
    Ok(engine)
}

/// 确保玻璃引擎已启动。返回引擎是否可用。
///
/// 已启动时直接返回可用状态；启动失败时记录中文日志并保持 `None`
/// （平台层据此回退旧 D2D 背景 + 原生拖拽/缩放路径）。
pub fn ensure_engine(app: &AppConfig) -> bool {
    let mut guard = match HOLDER.lock() {
        Ok(g) => g,
        Err(e) => {
            log::error!("玻璃引擎锁获取失败: {e}");
            return false;
        }
    };
    if let Some(holder) = guard.as_mut() {
        if holder.engine.is_running() {
            return true;
        }
        // 渲染线程已因致命错误退出：就地重建，恢复玻璃能力。
        log::warn!("玻璃引擎渲染线程已退出，尝试重建引擎");
        let style_cfg = holder.style_cfg;
        let engine_cfg = holder.engine_cfg;
        let boxes = collect_boxes_for_rebuild(holder);
        *guard = None;
        drop(guard);
        registry_clear();
        return spawn_holder_and_reattach(app, style_cfg, engine_cfg, boxes);
    }
    let config = engine_config_from(app);
    let engine = match Engine::new(config) {
        Ok(engine) => engine,
        Err(e) => {
            log::error!("玻璃引擎启动失败: {e}，分类框回退旧渲染路径");
            return false;
        }
    };
    let engine = match start_engine_with_handler(engine) {
        Ok(engine) => engine,
        Err(e) => {
            log::error!("玻璃引擎渲染线程启动失败: {e}，分类框回退旧渲染路径");
            return false;
        }
    };
    log::info!(
        "玻璃引擎已启动（liquid-verre-rs v{}）",
        liquid_verre::BUILD_INFO.version
    );
    *guard = Some(GlassEngineHolder {
        engine,
        boxes: HashMap::new(),
        engine_cfg: app.box_style.liquid_glass.engine,
        style_cfg: app.box_style.liquid_glass,
    });
    true
}

/// 以只读方式访问引擎持有者（引擎未启动 / 中毒锁时返回 None）。
fn with_holder<R>(f: impl FnOnce(&GlassEngineHolder) -> R) -> Option<R> {
    let guard = HOLDER.lock().ok()?;
    Some(f(guard.as_ref()?))
}

/// 玻璃引擎是否可用（渲染线程存活）。
#[must_use]
pub fn is_active() -> bool {
    with_holder(|holder| holder.engine.is_running()).unwrap_or(false)
}

/// 关闭引擎并清空全部玻璃块（应用退出路径）。
pub fn shutdown() {
    if let Ok(mut guard) = HOLDER.lock() {
        if let Some(mut holder) = guard.take() {
            for entry in holder.boxes.values() {
                entry.window.close();
            }
            holder.boxes.clear();
            holder.engine.shutdown();
            registry_clear();
            log::info!("玻璃引擎已关闭");
        }
    }
}

// =============================================================================
// 分类框 ↔ 玻璃块注册表
// =============================================================================

/// 为分类框创建玻璃底板块。成功返回 `true`（引擎未激活时返回 `false`，调用方回退旧路径）。
///
/// `content_phys` 为分类框**内容矩形**（不含外投影留白）的屏幕物理像素矩形；
/// `locked` 为当前固定状态（锁定时玻璃块不可命中，缩放/拖拽由内容层自理）。
pub fn attach_box(box_id: Uuid, hwnd: isize, content_phys: &RECT, dpi: u32, locked: bool) -> bool {
    let Ok(mut guard) = HOLDER.lock() else {
        return false;
    };
    let Some(holder) = guard.as_mut() else {
        return false;
    };
    if holder.boxes.contains_key(&box_id) {
        // 已存在（如重复恢复布局）：改为同步几何。
        return update_box_rect_locked(holder, box_id, content_phys, dpi);
    }
    let ratio = monitor_dpi_ratio(dpi);
    let logical = phys_to_logical_rect(content_phys, ratio);
    let config = WindowConfig {
        rect: logical,
        style: Some(glass_style_from(&holder.style_cfg)),
        resize: resize_options_from(&holder.style_cfg.resize),
        z_order: None,
        interactive: !locked,
        // 位置由分类框原生拖拽驱动（push_box_rect），库不需要自己的移动会话；
        // 边缘缩放会话与悬停 border 反馈是库的核心替换能力，保持开启。
        movable: false,
        visible: true,
    };
    match holder.engine.create_window(config) {
        Ok(window) => {
            registry_register(window.id(), hwnd);
            holder.boxes.insert(
                box_id,
                GlassBoxEntry {
                    window,
                    hwnd,
                    last_logical: logical,
                    ratio,
                    locked,
                },
            );
            true
        }
        Err(e) => {
            log::warn!("玻璃块创建失败（box={box_id}）: {e}，该分类框回退旧渲染路径");
            false
        }
    }
}

/// 移除分类框的玻璃块（窗口销毁路径）。
pub fn detach_box(box_id: Uuid) {
    if let Ok(mut guard) = HOLDER.lock() {
        if let Some(holder) = guard.as_mut() {
            if let Some(entry) = holder.boxes.remove(&box_id) {
                registry_unregister(entry.window.id());
                entry.window.close();
            }
        }
    }
}

/// 把分类框内容窗口的新位置同步到玻璃块（标题栏原生拖拽的 `WM_MOVING` 路径）。
///
/// 去重：与最后已知逻辑矩形一致时不下发命令，避免消息风暴。
pub fn push_box_rect(box_id: Uuid, content_phys: &RECT, dpi: u32) {
    let Ok(mut guard) = HOLDER.lock() else {
        return;
    };
    let Some(holder) = guard.as_mut() else {
        return;
    };
    update_box_rect_locked(holder, box_id, content_phys, dpi);
}

fn update_box_rect_locked(
    holder: &mut GlassEngineHolder,
    box_id: Uuid,
    content_phys: &RECT,
    dpi: u32,
) -> bool {
    let Some(entry) = holder.boxes.get_mut(&box_id) else {
        return false;
    };
    let ratio = monitor_dpi_ratio(dpi);
    let logical = phys_to_logical_rect(content_phys, ratio);
    entry.ratio = ratio;
    if approx_eq_rect(&entry.last_logical, &logical) {
        return true;
    }
    entry.window.set_rect(logical);
    entry.last_logical = logical;
    true
}

/// 缩放会话结束后，把库权威矩形回读为分类框内容矩形（物理像素）。
///
/// 库在渲染线程上应用把手拖拽并做中心钳制，`WindowSnapshot.rect` 是最终真值；
/// 内容层据此贴回玻璃块。返回 `None` 表示该框没有玻璃块或快照不可用。
#[must_use]
pub fn block_content_rect(box_id: Uuid) -> Option<RECT> {
    with_holder(|holder| {
        let entry = holder.boxes.get(&box_id)?;
        let snapshot = entry.window.snapshot()?;
        Some(logical_to_phys_rect(&snapshot.rect, entry.ratio))
    })?
}

/// 该玻璃块当前是否正处于库驱动的缩放会话（内容层跟随定时器据此开关）。
#[must_use]
pub fn block_resizing(box_id: Uuid) -> bool {
    with_holder(|holder| {
        holder
            .boxes
            .get(&box_id)
            .and_then(|e| e.window.snapshot())
            .is_some_and(|s| s.resizing)
    })
    .unwrap_or(false)
}

/// 同步固定（锁定）状态：锁定时玻璃块整体退出生效命中区，
/// 解锁时按当前配置恢复边缘缩放。
pub fn set_box_locked(box_id: Uuid, locked: bool) {
    let Ok(mut guard) = HOLDER.lock() else {
        return;
    };
    let Some(holder) = guard.as_mut() else {
        return;
    };
    let Some(entry) = holder.boxes.get_mut(&box_id) else {
        return;
    };
    if entry.locked == locked {
        return;
    }
    entry.locked = locked;
    entry.window.set_interactive(!locked);
    if !locked {
        entry
            .window
            .set_resize_options(resize_options_from(&holder.style_cfg.resize));
    }
}

/// 把分类框抬到玻璃层最前（与内容窗口激活置顶同步）。
pub fn raise_box(box_id: Uuid) {
    if let Ok(guard) = HOLDER.lock() {
        if let Some(holder) = guard.as_ref() {
            if let Some(entry) = holder.boxes.get(&box_id) {
                entry.window.bring_to_front();
            }
        }
    }
}

/// 当前已登记的玻璃块数量（诊断 / 测试用）。
#[must_use]
pub fn box_count() -> usize {
    with_holder(|holder| holder.boxes.len()).unwrap_or(0)
}

// =============================================================================
// 配置应用与引擎重建
// =============================================================================

/// 外观/交互配置变化：引擎级参数变化时自动重建引擎并重挂全部玻璃块，
/// 否则仅在线刷新默认样式与逐块样式。
pub fn apply_settings(app: &AppConfig) {
    let glass = &app.box_style.liquid_glass;
    let mut guard = match HOLDER.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Some(holder) = guard.as_mut() else {
        return;
    };
    if holder.engine_cfg != glass.engine {
        let old_engine_cfg = holder.engine_cfg;
        log::info!("玻璃引擎级配置变化，重建引擎以应用新参数");
        let boxes = collect_boxes_for_rebuild(holder);
        *guard = None;
        drop(guard);
        registry_clear();
        // 在锁外重建：spawn_holder_and_reattach 自行加锁，避免持锁再入。
        if !spawn_holder_and_reattach(app, *glass, glass.engine, boxes) {
            // 重建失败（新配置无法启动引擎）：恢复旧引擎级配置记录并回退旧路径。
            log::warn!("玻璃引擎重建失败，回退旧渲染路径（旧引擎级配置: {old_engine_cfg:?}）");
        }
        return;
    }
    holder.style_cfg = *glass;
    let style = glass_style_from(glass);
    let resize = resize_options_from(&glass.resize);
    holder.engine.set_default_style(style);
    holder.engine.set_default_resize(resize);
    for entry in holder.boxes.values_mut() {
        entry.window.set_style(Some(style));
        if !entry.locked {
            entry.window.set_resize_options(resize);
        }
    }
    holder.engine.request_render();
}

/// 收集重建引擎所需的玻璃块几何（逻辑矩形 + DPI 比例 + 锁定状态）。
fn collect_boxes_for_rebuild(
    holder: &GlassEngineHolder,
) -> Vec<(Uuid, isize, [f32; 4], f32, bool)> {
    holder
        .boxes
        .iter()
        .map(|(id, e)| (*id, e.hwnd, e.last_logical, e.ratio, e.locked))
        .collect()
}

/// 用给定配置启动新引擎，并把旧玻璃块按最后已知几何重新挂载。
fn spawn_holder_and_reattach(
    app: &AppConfig,
    style_cfg: LiquidGlassConfig,
    engine_cfg: GlassEngineConfig,
    boxes: Vec<(Uuid, isize, [f32; 4], f32, bool)>,
) -> bool {
    let mut config = engine_config_from(app);
    config.default_style = glass_style_from(&style_cfg);
    config.default_resize = resize_options_from(&style_cfg.resize);
    let engine = match Engine::new(config) {
        Ok(engine) => engine,
        Err(e) => {
            log::error!("玻璃引擎重建失败: {e}，分类框回退旧渲染路径");
            return false;
        }
    };
    let engine = match start_engine_with_handler(engine) {
        Ok(engine) => engine,
        Err(e) => {
            log::error!("玻璃引擎重建启动渲染线程失败: {e}，分类框回退旧渲染路径");
            return false;
        }
    };
    let mut holder = GlassEngineHolder {
        engine,
        boxes: HashMap::new(),
        engine_cfg,
        style_cfg,
    };
    for (id, hwnd, logical, ratio, locked) in boxes {
        let window_config = WindowConfig {
            rect: logical,
            style: Some(glass_style_from(&style_cfg)),
            resize: resize_options_from(&style_cfg.resize),
            z_order: None,
            interactive: !locked,
            movable: false,
            visible: true,
        };
        match holder.engine.create_window(window_config) {
            Ok(window) => {
                registry_register(window.id(), hwnd);
                holder.boxes.insert(
                    id,
                    GlassBoxEntry {
                        window,
                        hwnd,
                        last_logical: logical,
                        ratio,
                        locked,
                    },
                );
            }
            Err(e) => {
                log::warn!("引擎重建时玻璃块恢复失败（box={id}）: {e}");
            }
        }
    }
    if let Ok(mut guard) = HOLDER.lock() {
        *guard = Some(holder);
        log::info!("玻璃引擎重建完成");
        return true;
    }
    log::error!("玻璃引擎重建收尾失败：HOLDER 锁不可用");
    false
}

// =============================================================================
// Z 序与事件
// =============================================================================

/// 把库缩放会话的开始/结束转发到对应分类框窗口的消息循环。
///
/// 只在渲染线程回调里执行：查表 + 非阻塞 PostMessage，不重入引擎、不加锁等待。
fn dispatch_resize_session(glass_id: u32, started: bool) {
    let hwnd = match SESSION_REGISTRY.lock() {
        Ok(map) => map.get(&glass_id).copied(),
        Err(_) => None,
    };
    let Some(hwnd) = hwnd.filter(|h| *h != 0) else {
        return;
    };
    unsafe {
        super::ffi_imports::PostMessageW(
            hwnd as *mut std::ffi::c_void,
            super::ffi_types::WM_USER_GLASS_RESIZE,
            if started { 1 } else { 0 },
            0,
        );
    }
}

/// 把各屏画布重新压回 Z 序最底部（分类框内容窗口被推底后必须跟进调用，
/// 维持「画布 < 全部玻璃块内容窗口 < 其他应用窗口」不变量）。
pub fn reassert_canvas_bottom() {
    if let Ok(guard) = HOLDER.lock() {
        if let Some(holder) = guard.as_ref() {
            if holder.engine.is_running() {
                holder.engine.set_window_level(WindowLevel::AlwaysOnBottom);
            }
        }
    }
}

/// 轮询引擎事件队列并转入项目日志系统（runtime tick 调用）。
///
/// 全部使用低频路径日志：这些事件只在启动、显示器变化、采集失效等场景出现，
/// 不属于每帧热路径。
pub fn poll_events_to_log() {
    let Ok(guard) = HOLDER.lock() else {
        return;
    };
    let Some(holder) = guard.as_ref() else {
        return;
    };
    for event in holder.engine.poll_events() {
        match event {
            EngineEvent::Ready { monitor_count } => {
                log::info!("玻璃引擎就绪，参与渲染的显示器数量: {monitor_count}");
            }
            EngineEvent::MonitorsChanged { monitor_count } => {
                log::info!("显示器集合变化，画布与采集已重建，显示器数量: {monitor_count}");
            }
            EngineEvent::WindowCreated { id } => {
                log::debug!("玻璃块已创建: id={id}");
            }
            EngineEvent::WindowDestroyed { id } => {
                log::debug!("玻璃块已销毁: id={id}");
            }
            EngineEvent::MoveStarted { id } => {
                log::debug!("玻璃块开始移动: id={id}");
            }
            EngineEvent::MoveEnded { id, rect } => {
                log::debug!("玻璃块移动结束: id={id}, rect={rect:?}");
            }
            EngineEvent::ResizeStarted { id, handle } => {
                log::debug!("玻璃块开始缩放: id={id}, 把手={handle}");
            }
            EngineEvent::ResizeEnded { id, rect } => {
                log::info!("玻璃块缩放结束: id={id}, rect={rect:?}");
            }
            EngineEvent::LevelApplied { level } => {
                log::debug!("画布层级已应用: {level:?}");
            }
            EngineEvent::CaptureBackendSelected {
                monitor_index,
                monitor_name,
                backend,
            } => {
                log::info!("显示器 {monitor_name} 采集后端: {backend} (下标 {monitor_index})");
            }
            EngineEvent::CaptureLost {
                monitor_index,
                kind,
                reason,
            } => {
                log::warn!(
                    "显示器 {monitor_index} 桌面采集失效（{kind:?}）: {reason}，引擎将自动退避重建"
                );
            }
            EngineEvent::CaptureRestored { monitor_index } => {
                log::info!("显示器 {monitor_index} 桌面采集已恢复");
            }
            EngineEvent::ExclusiveFullscreen {
                monitor_index,
                active,
            } => {
                if active {
                    log::info!("显示器 {monitor_index} 检测到独占全屏应用，玻璃画布临时隐藏让路");
                } else {
                    log::info!("显示器 {monitor_index} 独占全屏应用已退出，玻璃画布恢复");
                }
            }
            EngineEvent::QualityLevelChanged { level } => {
                log::info!("玻璃自适应质量档位变化: {level}（0=不降档）");
            }
            EngineEvent::Warning { kind, message } => {
                log::warn!("玻璃引擎告警（{kind:?}）: {message}");
            }
            EngineEvent::Fatal { kind, message } => {
                log::error!("玻璃引擎致命错误（{kind:?}）: {message}，分类框将回退旧渲染路径");
            }
            EngineEvent::EventsDropped { count } => {
                log::warn!("玻璃引擎事件队列溢出，丢弃了 {count} 条最旧事件");
            }
            EngineEvent::Shutdown => {
                log::info!("玻璃引擎已退出");
            }
            _ => {}
        }
    }
}

// =============================================================================
// 内部小工具
// =============================================================================

/// 当前生效的分类框样式缓存（供内容层渲染读取，替代旧 `liquid_glass::get_box_style_config`）。
static BOX_STYLE_CACHE: Mutex<Option<crate::config::app_config::BoxStyleConfig>> = Mutex::new(None);

/// 缓存当前分类框样式（配置应用线程调用，渲染线程读取）。
pub fn set_box_style_cache(style: &crate::config::app_config::BoxStyleConfig) {
    if let Ok(mut guard) = BOX_STYLE_CACHE.lock() {
        *guard = Some(style.clone());
    }
}

/// 库缩放把手带的物理像素宽度（内容层据此保留透明穿透带）。
///
/// 与库 `hit_resize` 命中判据同源：带宽 = `grip_px × 块所在屏 DPI 比例`；
/// 缩放总开关关闭或本块锁定时返回 0（内容层整块捕获，不留穿透带）。
#[must_use]
pub fn resize_grip_band(box_id: Uuid) -> f32 {
    with_holder(|holder| {
        let entry = holder.boxes.get(&box_id)?;
        if entry.locked || !holder.style_cfg.resize.enabled {
            return Some(0.0_f32);
        }
        Some((holder.style_cfg.resize.grip_px * entry.ratio).max(0.0))
    })
    .flatten()
    .unwrap_or(0.0)
}

/// 读取当前分类框样式缓存（未初始化时返回默认样式）。
#[must_use]
pub fn box_style() -> crate::config::app_config::BoxStyleConfig {
    match BOX_STYLE_CACHE.lock() {
        Ok(guard) => guard.clone().unwrap_or_default(),
        Err(_) => crate::config::app_config::BoxStyleConfig::default(),
    }
}

/// 该分类框是否已登记玻璃块（内容层渲染据此选择引擎路径 / 回退路径）。
#[must_use]
pub fn has_box(box_id: Uuid) -> bool {
    with_holder(|holder| {
        holder
            .boxes
            .get(&box_id)
            .is_some_and(|e| e.window.is_alive())
    })
    .unwrap_or(false)
}

/// 引擎周期指标（供 mem_probe 低频日志）：`(渲染 fps, 自适应质量档位, 显示器数)`。
#[must_use]
pub fn engine_metrics() -> Option<(f32, u8, usize)> {
    with_holder(|holder| {
        if !holder.engine.is_running() {
            return None;
        }
        let snapshot = holder.engine.snapshot();
        Some((
            snapshot.fps,
            snapshot.quality_level,
            snapshot.monitors.len(),
        ))
    })?
}

/// 把分类框当前窗口矩形（外框，含留白）同步为玻璃块几何。
///
/// `WM_WINDOWPOSCHANGED` / `WM_MOVING` 统一入口：内部按 `outer` 裁掉留白
/// 还原内容矩形，再走与 `push_box_rect` 相同的去重下发。
pub fn push_geometry_for_window(box_id: Uuid, outer_phys: &RECT, padding: i32, dpi: u32) {
    let content = RECT {
        left: outer_phys.left + padding,
        top: outer_phys.top + padding,
        right: outer_phys.right - padding,
        bottom: outer_phys.bottom - padding,
    };
    push_box_rect(box_id, &content, dpi);
}

/// 逻辑矩形近似相等（避免 0.01 单位内的抖动触发无谓的 SetRect）。
fn approx_eq_rect(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
    }

    #[test]
    fn phys_logical_roundtrip_on_primary() {
        // 主屏（比例 1.0）：位置与尺寸原样往返。
        let src = rect(100, 200, 100 + 680, 200 + 800);
        let logical = phys_to_logical_rect(&src, 1.0);
        assert_eq!(logical, [100.0, 200.0, 680.0, 800.0]);
        let back = logical_to_phys_rect(&logical, 1.0);
        assert_eq!(
            (back.left, back.top, back.right, back.bottom),
            (100, 200, 780, 1000)
        );
    }

    #[test]
    fn phys_logical_roundtrip_on_scaled_monitor() {
        // 150% 屏：尺寸按 DPI 比例折叠，回读还原物理尺寸；位置保持物理像素。
        let src = rect(-1920, 0, -1920 + 1020, 600);
        let logical = phys_to_logical_rect(&src, 1.5);
        assert!((logical[2] - 680.0).abs() < 0.01);
        assert!((logical[3] - 400.0).abs() < 0.01);
        assert_eq!(logical[0], -1920.0);
        let back = logical_to_phys_rect(&logical, 1.5);
        assert!((back.right - back.left - 1020).abs() <= 1);
        assert!((back.bottom - back.top - 600).abs() <= 1);
    }

    #[test]
    fn invalid_ratio_falls_back_to_one() {
        let src = rect(0, 0, 100, 50);
        assert_eq!(
            phys_to_logical_rect(&src, f32::NAN),
            [0.0, 0.0, 100.0, 50.0]
        );
        assert_eq!(phys_to_logical_rect(&src, 0.0), [0.0, 0.0, 100.0, 50.0]);
    }

    #[test]
    fn style_maps_mode_and_key_fields() {
        let mut cfg = LiquidGlassConfig::default();
        cfg.enabled = true;
        cfg.liquid.corner_radius = 42.0;
        cfg.liquid.tint_color = [0.1, 0.2, 0.3, 1.0];
        let style = glass_style_from(&cfg);
        let GlassStyle::Liquid(params) = style else {
            panic!("enabled=true 必须映射为液态模式");
        };
        assert!((params.corner_radius - 42.0).abs() < f32::EPSILON);
        assert_eq!(params.tint_color, [0.1, 0.2, 0.3, 1.0]);

        cfg.enabled = false;
        cfg.transparent.opacity = 0.5;
        let style = glass_style_from(&cfg);
        let GlassStyle::Transparent(t) = style else {
            panic!("enabled=false 必须映射为透明模式");
        };
        assert!((t.opacity - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn resize_options_map_feedback_and_grips() {
        let mut cfg = GlassResizeConfig::default();
        cfg.grip_px = 16.0;
        cfg.feedback.border_width = 8.0;
        let opts = resize_options_from(&cfg);
        assert!((opts.grip_px - 16.0).abs() < f32::EPSILON);
        assert!((opts.feedback.border_width - 8.0).abs() < f32::EPSILON);
        assert!(opts.enabled && opts.edges && opts.corners);
    }

    #[test]
    fn approx_rect_dedup_small_jitter() {
        let a = [10.0, 20.0, 300.0, 400.0];
        let b = [10.005, 20.0, 300.0, 400.0];
        assert!(approx_eq_rect(&a, &b));
        let c = [11.0, 20.0, 300.0, 400.0];
        assert!(!approx_eq_rect(&a, &c));
    }
}
