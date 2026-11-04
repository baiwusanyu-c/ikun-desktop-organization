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
//!
//! # 玻璃块层序（0.5.2 起由库保证）
//! 每块玻璃的缩放反馈 border 在**该块自己的 pass 内**发射：紧跟它自己的玻璃、
//! 早于任何更高块（库 `core::draw::border_emit_plan`）。因此两个框重叠时，下层框
//! 的反馈白边会被上层框的玻璃正确遮挡，不再画到上层框身上（0.5.1 及以前 border
//! 是「独立叠加 pass、盖在全部玻璃之上」）。宿主不再为 border 抬层——0.5.1 时代
//! 的 `raise_box` 绕法实测修不了这条缺陷；块间层序只由「激活/操作谁在最前」这条
//! 产品语义决定（见 `wnd_proc` 的 Z 序不变量跟进 `raise_box`）。
//!
//! 0.5.3 起**命中的层序与绘制的层序共用同一套 z 判据**（库 BASELINE 不变量 13）：`hit_resize`
//! 会丢弃「被更上层块本体压住」的把手候选，所以上层框机身下的那条下层框边缘**根本不亮**，
//! 也不再变缩放手型；外来窗口压在光标上时库用 `WindowFromPoint` 门控（只在光标移动过的帧查，
//! 故灭边/恢复都要动一下鼠标）。宿主不得重算这套判据，也不要再用抬层或 `set_resizable(false)` 绕。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;

use uuid::Uuid;

use liquid_verre::{
    CaptureBackend, CaptureConfig, DiagnosticsConfig, Engine, EngineConfig, EngineEvent,
    GlassInputSink, GlassStyle, InputDecision, LiquidParams, LogLevel, PointerButton, PointerEvent,
    PointerPhase, Profile as VerreProfile, ResizeFeedbackOptions, ResizeOptions,
    Shape as VerreShape, TransparentParams, WindowConfig, WindowLevel,
};

use crate::config::app_config::{
    AppConfig, GlassCaptureBackend, GlassEngineConfig, GlassLiquidParams, GlassProfile,
    GlassResizeConfig, GlassShape, LiquidGlassConfig,
};

use super::ffi_imports::{GetWindowRect, PostMessageW};
use super::ffi_types::{
    HTCAPTION, MK_LBUTTON, MK_RBUTTON, RECT, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCLBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_USER_GLASS_FOCUS,
};
use super::grid_layout::{lock_btn_rect, more_btn_rect};
use super::monitor_utils::get_primary_monitor_dpi;
use std::sync::Arc;

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

/// 库渲染线程 panic 嫌疑标志：panic hook 只在「栈地址落在 liquid-verre 源码
/// 里」时置位（见 `main.rs::install_panic_hook`），runtime tick 消费。
///
/// 为什么需要它：库线程 panic 会跳过 `set_running(false)` 那段正常退出代码，
/// `Engine::is_running()` 永远停在 `true`，宿主只看这个标志就永远不会复活
/// （2026-10-27 实测：库 `QualityCtrl` 计数溢出 panic 后，日志持续输出
/// `玻璃引擎[运行中] fps=0`，分类框玻璃与全部块输入死亡）。
static ENGINE_SUSPECT: AtomicBool = AtomicBool::new(false);
/// 上一次 tick 看到的快照 epoch 与连续停滞拍数（仅 tick 线程读写）。
static LAST_SNAPSHOT_EPOCH: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_STALL_TICKS: AtomicU64 = AtomicU64::new(0);
/// 最近一次自动复活的单调时钟毫秒（限流，防止复活风暴）。
static LAST_REVIVE_MS: AtomicU64 = AtomicU64::new(0);

/// panic hook 入口：标记「渲染线程可能已被这次 panic 打死」。
///
/// 这里**绝不允许**调用任何引擎 API：hook 跑在正在死亡的那个线程上，
/// 它可能正持着库内部写锁，跨线程调用会直接死锁。
pub fn mark_engine_suspect() {
    ENGINE_SUSPECT.store(true, Ordering::SeqCst);
}

/// 快照推进是否停滞（连续 >=2 拍 epoch 不变）。
///
/// 空闲桌面本身就可能长时间不发布快照，所以本函数只在配合
/// [`mark_engine_suspect`] 时才有意义，不单独作为死亡判据。
fn snapshot_stalled() -> bool {
    let epoch = with_holder(|holder| holder.engine.snapshot().epoch).unwrap_or(0);
    let prev = LAST_SNAPSHOT_EPOCH.swap(epoch, Ordering::Relaxed);
    let ticks = if epoch == prev {
        SNAPSHOT_STALL_TICKS.fetch_add(1, Ordering::Relaxed) + 1
    } else {
        SNAPSHOT_STALL_TICKS.store(0, Ordering::Relaxed);
        0
    };
    ticks >= 2
}

/// 进程内单调时钟基准（自动复活限流用）。
static CLOCK_BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// 进程内单调毫秒（供内容层节流等跨模块小工具复用，避免各算各的基准）。
#[must_use]
pub(crate) fn now_ms() -> u64 {
    let base = CLOCK_BASE.get_or_init(std::time::Instant::now);
    u64::try_from(base.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// 距上次自动复活是否已过限流窗口（10 s）；从未复活过则直接放行。
///
/// 只**读**时间戳：判定与记账分离，否则每拍探测都会刷新时间戳，
/// 引擎真死的时候反而永远过不了限流门。
fn revive_allowed() -> bool {
    let prev = LAST_REVIVE_MS.load(Ordering::Relaxed);
    prev == 0 || now_ms().saturating_sub(prev) >= 10_000
}

/// 记一次实际发生的复活（仅在全量重建真的下发时调用）。
fn mark_revive() {
    LAST_REVIVE_MS.store(now_ms(), Ordering::Relaxed);
}

/// 命中判定用的块几何快照（渲染线程只读，宿主线程在几何/状态变化时整体替换）。
#[derive(Clone, Copy)]
pub struct BlockGeom {
    /// 块内容宽（物理像素）
    pub win_w: i32,
    /// 标题栏高（物理像素）
    pub title_h: i32,
    /// 窗口 DPI
    pub dpi: u32,
    /// 锁定（固定）状态：锁定时标题不发起拖拽
    pub locked: bool,
}

/// 单个块的路由状态（渲染线程读写；锁持有期间只做拷贝与 PostMessage）。
struct RouteState {
    /// 代理窗口句柄（isize 形式的 HWND）。
    agent: isize,
    /// 几何快照（None = 尚未发布，标题拖拽判定退化为客户端消息）。
    geom: Option<BlockGeom>,
    /// 最近一次 Move 的块内物理坐标（cursor_override 查询 hover 用）。
    last_local: [f32; 2],
    /// 左键是否在块内被宿主消费（Down 返回 Consumed 置位、Up 清位）。
    /// 按压期间的 Move 必须转发给代理窗口，否则框选矩形/图标拖拽只能靠
    /// 100ms hover 轮询更新坐标（实测 ~10Hz，用户报「框选非常卡顿」）。
    host_pressing: bool,
}

// 库玻璃 id → 路由状态的独立登记表（输入 sink 与事件回调专用）。
//
// 回调在渲染线程触发，且发生在库内部 `handler` 读锁持有期间；若回调再去锁
// `HOLDER`，会与「主线程持 `HOLDER` 等 `handler` 写锁」构成三方锁序环
// （实测启动即死锁）。因此回调路径只允许碰这张极轻量的表：表操作
// microsecond 级，且持本表锁时绝不调用任何引擎 API。
lazy_static::lazy_static! {
    static ref ROUTES: Mutex<HashMap<u32, RouteState>> = Mutex::new(HashMap::new());
}

fn registry_register(glass_id: u32, agent: isize) {
    if let Ok(mut map) = ROUTES.lock() {
        map.insert(
            glass_id,
            RouteState {
                agent,
                geom: None,
                last_local: [0.0; 2],
                host_pressing: false,
            },
        );
    }
}

fn registry_unregister(glass_id: u32) {
    if let Ok(mut map) = ROUTES.lock() {
        map.remove(&glass_id);
    }
}

fn registry_clear() {
    if let Ok(mut map) = ROUTES.lock() {
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

/// 库缩放会话最小尺寸在**宿主侧的硬上限**（单位 = 库逻辑像素，见 `phys_to_logical_rect`）。
///
/// 为什么需要上限：`box_style.liquid_glass.resize.min_width/min_height` 是样式页
/// 滑条能写到 4096 的原始值，用户配置里实测出现过 605x1150（正好等于示例玻璃框
/// 当时的物理尺寸）。库把它当硬下限（`input::drag::ResizeState::apply` 里的
/// `max(min_w)`），于是**每个分类框都被钉死在 605x1150，边缘拖拽完全改不小**
/// （2026-11-03 用户报「拖到一定程度就无法改变大小」；日志里 `缩放结束` 的宽全部
/// 停在 605/732 这类 >=605 的值、高恒为 1150，即为实锤）。
///
/// 上限取 `WM_GETMINMAXINFO` 已经在用的最小跟踪尺寸同一口径（宽
/// `scale_for_dpi(200)`，高 `title_h + scale_for_dpi(100)`，都不含外投影留白），
/// 保证全项目只有一个「最小分类框」定义，玻璃块与代理窗口不会各按各的。
#[must_use]
pub(crate) fn glass_resize_floor(dpi: u32) -> (f32, f32) {
    let dpi = if dpi > 0 { dpi } else { 96 };
    let ratio = monitor_dpi_ratio(dpi).max(1e-3);
    let title_h = super::ffi_types::scale_for_dpi(40, dpi) as f32;
    let min_w = super::ffi_types::scale_for_dpi(200, dpi) as f32 / ratio;
    let min_h = (title_h + super::ffi_types::scale_for_dpi(100, dpi) as f32) / ratio;
    // 24 = 库 `core::block::MIN_SIZE`，再小库自己也会夹回去。
    (min_w.max(24.0), min_h.max(24.0))
}

/// 分类框状态 → 下发给库的块参数（`attach_box` 与引擎重建共用一份口径）。
///
/// # 为什么锁定不能关 `interactive`（2026-10-27 分类框全灭根因）
/// 引擎架构下分类框窗口是**全透明代理窗**（画面由库合成），鼠标命中链是
/// 「代理窗透明像素 → 库画布 → sink → 代理窗」。库把 `interactive=false`
/// 实现成「块不参与点击穿透判定」（见库 `runtime/mod.rs` 的 inside 计算），
/// 于是画布永远保持 `WS_EX_TRANSPARENT`：整块区域一个鼠标消息都收不到，
/// 锁定/更多按钮点不动、标题双击无反应、hover 不动、拖不动，**连解锁都
/// 无法完成**——用户侧表现就是「整个分类框完全不可用」。
///
/// 锁定的正确语义 = 只关库自己的缩放/移动会话（`resize.enabled=false`），
/// 输入路由必须恒开；标题拖拽是否放行由 sink 的 `BlockGeom::locked` 与
/// 内容层 `WM_LBUTTONDOWN` 共同决定（两处都已有判定）。
#[must_use]
fn block_config_for(
    style_cfg: &LiquidGlassConfig,
    logical: [f32; 4],
    locked: bool,
    dpi: u32,
) -> WindowConfig {
    let mut resize = resize_options_from(&style_cfg.resize);
    // 最小尺寸只能「托底」不能「钉死」：超过宿主上限的部分一律压回
    // （见 `glass_resize_floor`），否则样式页里一个误拉的滑条就让全部框改不了大小。
    let (floor_w, floor_h) = glass_resize_floor(dpi);
    if resize.min_width > floor_w || resize.min_height > floor_h {
        log::warn!(
            "玻璃缩放最小尺寸被夹取: ({},{}) -> ({floor_w:.0},{floor_h:.0}) dpi={dpi}（配置值大于最小跟踪尺寸）",
            resize.min_width,
            resize.min_height,
        );
        resize.min_width = resize.min_width.min(floor_w);
        resize.min_height = resize.min_height.min(floor_h);
    }
    if locked {
        resize.enabled = false;
    }
    WindowConfig {
        rect: logical,
        style: Some(glass_style_from(style_cfg)),
        resize,
        z_order: None,
        // 恒为 true：见函数注释。
        interactive: true,
        // 移动会话交给库驱动（未锁定时）。库 README 口径：不需要改拖拽位置的块
        // 返回 `DragMove` 用库的 movable；库会话是「鼠标消息只记 pending、每帧
        // 应用一次」并把 capture_wait_ms 压到 0（循环全速），所以块几何逐帧更新
        // （BASELINE：单块拖拽 113~116fps / p50 8.34ms）。
        // 宿主自己接管（sink 内 set_rect）实测只有 ~45Hz：库不在会话里时按
        // IDLE_WAIT_MS 步长跑，Move 事件与宿主 tick 一起被拖慢（2026-10-28 计数实锤）。
        movable: !locked,
        visible: true,
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
        // 0.5.2 新增的渲染队列深度（1..=8，None = 1 跟手优先）。本项目以「光标到
        // 光子」延迟为验收口径，不暴露到风格面板，也不设 GLASS_FRAME_LATENCY
        // 环境变量（库侧优先级：环境变量 > 本字段 > 默认 1）。
        frame_latency: None,
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
    engine.set_event_handler(Some(Arc::new(|event| match event {
        EngineEvent::ResizeStarted { id, .. } | EngineEvent::MoveStarted { id, .. } => {
            dispatch_glass_session(*id, true);
        }
        EngineEvent::ResizeEnded { id, .. } | EngineEvent::MoveEnded { id, .. } => {
            dispatch_glass_session(*id, false);
        }
        _ => {}
    })));
    engine.start()?;
    Ok(engine)
}

/// 画布主循环唤醒保险丝（**只唤醒，不接管**；库 0.5.1 已内建同构定时器，
/// 本函数按库侧指南建议保留为双保险）。
///
/// 机制：块被遮挡/全屏让路导致采集停产后，库主循环失去帧时钟、阻塞在
/// `GetMessage`；「光标进块 → 解除点击穿透」的判定恰在该循环内执行 —— 于是
/// 画布永久 `WS_EX_TRANSPARENT`，全部块输入死亡（移开遮挡窗口也无法自愈）。
/// 本函数向每块画布 `PostMessage(WM_NULL)`：异步、非阻塞、无跨线程等待，
/// 仅让库循环空转一圈，由其自身逻辑正确决定穿透状态与遮挡复检。
/// 库侧根治 = 画布常驻低频 SetTimer 兜底（见桌面缺陷清单 #4）。
pub fn wake_canvas_idle() {
    const WM_NULL: u32 = 0x0000;
    for canvas in canvas_hwnds() {
        unsafe {
            super::ffi_imports::PostMessageW(canvas as *mut std::ffi::c_void, WM_NULL, 0, 0);
        }
    }
}

/// 引擎渲染线程已死时立即重建（runtime tick 每周期调用，存活时零成本）。
///
/// 两条死亡通道：
/// 1. 正常退出（显示器热插拔触发库 Shutdown）：`is_running()` 变 false；
///    0.5.1 已内建「类已存在」容忍，重建不再需要注销窗口类。
/// 2. 库线程 panic：`set_running(false)` 在 panic  unwind 中被跳过，
///    `is_running()` 永远停在 true，只能靠 panic hook 的嫌疑标志 +
///    快照 epoch 停滞来识别（见 [`mark_engine_suspect`]）。
///
/// 限流 10 s，避免异常状态下反复重建。复活后旧画布窗口属于已死线程，
/// 宿主无法跨线程销毁（`DestroyWindow` 限制），可能残留一层静止玻璃，
/// 需重启进程才能彻底清掉——这是「分类框全灭」与「留一层残影」的取舍。
pub fn revive_if_dead(app: &AppConfig) {
    let suspect = ENGINE_SUSPECT.swap(false, Ordering::SeqCst);
    let dead = match HOLDER.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(holder) => !holder.engine.is_running(),
            None => return,
        },
        Err(_) => return,
    };
    let panic_killed_thread = suspect && snapshot_stalled();
    if !dead && !panic_killed_thread {
        return;
    }
    if !revive_allowed() {
        log::warn!("玻璃引擎需要重建，但距上次复活不足 10 s，本轮跳过");
        return;
    }
    mark_revive();
    if panic_killed_thread {
        log::error!(
            "玻璃引擎渲染线程疑似被库内 panic 打死（is_running 仍为 true、快照已停滞），强制重建"
        );
    } else {
        log::warn!("检测到玻璃引擎渲染线程已退出，立即重建");
    }
    let (style_cfg, engine_cfg, boxes) = {
        let mut guard = match HOLDER.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match guard.as_mut() {
            Some(holder) if dead || panic_killed_thread => {
                let out = (
                    holder.style_cfg,
                    holder.engine_cfg,
                    collect_boxes_for_rebuild(holder),
                );
                *guard = None;
                out
            }
            _ => return,
        }
    };
    registry_clear();
    if !spawn_holder_and_reattach(app, style_cfg, engine_cfg, boxes) {
        log::error!("玻璃引擎自动复活失败，分类框回退旧渲染路径");
    }
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
    drop(guard);
    // 抢先压底：画布此刻还是**隐藏**窗口，但已经占在「普通带顶部」的 Z 序位次上。
    // 0.5.2 起库显示画布走「层级 + SWP_SHOWWINDOW 同一次 SetWindowPos」
    // （platform/window.rs::show_request），首次显示不再把画布抬进普通带；
    // 这里压一次 + runtime tick 判据兜底足够（4ms 收敛守护已退役）。
    press_canvas_bottom();
    true
}

/// 全部画布的 HWND（isize 形式），供宿主在画布上注册 OLE 拖放转发器等。
#[must_use]
pub fn canvas_hwnds() -> Vec<isize> {
    with_holder(|holder| {
        holder
            .engine
            .monitors()
            .iter()
            .map(|m| m.canvas_handle)
            .filter(|h| *h != 0)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default()
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
/// `locked` 为当前固定状态：锁定时库的缩放把手关闭，块仍可命中（输入走 sink 路由）。
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
    let config = block_config_for(&holder.style_cfg, logical, locked, dpi);
    let created = match holder.engine.create_window(config) {
        Ok(window) => {
            registry_register(window.id(), hwnd);
            // 输入路由 sink：块内指针事件合成标准鼠标消息转投代理窗口，
            // 复用分类框全部既有消息驱动交互（见 route_pointer）。
            window.set_input_sink(Some(Arc::new(HostSink)));
            // 焦点路由：块被按压消费时库发 FocusHint，宿主侧对代理窗口 SetFocus。
            window.set_focus_hwnd(hwnd);
            let info = format!(
                "attach box={box_id} 内容物理=({},{})-({},{}) ratio={ratio:.3} dpi={dpi} logical=[{}, {}, {}, {}]",
                content_phys.left,
                content_phys.top,
                content_phys.right,
                content_phys.bottom,
                logical[0],
                logical[1],
                logical[2],
                logical[3]
            );
            log::info!("{}", info.replace(",  ", ", "));
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
    };
    drop(guard);
    if created {
        // 锁外执行：rebind → canvas_hwnds 会再锁 HOLDER（std Mutex 同线程重入即死锁）。
        super::drop_target::rebind_canvas_drop_targets();
    }
    created
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
    let phys = with_holder(|holder| {
        let entry = holder.boxes.get(&box_id)?;
        let snapshot = entry.window.snapshot()?;
        Some((
            logical_to_phys_rect(&snapshot.rect, entry.ratio),
            [snapshot.rect[2], snapshot.rect[3]],
            entry.ratio,
        ))
    })??;
    let (rect, _block_logical, _ratio) = phys;
    Some(rect)
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

/// 该玻璃块当前是否正处于库驱动的任意会话（移动或缩放）。
///
/// 宿主的本地标志位靠 PostMessage 送达，有几拍延迟；判断「能不能反推几何」
/// 必须问库要即时状态，否则会在会话刚起的窗口期里把块拽偏。
#[must_use]
pub fn block_in_session(box_id: Uuid) -> bool {
    with_holder(|holder| {
        holder
            .boxes
            .get(&box_id)
            .and_then(|e| e.window.snapshot())
            .is_some_and(|s| s.moving || s.resizing)
    })
    .unwrap_or(false)
}

/// 该玻璃块当前是否正处于库驱动的**移动**会话。
#[must_use]
pub fn block_moving(box_id: Uuid) -> bool {
    with_holder(|holder| {
        holder
            .boxes
            .get(&box_id)
            .and_then(|e| e.window.snapshot())
            .is_some_and(|s| s.moving)
    })
    .unwrap_or(false)
}

/// 同步固定（锁定）状态：锁定时关掉库的移动/缩放把手与会话，
/// 解锁时按当前配置恢复；**输入路由（interactive）恒开**，
/// 否则全透明代理窗会失去全部鼠标消息（见 [`block_config_for`]）。
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
    // 双保险：即使历史路径把块设成了非交互，这里也把它拉回来。
    entry.window.set_interactive(true);
    // 锁定 = 关掉库自己的移动/缩放会话（输入路由仍然在线，见 block_config_for）
    entry.window.set_movable(!locked);
    entry.window.set_resizable(!locked);
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
    let mut resize = resize_options_from(&glass.resize);
    // 与 block_config_for 同口径夹取：否则样式切换会把原始配置的 605x1150
    // 直接推给每个块 set_resize_options，框又被钉死（2026-11-03 用户复验）。
    holder.engine.set_default_style(style);
    let primary = get_primary_monitor_dpi();
    for entry in holder.boxes.values_mut() {
        let dpi = (entry.ratio * primary as f32).round() as u32;
        let (fw, fh) = glass_resize_floor(dpi);
        let mut r = resize;
        if r.min_width > fw || r.min_height > fh {
            r.min_width = r.min_width.min(fw);
            r.min_height = r.min_height.min(fh);
        }
        entry.window.set_style(Some(style));
        if !entry.locked {
            entry.window.set_resize_options(r);
        }
    }
    // default_resize 也要夹（新建块的兜底）。
    let primary_dpi = primary;
    let (dfw, dfh) = glass_resize_floor(primary_dpi);
    if resize.min_width > dfw || resize.min_height > dfh {
        resize.min_width = resize.min_width.min(dfw);
        resize.min_height = resize.min_height.min(dfh);
    }
    holder.engine.set_default_resize(resize);
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
        // 按代理窗口当前 DPI 重算比例：热插拔后主屏 DPI 可能已变，
        // 沿用旧 ratio 会让块尺寸成倍膨胀（200% ↔ 100% 时正好 2x）。
        let dpi_now = unsafe { super::ffi_imports::GetDpiForWindow(hwnd as *mut std::ffi::c_void) };
        let ratio = if dpi_now > 0 {
            monitor_dpi_ratio(dpi_now)
        } else {
            ratio
        };
        // 旧 logical 是按旧 ratio 折算的尺寸，按新 ratio 重新折算物理尺寸。
        let old_ratio = with_holder(|holder| holder.boxes.get(&id).map(|e| e.ratio))
            .flatten()
            .unwrap_or(ratio);
        let logical = if (old_ratio - ratio).abs() > 1e-3 {
            [
                logical[0],
                logical[1],
                logical[2] * old_ratio / ratio,
                logical[3] * old_ratio / ratio,
            ]
        } else {
            logical
        };
        let window_config = block_config_for(&style_cfg, logical, locked, dpi_now);
        match holder.engine.create_window(window_config) {
            Ok(window) => {
                registry_register(window.id(), hwnd);
                window.set_input_sink(Some(Arc::new(HostSink)));
                window.set_focus_hwnd(hwnd);
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
    } else {
        log::error!("玻璃引擎重建收尾失败：HOLDER 锁不可用");
        return false;
    }
    // 重建后必须把新画布重新压回 Z 序最底部：旧引擎销毁→新引擎就绪之间，
    // 面板等应用窗口可能被这块全屏画布压在下面，DWM 遮挡状态错乱，
    // WebView2 判定窗口被遮挡后停止绘制新暴露区域（2026-11-02 实锤：WGC↔DDA
    // 切换重建后控制面板整页空白、滚动条失灵，EnumWindows 显示画布压在面板上）。
    press_canvas_bottom();
    // 画布压在别的窗口之上期间，OLE 放置目标也要跟着重挂（同 attach_box 口径）。
    super::drop_target::rebind_canvas_drop_targets();
    true
}

// =============================================================================
// Z 序与事件
// =============================================================================

/// 把库会话（移动 / 缩放）的起止转发到对应分类框窗口的消息循环。
///
/// 只在渲染线程回调里执行：查表 + 非阻塞 PostMessage，不重入引擎、不加锁等待。
fn dispatch_glass_session(glass_id: u32, started: bool) {
    let hwnd = match ROUTES.lock() {
        Ok(map) => map.get(&glass_id).map(|route| route.agent),
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

/// 桌面类窗口（画布必须正好压在它们之上，除此之外的任何窗口都不该比画布低）。
fn is_desktop_class(hwnd: isize) -> bool {
    let mut buf = [0u16; 64];
    let n = unsafe {
        super::ffi_imports::GetClassNameW(
            hwnd as *mut std::ffi::c_void,
            buf.as_mut_ptr(),
            buf.len() as i32,
        )
    };
    if n <= 0 {
        return false;
    }
    let class = String::from_utf16_lossy(&buf[..n as usize]);
    class == "Progman" || class == "WorkerW"
}

/// 画布当前是否已经在「除桌面外最低」的位置。
///
/// 判据只沿 `GW_HWNDNEXT`（更低的窗口）走最多 8 步：跳过本进程自己的窗口
/// （两块画布、代理窗、tooltip/IME 伴生窗），第一个外部窗口必须是桌面
/// （Progman/WorkerW）或者干脆没有窗口，才算「已经在底」。
///
/// 这么做的理由是**成本**：下发一次层级要等库渲染线程处理一次位置变更
/// （实测 ~10ms），所以稳定态必须零成本：只有画布**真的**被抬进普通带才下发，
/// 并带 120ms 防抖（避免与同样把自己压到底的第三方窗口互相推搡）。
fn canvas_at_bottom(canvas: isize) -> bool {
    const MAX_HOPS: usize = 8;
    let own = unsafe { super::ffi_imports::GetCurrentProcessId() };
    let mut cur = canvas;
    for _ in 0..MAX_HOPS {
        let next = unsafe {
            super::ffi_imports::GetWindow(
                cur as *mut std::ffi::c_void,
                super::ffi_types::GW_HWNDNEXT,
            )
        } as isize;
        if next == 0 {
            return true; // 已经是 Z 序最底
        }
        let mut pid = 0u32;
        unsafe {
            super::ffi_imports::GetWindowThreadProcessId(next as *mut std::ffi::c_void, &mut pid)
        };
        if pid != own && !is_desktop_class(next) {
            return false; // 画布下面还压着别的应用窗口
        }
        cur = next;
    }
    true
}

/// 画布位次判据：`(画布总数, 被抬进普通带的画布)`。稳定态只走 O(1) 的 GetWindow。
fn canvas_bottom_state() -> (usize, Vec<isize>) {
    let canvases = canvas_hwnds();
    let stuck: Vec<isize> = canvases
        .iter()
        .copied()
        .filter(|h| !canvas_at_bottom(*h))
        .collect();
    (canvases.len(), stuck)
}

/// 取证用：把当前能拿到的画布句柄打成十六进制串。
fn canvases_hex() -> String {
    canvas_hwnds()
        .iter()
        .map(|h| format!("{h:#X}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// 走库 API 把全部画布压回 Z 序底部（0.5.2 起等值调用也会重发层级）。
///
/// 0.5.1 时代这条调用是**空操作**（库 `Command::SetWindowLevel` 有等值早退，
/// 而 `EngineConfig.window_level` 建引擎时已是 `AlwaysOnBottom`），宿主只能自己
/// 拿 `canvas_handle` 发 `SetWindowPos(HWND_BOTTOM)`。0.5.2 去掉早退后
/// （#3：等值也重发一次 SetWindowPos，但不发事件、不重建金字塔），
/// 层级下发统一交回库，宿主侧那条裸 SetWindowPos 路径整体退役。
fn apply_canvas_bottom_level() {
    if let Ok(guard) = HOLDER.lock() {
        if let Some(holder) = guard.as_ref() {
            if holder.engine.is_running() {
                holder.engine.set_window_level(WindowLevel::AlwaysOnBottom);
            }
        }
    }
}

/// 判据驱动的压底：只有画布真的被抬起来才下发层级，带 120ms 防抖。
///
/// 返回 `(画布总数, 被抬起并已修正的画布数)`，供日志与复验脚本读取。
fn reassert_canvas_bottom_once() -> (usize, usize) {
    static NEXT_PUSH_MS: AtomicU64 = AtomicU64::new(0);
    static NEXT_LOG_MS: AtomicU64 = AtomicU64::new(0);
    let (count, stuck) = canvas_bottom_state();
    if stuck.is_empty() {
        return (count, 0);
    }
    let now = now_ms();
    if now < NEXT_PUSH_MS.load(Ordering::Relaxed) {
        return (count, stuck.len());
    }
    NEXT_PUSH_MS.store(now + 120, Ordering::Relaxed);
    apply_canvas_bottom_level();
    if now >= NEXT_LOG_MS.load(Ordering::Relaxed) {
        NEXT_LOG_MS.store(now + 1_000, Ordering::Relaxed);
        log::info!(
            "画布层级修正：{} 块画布被抬进了普通带，已请库重发底部层级（hwnd={}）",
            stuck.len(),
            canvases_hex()
        );
    }
    (count, stuck.len())
}

/// 把各屏画布压回 Z 序底部（对外入口：runtime tick 与「内容窗口被推底后」
/// 的跟进点用它——判据 + 防抖，稳定态零成本）。
pub fn reassert_canvas_bottom() {
    reassert_canvas_bottom_once();
}

/// 关键时机（引擎启动 / 重建、`Ready`、`MonitorsChanged`、新建块显示）无条件
/// 请库压一次底部，再补一轮判据日志。
///
/// 0.5.1 时代这里还挂着 4ms「压底收敛守护」线程盯库的首次 `ShowWindow`（它会把
/// 画布抬进普通带顶部，实测 t+0.66s 压在 Chrome 上）。0.5.2 把显示与层级合进
/// 同一次 `SetWindowPos`（#2 `show_request`）后那条守护整体退役，留下的兜底是：
/// 这几个关键时机 + runtime 100ms tick 的判据复查。
pub fn press_canvas_bottom() {
    apply_canvas_bottom_level();
    reassert_canvas_bottom_once();
}

/// 轮询引擎事件队列并转入项目日志系统（runtime tick 调用）。
///
/// 全部使用低频路径日志：这些事件只在启动、显示器变化、采集失效等场景出现，
/// 不属于每帧热路径。
pub fn poll_events_to_log() {
    let (events, alive) = match HOLDER.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(holder) => (holder.engine.poll_events(), true),
            None => (Vec::new(), false),
        },
        Err(_) => (Vec::new(), false),
    };
    if alive {
        // 画布可能在 Ready/显示器变化后重建；先试注册（幂等去重，未就绪静默跳过）。
        super::drop_target::rebind_canvas_drop_targets();
    }
    for event in events {
        match event {
            EngineEvent::Ready { monitor_count } => {
                log::info!("玻璃引擎就绪，参与渲染的显示器数量: {monitor_count}");
                // 画布是「创建时置底、首帧就绪才显示」。0.5.2 起库把显示与层级
                // 合进同一次 SetWindowPos，不再从底部弹起来；这里仍无条件压一次
                // 兜底（等值调用现在也会重发层级），避免全屏画布压在面板等应用
                // 窗口之上（2026-11-02 控制面板整页空白的根因，见
                // spawn_holder_and_reattach 注释）。
                press_canvas_bottom();
            }
            EngineEvent::MonitorsChanged { monitor_count } => {
                log::info!("显示器集合变化，画布与采集已重建，显示器数量: {monitor_count}");
                press_canvas_bottom();
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
                // 该事件在画布真正开始出帧后到达，是「画布已显示」的最晚可靠信号；
                // 再压一次底部兜底（幂等，SetWindowPos 只动 Z 序）。
                reassert_canvas_bottom();
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
            EngineEvent::FocusHint { id, hwnd } => {
                log::debug!("玻璃块请求焦点: id={id}");
                // 焦点必须同线程设置：转发给代理窗口，由它在自己消息循环里 SetFocus 自己。
                if hwnd != 0 {
                    unsafe {
                        PostMessageW(hwnd as *mut std::ffi::c_void, WM_USER_GLASS_FOCUS, 0, 0);
                    }
                }
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

/// 诊断快照：每秒一条，打印每个玻璃块的库侧真值状态。
///
/// 排查「某个框点不动 / 另一框正常」这类问题的唯一硬证据：库侧
/// `interactive`/`visible`/`occluded`/`hovered`/`rect` 与宿主窗口矩形是否一致。
/// debug 级（默认 info 过滤下不输出），零副作用，只读快照。
pub fn debug_log_blocks() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let now = now_ms();
    if now < NEXT.load(Ordering::Relaxed) {
        return;
    }
    NEXT.store(now + 1_000, Ordering::Relaxed);
    let lines = with_holder(|holder| {
        let mut out = Vec::new();
        for (id, entry) in holder.boxes.iter() {
            match entry.window.snapshot() {
                Some(s) => out.push(format!(
                    "box={id} glass_id={} rect=[{:.0}, {:.0}, {:.0}, {:.0}] scale={:.2} mon={} interactive={} visible={} occluded={} hovered={} moving={} resizing={} resize_enabled={} movable={}",
                    s.id, s.rect[0], s.rect[1], s.rect[2], s.rect[3], s.scale, s.monitor_index,
                    s.interactive, s.visible, s.occluded, s.hovered, s.moving, s.resizing,
                    s.resize.enabled, s.movable
                )),
                None => out.push(format!("box={id} glass_id={} snapshot=None", entry.window.id())),
            }
        }
        out
    });
    if let Some(lines) = lines {
        for line in lines {
            log::debug!("[glass-dbg] {line}");
        }
    }
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

// =============================================================================
// 库窗口即本体：输入路由（sink → 合成消息 → 代理窗口）与内容推流
// =============================================================================

/// lParam 打包（x 低 16 位、y 高 16 位，Win32 标准形态）。
#[must_use]
fn pack_lparam(x: i32, y: i32) -> isize {
    (((y << 16) | (x & 0xFFFF)) as i32) as isize
}

/// 块内坐标 → 屏幕坐标（代理窗口矩形与块矩形恒等对齐，engine 模式 padding=0）。
fn agent_screen_pos(agent: isize, lx: i32, ly: i32) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetWindowRect(agent as *mut std::ffi::c_void, &mut rect) } != 0 {
        (rect.left + lx, rect.top + ly)
    } else {
        (lx, ly)
    }
}

/// 向代理窗口投递「双击」消息。
///
/// 标题区走 `WM_NCLBUTTONDBLCLK(HTCAPTION)`（编辑分类框，纯自绘处理，
/// 无系统模态循环参与，posted 安全）；主体区走 `WM_LBUTTONDBLCLK`（打开文件）。
unsafe fn post_double_click(agent: isize, in_title: bool, lx: i32, ly: i32) {
    let hwnd = agent as *mut std::ffi::c_void;
    if in_title {
        let (sx, sy) = agent_screen_pos(agent, lx, ly);
        PostMessageW(
            hwnd,
            WM_NCLBUTTONDBLCLK,
            HTCAPTION as usize,
            pack_lparam(sx, sy),
        );
    } else {
        PostMessageW(hwnd, WM_LBUTTONDBLCLK, 0, pack_lparam(lx, ly));
    }
}

/// 按压是否路由为「标题拖拽」：标题栏区域（按钮除外）且未锁定。
///
/// 命中判定复用内容层同一套按钮矩形纯函数（`lock_btn_rect`/`more_btn_rect`），
/// 保证 sink 决策与 `WM_LBUTTONDOWN` 处理器对同一坐标给出一致答案。
#[must_use]
fn down_routes_title_drag(geom: &BlockGeom, x: i32, y: i32) -> bool {
    if geom.locked || y >= geom.title_h {
        return false;
    }
    let (bx, by, bs) = lock_btn_rect(geom.win_w, geom.title_h, geom.dpi);
    if x >= bx && x < bx + bs && y >= by && y < by + bs {
        return false;
    }
    let (mx, my, ms) = more_btn_rect(geom.win_w, geom.title_h, geom.dpi);
    if x >= mx && x < mx + ms && y >= my && y < my + ms {
        return false;
    }
    // 上边缘把手带内不接管（库的 resize 会话优先级更高，能走到这里说明不在把手带）。
    true
}

/// 标题区命中锁定/更多按钮（cursor_override 手型判定用）。
#[must_use]
fn in_title_buttons(geom: &BlockGeom, x: i32, y: i32) -> bool {
    let (bx, by, bs) = lock_btn_rect(geom.win_w, geom.title_h, geom.dpi);
    if x >= bx && x < bx + bs && y >= by && y < by + bs {
        return true;
    }
    let (mx, my, ms) = more_btn_rect(geom.win_w, geom.title_h, geom.dpi);
    x >= mx && x < mx + ms && y >= my && y < my + ms
}

/// 块内光标判定（纯函数，单测钉死）：标题按钮手型，其余箭头。
#[must_use]
fn cursor_id_for(geom: &BlockGeom, x: i32, y: i32) -> isize {
    if in_title_buttons(geom, x, y) {
        32649 // IDC_HAND
    } else {
        32512 // IDC_ARROW
    }
}

/// 全局输入路由 sink：把库画布收到的块内指针事件合成为标准鼠标消息，
/// PostMessage 到对应分类框代理窗口，复用既有消息驱动交互全链路。
///
/// # 消息映射
/// - 左键 Down/Up → `WM_LBUTTONDOWN/WM_LBUTTONUP`（块内坐标即 client 坐标）。
///   标题区拖拽**不**合成 `WM_NCLBUTTONDOWN`：posted down 与系统模态移动循环
///   存在竞态（up 早于模态启动即丢 → 主线程卡死），由代理窗口在 `WM_LBUTTONDOWN`
///   里自管「接管式移动会话」（`grid_window::begin_box_drag`：SetCapture +
///   定时器轮询光标 → SetWindowPos → WM_WINDOWPOSCHANGED 反向同步块矩形）。
/// - `WM_MOUSEMOVE` **空闲不转发、按压必转发**：posted 移动不会被系统按队列
///   合并（真实输入才会），120Hz 洪水会压垮宿主消息循环（实测按钮失灵、hover
///   闪烁），所以空闲 hover 由低频轮询定时器驱动（`poll_glass_hover`）；
///   但左键按压期间（框选/拖拽阈值）不转发就没有实时坐标源，必须逐条投递。
/// - 双击/右键/滚轮 → 对应标准消息（低频，直接转发）。
struct HostSink;

impl GlassInputSink for HostSink {
    fn on_pointer(&self, id: u32, ev: &PointerEvent) -> InputDecision {
        route_pointer(id, ev)
    }

    fn cursor_override(&self, id: u32) -> Option<isize> {
        // 锁定/更多按钮 hover 手型；块内其余位置**必须显式回箭头**。
        //
        // 返回 None 时库画布走 `DefWindowProcW`，而画布窗口类注册时
        // hCursor 为空（liquid-verre-rs `platform/window.rs` 的 WNDCLASSEXW
        // 用 `..Default::default()`）→ DefWindowProc 不设置光标 → 上一阶段的
        // 光标形状原地粘住：边缘缩放后块内仍是 SIZEWE/SIZENS、点过锁定按钮后
        // 块内仍是 HAND（2026-11-02 用户报「分类框内鼠标不对」，GetCursorInfo
        // 实测钉死：拖完右边缘进块内光标停在 SIZEWE）。
        // 把手带光标由库自己按 `hover_handle` 优先处理，不会被这里覆盖。
        let Ok(map) = ROUTES.lock() else {
            return None;
        };
        let route = map.get(&id)?;
        let geom = route.geom?;
        let x = route.last_local[0] as i32;
        let y = route.last_local[1] as i32;
        Some(cursor_id_for(&geom, x, y))
    }
}

fn route_pointer(id: u32, ev: &PointerEvent) -> InputDecision {
    // 锁内只做状态更新与取快照；锁外 PostMessage（非阻塞、任意线程合法）。
    let (agent, title_h, hand_off_move, pressing) = {
        let Ok(mut map) = ROUTES.lock() else {
            return InputDecision::Ignored;
        };
        let Some(route) = map.get_mut(&id) else {
            return InputDecision::Ignored;
        };
        route.last_local = ev.block_local;
        let title_h = route.geom.map(|g| g.title_h);
        let lx = ev.block_local[0] as i32;
        let ly = ev.block_local[1] as i32;
        // 标题空白按压（未锁定、不在按钮上）→ 让位给库的 movable 会话，由库逐帧
        // 应用光标位移（README：不需要改拖拽位置的块返回 DragMove）。宿主自己
        // 接管实测只有 ~45Hz，原因见 block_config_for 注释。
        let hand_off_move = matches!(ev.phase, PointerPhase::Down)
            && route
                .geom
                .is_some_and(|g| down_routes_title_drag(&g, lx, ly));
        // 按压消费状态机：Down（宿主消费，未让位给库会话）→ 开始转发 Move；
        // Up → 停止。库会话（DragMove/缩放）期间不置位，由库自行逐帧驱动。
        match ev.phase {
            PointerPhase::Down => {
                if !hand_off_move {
                    route.host_pressing = true;
                }
            }
            PointerPhase::Up(button) if button != Some(PointerButton::Right) => {
                route.host_pressing = false;
            }
            PointerPhase::DoubleClick => {
                route.host_pressing = false;
            }
            _ => {}
        }
        let pressing = route.host_pressing;
        (route.agent, title_h, hand_off_move, pressing)
    };
    if agent == 0 {
        return InputDecision::Ignored;
    }
    let lx = ev.block_local[0] as i32;
    let ly = ev.block_local[1] as i32;
    let hwnd = agent as *mut std::ffi::c_void;
    if matches!(ev.phase, PointerPhase::Move) {
        log::debug!("[glass-dbg] sink id={id} phase=Move local=({lx},{ly})");
    } else {
        log::info!(
            "[glass-dbg] sink id={id} phase={:?} local=({lx},{ly})",
            ev.phase
        );
    }

    match ev.phase {
        // Move：空闲态只更新 last_local（供 cursor_override 手型判定），不转发
        // ——posted 移动不会被系统按队列合并，120Hz 洪水会压垮宿主消息循环
        // （hover 高亮/tooltip 由低频轮询驱动）。但**左键按压期间必须转发**：
        // 框选矩形与图标拖拽阈值都靠 WM_MOUSEMOVE 更新坐标，不转发时只剩
        // 100ms hover 轮询喂坐标（实测框选 ~10Hz，用户报「框选非常卡顿」）。
        // 按压是有限时长事件流，转发成本可控（框选处理器只记坐标不重绘）。
        PointerPhase::Move => {
            if pressing {
                // 自愈守卫：按压标志靠 Up 清除，但「在其它窗口上松手」时画布
                // 收不到 WM_LBUTTONUP（无捕获），标志会一直挂着 → 空闲 Move 被
                // 持续转发 = 当初要避免的消息洪水。按物理按键状态兜底清位。
                let lbutton_down =
                    unsafe { super::ffi_imports::GetAsyncKeyState(super::ffi_types::VK_LBUTTON) }
                        & 0x8000u16 as i16
                        != 0;
                if lbutton_down {
                    unsafe { PostMessageW(hwnd, WM_MOUSEMOVE, MK_LBUTTON, pack_lparam(lx, ly)) };
                } else {
                    if let Ok(mut map) = ROUTES.lock() {
                        if let Some(route) = map.get_mut(&id) {
                            route.host_pressing = false;
                        }
                    }
                }
            }
            InputDecision::Consumed
        }
        PointerPhase::Down => {
            if hand_off_move {
                log::debug!("[glass-dbg] sink 让位给库移动会话 id={id} local=({lx},{ly})");
                return InputDecision::DragMove;
            }
            unsafe {
                PostMessageW(hwnd, WM_LBUTTONDOWN, MK_LBUTTON, pack_lparam(lx, ly));
            }
            InputDecision::Consumed
        }
        PointerPhase::Up(button) => match button {
            Some(PointerButton::Right) => {
                unsafe {
                    PostMessageW(hwnd, WM_RBUTTONUP, 0, pack_lparam(lx, ly));
                }
                InputDecision::Consumed
            }
            Some(PointerButton::Left) | None => {
                unsafe {
                    PostMessageW(hwnd, WM_LBUTTONUP, 0, pack_lparam(lx, ly));
                }
                InputDecision::Consumed
            }
        },
        // 0.5.2 起库画布类带 CS_DBLCLKS，物理双击走这条真相位：Windows 到的是
        // Down → Up → DoubleClick → Up（第二次按下**不再**产生 Down）。
        PointerPhase::DoubleClick => {
            unsafe { post_double_click(agent, title_h.is_some_and(|th| ly < th), lx, ly) };
            InputDecision::Consumed
        }
        PointerPhase::RightDown => {
            unsafe {
                PostMessageW(hwnd, WM_RBUTTONDOWN, MK_RBUTTON, pack_lparam(lx, ly));
            }
            InputDecision::Consumed
        }
        PointerPhase::Wheel => {
            // 既有 handler 从 HIWORD(wparam) 取 delta；库已按 /120 归一，回填原始值。
            let delta = ev.wheel_delta.saturating_mul(120) as i16 as u16;
            unsafe {
                PostMessageW(
                    hwnd,
                    WM_MOUSEWHEEL,
                    (delta as usize) << 16,
                    pack_lparam(lx, ly),
                );
            }
            InputDecision::Consumed
        }
    }
}

/// 宿主发布块几何快照（创建/WM_SIZE/DPI/锁定切换后调用；渲染线程零锁竞争压力）。
pub fn update_block_geom(box_id: Uuid, geom: BlockGeom) {
    let glass_id = with_holder(|holder| holder.boxes.get(&box_id).map(|e| e.window.id())).flatten();
    let Some(glass_id) = glass_id else {
        return;
    };
    if let Ok(mut map) = ROUTES.lock() {
        if let Some(route) = map.get_mut(&glass_id) {
            route.geom = Some(geom);
        }
    }
}

/// 推送块内容位图（预乘 BGRA8、物理像素、行距 w*4；dirty=None 全量）。
///
/// 语义与校验由库承担：非法参数丢弃 + warn-once；w/h=0 等价 clear_content。
pub fn push_content(box_id: Uuid, w_phys: u32, h_phys: u32, buffer: &[u8]) {
    push_content_dirty(box_id, w_phys, h_phys, buffer, None);
}

/// 带脏区的增量推送（框选快路径专用）。
///
/// `dirty` 为块内物理像素半开区间 `[l, t, r, b)`（库 `update_content` 口径：
/// 纹理与块矩形 1:1 锚定左上角，dirty 直接就是纹理空间坐标）。库据此只重传
/// 子矩形 GPU 纹理——整块 ~4MB 全量重传是框选每拍成本大头（2026-11-03）。
pub fn push_content_dirty(
    box_id: Uuid,
    w_phys: u32,
    h_phys: u32,
    buffer: &[u8],
    dirty: Option<[u32; 4]>,
) {
    SESSION_CONTENT_PUSHES.fetch_add(1, Ordering::Relaxed);
    with_holder(|holder| {
        if let Some(entry) = holder.boxes.get(&box_id) {
            entry.window.update_content(w_phys, h_phys, buffer, dirty);
        }
    });
}

// =============================================================================
// 会话期帧率对比（零成本计数 + 1 s 一条 info 汇总）
//
// 为什么不用库的取证落盘 `GLASS_INTERNAL_SHOT`：它每帧要把整屏 RT 回读并写盘
// （实测 24.8MB/帧，跑一次就落 4.9GB），GPU 回读本身就会把主循环拖慢 ——
// 用它测掉帧等于制造掉帧。计数器只读写几个原子量。
// =============================================================================
/// 会话驱动次数（跟随拍数 / 拖拽 Move 数）
pub static SESSION_TICKS: AtomicU64 = AtomicU64::new(0);
/// 会话期「块矩形相对上一拍发生变化」的次数（= 内容真正需要跟上的次数）
pub static SESSION_RECT_CHANGES: AtomicU64 = AtomicU64::new(0);
/// 会话期内容纹理重推次数
pub static SESSION_CONTENT_PUSHES: AtomicU64 = AtomicU64::new(0);
/// 上一拍读到的块矩形（i64::MIN = 未初始化）
static SESSION_LAST_RECT: [AtomicI64; 4] = [
    AtomicI64::new(i64::MIN),
    AtomicI64::new(i64::MIN),
    AtomicI64::new(i64::MIN),
    AtomicI64::new(i64::MIN),
];
static SESSION_LAST_EPOCH: AtomicU64 = AtomicU64::new(0);
static SESSION_LOG_NEXT_MS: AtomicU64 = AtomicU64::new(0);

/// 读库侧帧计数：`(快照 epoch, fps)`；引擎不可用时 `(0, 0.0)`。
#[must_use]
pub fn engine_frame_stats() -> (u64, f32) {
    with_holder(|holder| {
        let s = holder.engine.snapshot();
        (s.epoch, s.fps)
    })
    .unwrap_or((0, 0.0))
}

/// 记一拍跟随：块矩形变了就计数。
pub fn note_session_rect(rect: [i32; 4]) {
    SESSION_TICKS.fetch_add(1, Ordering::Relaxed);
    let changed = SESSION_LAST_RECT[0].load(Ordering::Relaxed) != i64::from(rect[0])
        || SESSION_LAST_RECT[1].load(Ordering::Relaxed) != i64::from(rect[1])
        || SESSION_LAST_RECT[2].load(Ordering::Relaxed) != i64::from(rect[2])
        || SESSION_LAST_RECT[3].load(Ordering::Relaxed) != i64::from(rect[3]);
    if changed {
        SESSION_RECT_CHANGES.fetch_add(1, Ordering::Relaxed);
        for (i, slot) in SESSION_LAST_RECT.iter().enumerate() {
            slot.store(i64::from(rect[i]), Ordering::Relaxed);
        }
    }
}

/// 会话期 1 秒一条的帧率对比汇总（info 级，零成本，只在会话期出现）。
///
/// 四个数直接指出掉帧在哪一环：
/// `tick` 跟随拍数（8ms 档理论 ~125/s）、`块矩形变化` 几何真的动了多少次、
/// `内容重推` 内容纹理重画并推给库多少次、`库发布快照` 库侧 epoch 增量（≈渲染帧数）。
pub fn maybe_log_session_stats() {
    let now = now_ms();
    if now < SESSION_LOG_NEXT_MS.load(Ordering::Relaxed) {
        return;
    }
    SESSION_LOG_NEXT_MS.store(now + 1_000, Ordering::Relaxed);
    let ticks = SESSION_TICKS.swap(0, Ordering::Relaxed);
    let rect_changes = SESSION_RECT_CHANGES.swap(0, Ordering::Relaxed);
    let pushes = SESSION_CONTENT_PUSHES.swap(0, Ordering::Relaxed);
    if ticks == 0 {
        return;
    }
    let (epoch, fps) = engine_frame_stats();
    let prev_epoch = SESSION_LAST_EPOCH.swap(epoch, Ordering::Relaxed);
    let epochs = epoch.saturating_sub(prev_epoch);
    log::info!(
        "[glass-dbg] 会话帧率对比: tick={} 块矩形变化={} 内容重推={} 库发布快照={} fps={:.0}",
        ticks,
        rect_changes,
        pushes,
        epochs,
        fps
    );
}

/// 会话期催库出帧（将跟随拍数与库帧率对齐）。
///
/// 0.5.1 时代这里还要顺手调一次 `set_default_resize` 当「快照发布催办器」：
/// 那时库的移动/缩放会话改块几何**不置** `snapshot_dirty`，宿主读到的 `rect` 只有
/// 兜底档 ~4Hz，内容纹理跟着卡成 3~4Hz（用户看到「玻璃边缘在动、图标一卡一卡」）。
/// 0.5.2 起库的契约是**块几何变化 ⇒ 同帧快照可见**，催发布随之退役；`moving`/
/// `resizing` 也已接进快照比对键（#4 连带修正），会话收尾判据不会再卡在 true。
pub fn request_render() {
    if let Ok(guard) = HOLDER.lock() {
        if let Some(holder) = guard.as_ref() {
            if holder.engine.is_running() {
                holder.engine.request_render();
            }
        }
    }
}

/// 逻辑矩形近似相等。
///
/// 阈值 1.5px：块矩形经「物理→逻辑→物理」往返存在 ±1px 舍入，
/// 过严的去重会让 窗口→push→块→跟随→窗口 形成每 tick 拉锯
/// （实测表现为 hover 闪烁）。1.5px 内的差异直接不下发，切断抖动源。
fn approx_eq_rect(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1.5)
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
    fn locked_block_keeps_input_routing_and_disables_resize_handles() {
        // 回归钉（2026-10-27 分类框全灭）：锁定只能关库的缩放把手，绝不
        // 能把 interactive 关掉——代理窗是全透明的，画布一旦保持
        // WS_EX_TRANSPARENT，分类框就收不到任何鼠标消息（连解锁都点不动）。
        let cfg = LiquidGlassConfig::default();
        let logical = [0.0, 0.0, 400.0, 300.0];
        let unlocked = block_config_for(&cfg, logical, false, 96);
        let locked = block_config_for(&cfg, logical, true, 96);
        assert!(
            unlocked.interactive && locked.interactive,
            "输入路由必须恒开"
        );
        assert!(
            unlocked.movable && !locked.movable,
            "未锁定应交给库驱动移动（每帧应用一次，120Hz）；锁定则关掉库的移动会话"
        );
        assert!(
            unlocked.resize.enabled,
            "解锁态应保留库的边缘缩放（默认配置 enabled=true）"
        );
        assert!(!locked.resize.enabled, "锁定时必须关掉库的缩放把手");
        assert_eq!(locked.rect, logical);
    }

    #[test]
    fn pathological_resize_min_is_capped_so_boxes_can_shrink() {
        // 回归钉（2026-11-03）：样式页「最小宽度/最小高度」被拉到 605x1150 后，
        // 库会把它当硬下限，所有分类框都被钉死在示例框的尺寸上、边缘拖不小。
        let mut cfg = LiquidGlassConfig::default();
        cfg.resize.min_width = 605.0;
        cfg.resize.min_height = 1150.0;
        let logical = [100.0, 100.0, 605.0, 1150.0];

        let (floor_w, floor_h) = glass_resize_floor(192);
        let capped = block_config_for(&cfg, logical, false, 192);
        assert_eq!(
            capped.resize.min_width, floor_w,
            "超过上限的最小宽必须被夹取"
        );
        assert_eq!(
            capped.resize.min_height, floor_h,
            "超过上限的最小高必须被夹取"
        );
        assert!(
            floor_w < 605.0 && floor_h < 1150.0,
            "上限必须低于钉死用户的 605x1150，否则问题依旧：{floor_w}x{floor_h}"
        );

        // 正常配置（默认 24）必须原样下发，不能因为「有上限」而被抬高。
        let kept = block_config_for(&LiquidGlassConfig::default(), logical, false, 192);
        assert_eq!(kept.resize.min_width, 24.0);
        assert_eq!(kept.resize.min_height, 24.0);
    }

    #[test]
    fn resize_floor_is_monitor_physical_size_independent() {
        // 库逻辑像素 = 主屏物理像素，`glass_resize_floor` 折算后应当落在**同一物理宽度**：
        // `scale_for_dpi(200, dpi) / (dpi / primary_dpi)` 与 dpi 无关。高 DPI 副屏
        // 上的最小框必须和主屏一样大，否则跨屏拖一下尺寸下限就跳变。
        let (w96, h96) = glass_resize_floor(96);
        let (w192, h192) = glass_resize_floor(192);
        let (w144, h144) = glass_resize_floor(144);
        assert!(
            (w96 - w192).abs() < 1.0,
            "最小宽不该随 DPI 变化: {w96} vs {w192}"
        );
        assert!(
            (h96 - h144).abs() < 1.0,
            "最小高不该随 DPI 变化: {h96} vs {h144}"
        );
        assert!(w96 >= 24.0 && h96 >= 24.0, "不得低于库 MIN_SIZE=24");
        assert!(
            w96 <= 1024.0 && h96 <= 1024.0,
            "上限本身也得合理: {w96}x{h96}"
        );
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
    fn cursor_override_pins_arrow_inside_block_and_hand_on_buttons() {
        // 2026-11-02 回归钉：画布窗口类无 hCursor，override 返回 None 会让
        // 上一阶段的 SIZEWE/HAND 在块内粘住（用户报「分类框内鼠标不对」）。
        // 块内非按钮区域必须显式回箭头；锁定/更多按钮回手型。
        let geom = BlockGeom {
            win_w: 676,
            title_h: 80,
            dpi: 192,
            locked: false,
        };
        assert_eq!(cursor_id_for(&geom, 300, 400), 32512); // 内容区 → IDC_ARROW
        let (bx, by, bs) = lock_btn_rect(geom.win_w, geom.title_h, geom.dpi);
        assert_eq!(cursor_id_for(&geom, bx + bs / 2, by + bs / 2), 32649); // 锁定 → IDC_HAND
        let (mx, my, ms) = more_btn_rect(geom.win_w, geom.title_h, geom.dpi);
        assert_eq!(cursor_id_for(&geom, mx + ms / 2, my + ms / 2), 32649); // 更多 → IDC_HAND
        assert_eq!(cursor_id_for(&geom, 10, 40), 32512); // 标题空白 → IDC_ARROW
    }

    #[test]
    fn title_drag_route_matches_legacy_semantics() {
        let g = BlockGeom {
            win_w: 800,
            title_h: 40,
            dpi: 96,
            locked: false,
        };
        // 标题空白 → 拖拽接管；主体区域 → 客户端消息
        assert!(
            down_routes_title_drag(&g, 100, 20),
            "标题栏空白应接管为拖拽"
        );
        assert!(!down_routes_title_drag(&g, 100, 300), "主体区域不是拖拽");
        // 注：顶部/边缘 grip 带「恒优先于 sink」由库侧规则保证（库不会把把手带
        // 按压送进 sink），本函数只需回答「送进来的点是否属于标题拖拽区」。
        assert!(
            down_routes_title_drag(&g, 100, 0),
            "y=0 归 sink 时仍是标题区"
        );
        // 锁定/更多按钮区域必须让位给点击（与内容层既有热区一致）
        let (bx, by, bs) = lock_btn_rect(800, 40, 96);
        assert!(
            !down_routes_title_drag(&g, bx + 1, by + 1),
            "锁定按钮不触发拖拽"
        );
        let (mx, my, ms) = more_btn_rect(800, 40, 96);
        assert!(
            !down_routes_title_drag(&g, mx + 1, my + 1),
            "更多按钮不触发拖拽"
        );
        assert!(bs > 0 && ms > 0);
        // 锁定（固定）状态下标题不可拖拽 —— 与重构前 is_locked 全 HTCLIENT 一致
        let locked = BlockGeom { locked: true, ..g };
        assert!(
            !down_routes_title_drag(&locked, 100, 20),
            "锁定时不接管拖拽"
        );
    }

    #[test]
    fn pack_lparam_roundtrip() {
        let packed = pack_lparam(-92, 172) as i32;
        let x = (packed & 0xFFFF) as i16 as i32;
        let y = ((packed >> 16) & 0xFFFF) as i16 as i32;
        assert_eq!((x, y), (-92, 172));
    }

    #[test]
    fn approx_rect_dedup_small_jitter() {
        let a = [10.0, 20.0, 300.0, 400.0];
        let b = [10.005, 20.0, 300.0, 400.0];
        assert!(approx_eq_rect(&a, &b));
        // ±1px 舍入往返必须被吸收（空闲拉锯抖动源）
        assert!(approx_eq_rect(&a, &[11.0, 20.0, 300.0, 400.0]));
        // 真实移动（≥2px）必须照常下发
        assert!(!approx_eq_rect(&a, &[13.0, 20.0, 300.0, 400.0]));
    }
}
