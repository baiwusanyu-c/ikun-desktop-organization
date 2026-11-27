//! 显示器对账：拔屏等比重定位 / 重连自动搬回（2026-11-27）。
//!
//! # 为什么需要独立模块
//! 拔掉副屏（或关闭副屏电源）的瞬间，停在副屏上的分类框滞留在已消失的坐标区域：
//! 不可见、不可操控。系统**不会**给这类窗口发位置消息，既有的
//! `WM_WINDOWPOSCHANGED` 自愈吸附不会被触发；`monitor_utils` 的矩形缓存又是
//! thread_local（玻璃引擎线程失效 ≠ 窗口线程失效）。因此在 `WM_DISPLAYCHANGE` /
//! 玻璃引擎 `MonitorsChanged` 广播后，由**分类框自己的窗口线程**防抖对账：
//!
//! 1. 框滞留（不完整落在任何在岗屏）→ 按归属屏记录的屏内比例，等比映射到
//!    离消失屏最近的在岗屏（允许互相重叠），并标记「待搬回」；
//! 2. 归属屏回归 → 按屏内比例自动搬回原屏原相对位置
//!    （用户滞留期间手动拖过框 → 会话收尾已重新收养归属，自动取消搬回）；
//! 3. 一切合法 → 只刷新归属记账（与现状相等则不落盘，空转零成本）。
//!
//! # 线程与借用纪律
//! - 所有几何动作只发生在窗口自己的消息线程（历史前科：跨线程 SetWindowPos
//!   同步等待平台线程，卡死 WebView2 事件循环）；
//! - 遵守 grid_window 的别名 UB 纪律：先抄标量、结束 `get_data` 借用，
//!   再调 SetWindowPos / 重绘，之后重新借用写回。

use super::drag_handler;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::glass_engine;
use super::grid_layout::relayout_for_dpi;
use super::grid_window::{
    force_full_content_refresh, get_data, get_grid_content_rect, shadow_padding_for_dpi,
    update_dcomp_scale,
};
use super::monitor_utils;
use super::zorder;
use crate::box_core::monitor_geom::{monitor_index_for_center, BoundsI, MonitorI, RectI};
use crate::box_core::monitor_remap::{
    self, current_origin, from_relative, live_index_for_origin, to_relative, MonitorOrigin,
    RelocatePlan,
};

/// 防抖：拔屏/插屏广播后等拓扑落定再做对账（热插拔风暴合并成一次）。
const RECONCILE_DEBOUNCE_MS: u32 = 500;
/// 复查：热插拔瞬间 `EnumDisplayMonitors` 可能仍返回旧屏集，追加一拍确认。
const RECONCILE_RETRY_MS: u32 = 1500;
/// 单次事件最多执行的拍数（防极端抖动下无限重试；新的显示事件会重新计数）。
const RECONCILE_MAX_PASSES: u8 = 2;

/// 安排一次防抖对账（`WM_DISPLAYCHANGE` 与 `WM_USER_DISPLAY_RECONCILE` 共用入口）。
///
/// 重复调度自动合并（KillTimer + SetTimer）；新事件把拍数计数清零，
/// 让「防抖首轮 + 复查轮」重新开始。
pub(crate) fn schedule_reconcile(hwnd: HWND) {
    log::info!("收到显示器变化广播，安排 500ms 防抖对账（滞留重定位 / 重连搬回）");
    if let Some(data) = unsafe { get_data(hwnd) } {
        data.reconcile_pass = 0;
    }
    unsafe {
        KillTimer(hwnd, TIMER_ID_MONITOR_RECONCILE);
        let timer = SetTimer(
            hwnd,
            TIMER_ID_MONITOR_RECONCILE,
            RECONCILE_DEBOUNCE_MS,
            std::ptr::null_mut(),
        );
        if timer == 0 {
            log::warn!("显示器对账：防抖定时器创建失败，等待下一次显示事件");
        }
    }
}

/// 一轮对账的结果。
enum ReconcileOutcome {
    /// 本轮已给出确定结论；仍保留一次复查拍，防止首次枚举拿到旧屏集。
    Settled,
    /// 本轮有搬动、屏集暂空或无法给出安全落点，需要复查。
    Retry,
    /// 用户拖拽/缩放会话进行中：不计入复查拍，等会话结束再继续。
    Busy,
    /// 窗口数据已失效，没有继续对账的意义。
    Abort,
}

/// 重新武装对账定时器。调用方负责先 KillTimer 或明确允许覆盖。
fn arm_reconcile_timer(hwnd: HWND, delay_ms: u32) {
    let timer = unsafe {
        SetTimer(
            hwnd,
            TIMER_ID_MONITOR_RECONCILE,
            delay_ms,
            std::ptr::null_mut(),
        )
    };
    if timer == 0 {
        log::warn!("显示器对账：复查定时器创建失败，本轮停止重试");
    }
}

/// `WM_TIMER` tick（窗口线程）：执行一轮对账，必要时追加 1.5s 复查拍。
///
/// 热插拔时 `EnumDisplayMonitors` 可能仍返回旧屏集，而旧屏集会让首轮误判
/// 「框仍在岗」并提前收敛。因此每轮显示事件至少执行两次非 busy 对账拍：
/// 首轮 + 1.5s 确认拍；屏集暂空也在这两拍内收敛，不会留下无限定时器。
pub(crate) fn on_reconcile_timer(hwnd: HWND) {
    unsafe { KillTimer(hwnd, TIMER_ID_MONITOR_RECONCILE) };
    // 缓存失效必须在**窗口线程**做：monitor_utils 是 thread_local 缓存，
    // 玻璃引擎线程的 invalidate 覆盖不到这里。
    monitor_utils::invalidate_monitor_rects_cache();

    let pass = match unsafe { get_data(hwnd) } {
        Some(data) => data.reconcile_pass,
        None => return,
    };
    let outcome = reconcile_once(hwnd);
    // 拍数推进/停表判定下沉为纯函数（next_timer_action），可单测收敛语义。
    let (next_pass, rearm) = next_timer_action(pass, &outcome);
    if let Some(data) = unsafe { get_data(hwnd) } {
        data.reconcile_pass = next_pass;
    }
    if rearm {
        arm_reconcile_timer(hwnd, RECONCILE_RETRY_MS);
    }
}

/// 对账定时器拍状态机（纯函数，可单测）：由「当前拍数 + 本轮结果」推导
/// （新拍数, 是否追加复查拍）。
///
/// 语义：
/// - `Abort`（数据失效）：清零，不再武装；
/// - `Busy`（用户在拖框/缩放）：**不消耗拍数**继续武装——拖拽可能持续数秒，
///   会话结束后的下一拍仍必须能完成搬动；
/// - `Settled | Retry`：计一拍；未达上限（2 拍）则武装 1.5s 确认拍
///   （热插拔首轮枚举可能返回旧屏集），达到上限清零停拍，绝不无限重试。
fn next_timer_action(pass: u8, outcome: &ReconcileOutcome) -> (u8, bool) {
    match outcome {
        ReconcileOutcome::Abort => (0, false),
        ReconcileOutcome::Busy => (pass, true),
        ReconcileOutcome::Settled | ReconcileOutcome::Retry => {
            let next = pass.saturating_add(1);
            if next < RECONCILE_MAX_PASSES {
                (next, true)
            } else {
                (0, false)
            }
        }
    }
}

/// 执行一轮对账。
///
/// 返回结果供定时器决定：`Busy` 不消耗拍数并继续等会话收尾；`Settled` / `Retry`
/// 在拍数上限内追加 1.5s 确认拍，达到上限后清计数，不会无限重试。
fn reconcile_once(hwnd: HWND) -> ReconcileOutcome {
    let Some(content) = get_grid_content_rect(hwnd) else {
        return ReconcileOutcome::Abort;
    };
    let (origin, relocated, config_id, busy) = match unsafe { get_data(hwnd) } {
        Some(data) => (
            data.monitor_origin.clone(),
            data.monitor_relocated,
            data.config_id,
            // 玻璃会话 / 图标拖拽进行中不得抢几何（块矩形与系统模态循环是真值），
            // 让位给会话收尾链路（收尾会走 adopt_current_monitor）。回退模式的
            // 系统模态移动/缩放在下面通过 IN_DRAG 单独检查。
            data.glass_session_active || data.drag_active || data.drag_pending,
        ),
        None => return ReconcileOutcome::Abort,
    };
    let live = monitor_utils::get_live_monitors();
    let bounds = monitor_utils::get_virtual_desktop_bounds();
    if live.is_empty() {
        log::warn!("显示器对账：在岗屏集合为空（热插拔枚举暂不稳定），稍后复查");
        return ReconcileOutcome::Retry;
    }
    if busy || zorder::is_drag_in_progress() || super::wnd_proc::user_move_resize_in_progress() {
        log::debug!("显示器对账：拖拽/缩放会话进行中，让位收尾链路，稍后复查");
        return ReconcileOutcome::Busy;
    }

    let box_rect: RectI = [content.left, content.top, content.right, content.bottom];
    // 尺寸口径（PRD 定案）：位置按屏内比例映射，尺寸保持逻辑口径——
    // 目标屏物理尺寸 = 当前物理尺寸 x（目标屏 DPI / 当前窗口 DPI），与手工
    // 跨屏拖拽的 PerMonitorV2 换算同构（搬动后 WM_DPICHANGED 也会按同一
    // 规则校正，二者不会叠加）。纯函数层自带该换算与玻璃下限 / 装不下时的
    // 长宽比等比缩小兜底，这里只需提供窗口当前 DPI。
    let cur_dpi = {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi > 0 {
            dpi
        } else {
            96
        }
    };
    let plan =
        monitor_remap::plan_for_box(box_rect, origin.as_ref(), relocated, &live, bounds, cur_dpi);
    match plan {
        RelocatePlan::Skip => {
            log::debug!("显示器对账：暂无法给出安全落点，稍后复查");
            ReconcileOutcome::Retry
        }
        RelocatePlan::Settled {
            origin: new_origin,
            relocated: still_relocated,
        } => {
            // 借用纪律：改完记账先结束借用，再调 save_config（其内部会读全部窗口数据）
            let changed = if let Some(data) = unsafe { get_data(hwnd) } {
                let changed = data.monitor_origin.as_ref() != Some(&new_origin)
                    || data.monitor_relocated != still_relocated;
                if changed {
                    data.monitor_origin = Some(new_origin);
                    data.monitor_relocated = still_relocated;
                }
                changed
            } else {
                return ReconcileOutcome::Abort;
            };
            if changed {
                drag_handler::save_config();
            }
            ReconcileOutcome::Settled
        }
        RelocatePlan::ReturnHome {
            rect,
            origin: new_origin,
            ..
        } => {
            let Some(moved) = apply_relocation(hwnd, rect) else {
                log::warn!("显示器对账：搬回原屏失败，保留待搬回状态并复查 config_id={config_id}");
                return ReconcileOutcome::Retry;
            };
            if let Some(data) = unsafe { get_data(hwnd) } {
                data.monitor_origin = Some(new_origin);
                data.monitor_relocated = false;
            } else {
                return ReconcileOutcome::Abort;
            }
            log::info!(
                "显示器对账：原屏已回归，分类框自动搬回原相对位置 config_id={config_id} moved={moved}"
            );
            drag_handler::save_config();
            ReconcileOutcome::Retry
        }
        RelocatePlan::Relocate { target, rect } => {
            let home_is_live = origin
                .as_ref()
                .is_some_and(|origin| live_index_for_origin(origin, &live).is_some());
            let Some(moved) = apply_relocation(hwnd, rect) else {
                log::warn!("显示器对账：搬离消失屏失败，保留原状态并复查 config_id={config_id}");
                return ReconcileOutcome::Retry;
            };
            let actual_origin = if home_is_live {
                get_grid_content_rect(hwnd).and_then(|actual| {
                    current_origin(
                        [actual.left, actual.top, actual.right, actual.bottom],
                        &live,
                    )
                })
            } else {
                None
            };
            if let Some(data) = unsafe { get_data(hwnd) } {
                if home_is_live {
                    // 归属屏仍在岗，只是框跨屏/越界被吸附回来：收养实际落位，
                    // 不进入「等待原屏重连」状态。
                    if let Some(actual_origin) = actual_origin {
                        data.monitor_origin = Some(actual_origin);
                    }
                    data.monitor_relocated = false;
                } else {
                    // 归属身份保持不变（仍指向已拔掉的屏），重连时对账自动搬回
                    data.monitor_relocated = true;
                }
            } else {
                return ReconcileOutcome::Abort;
            }
            if home_is_live {
                log::info!(
                    "显示器对账：分类框越界但归属屏在岗，已吸附到最近在岗屏（序号 {target}）config_id={config_id} moved={moved}"
                );
            } else {
                if moved {
                    log::info!(
                        "显示器对账：分类框滞留于已拔除的屏幕，已等比重定位到最近的在岗屏（序号 {target}）config_id={config_id}（原屏重连后将自动搬回）"
                    );
                } else {
                    // 系统救援已把它放在目标附近（±2px）：只补记账，不重复搬窗。
                    log::info!("显示器对账：分类框已在在岗屏合法落位（系统救援），标记待原屏重连后搬回 config_id={config_id}");
                }
            }
            drag_handler::save_config();
            ReconcileOutcome::Retry
        }
        RelocatePlan::Snap { rect, origin } => {
            let Some(moved) = apply_relocation(hwnd, rect) else {
                log::warn!("显示器对账：历史配置吸附失败，保留原状态并复查 config_id={config_id}");
                return ReconcileOutcome::Retry;
            };
            if let Some(data) = unsafe { get_data(hwnd) } {
                data.monitor_origin = Some(origin);
                data.monitor_relocated = false;
            } else {
                return ReconcileOutcome::Abort;
            }
            log::info!(
                "显示器对账：分类框滞留屏外且无归属记录，已吸附回在岗屏 config_id={config_id} moved={moved}"
            );
            drag_handler::save_config();
            ReconcileOutcome::Retry
        }
    }
}

/// 判断两个坐标是否在容差内。
fn within_tolerance(a: i32, b: i32, tolerance: i32) -> bool {
    (i64::from(a) - i64::from(b)).abs() <= i64::from(tolerance.max(0))
}
/// 把分类框内容矩形移动到 `target`（虚拟桌面物理像素）。
///
/// # 尺寸换算归属（2026-11-27 二次复测定案）
/// 跨 DPI 搬移的尺寸换算**只允许做一次**，且必须由本函数做：`target` 矩形携带
/// 纯逻辑层按「逻辑口径保持」换算好的目标物理尺寸（plan_for_box 内部
/// target_physical_size）。同一次 SetWindowPos 系统会同步发 WM_DPICHANGED，
/// 既有 handler 若再按建议矩形重缩 = 双重换算（复测「拔屏后变小 / 搬回又减半」
/// 的根因）；因此搬窗前必须给窗口挂一个**抑制窗口**（`dpi_suppress_*`），
/// WM_DPICHANGED 在窗口内只更新 DPI 口径与重绘，不改几何。
///
/// 系统自己「救援搬移」触发的 DPI 消息不在抑制窗口内（时间上先于对账 tick），
/// 照旧走既有换算路径，语义仍是逻辑口径保持，与本函数一致。
///
/// 返回：
/// - `Some(true)`：已执行搬动；
/// - `Some(false)`：位置与尺寸均已在目标（±2px 且 DPI 一致），跳过；
/// - `None`：窗口状态读取失败或 SetWindowPos 失败，调用方不得推进状态机。
fn apply_relocation(hwnd: HWND, target: RectI) -> Option<bool> {
    let cur = get_grid_content_rect(hwnd)?;
    let content_w = target[2].saturating_sub(target[0]).max(1);
    let content_h = target[3].saturating_sub(target[1]).max(1);
    let cx = target[0].saturating_add(content_w / 2);
    let cy = target[1].saturating_add(content_h / 2);
    let dpi = monitor_utils::dpi_at_screen_point(cx, cy);
    let current_dpi = {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi > 0 {
            dpi
        } else {
            96
        }
    };
    let padding = shadow_padding_for_dpi(dpi);
    let target_content = RECT {
        left: target[0],
        top: target[1],
        right: target[2],
        bottom: target[3],
    };
    let available_w = monitor_utils::monitor_rect_for_content_center(&target_content)
        .map(|(_, _, width, _)| width.saturating_sub(padding * 2).max(1));

    // 借用段 1：按目标屏 DPI 重采样布局（保列数吸附，防「图标变乱」复发）。
    let (new_w, padding, config_id, locked) = {
        let data = (unsafe { get_data(hwnd) })?;
        data.render_padding = padding;
        let mut snapped_w = relayout_for_dpi(data, content_w, content_h, dpi);
        // 保列数只吸收取整噪声；吸附后越过目标屏宽度时按可用宽度重排，
        // 不把「整框单屏内」交给 WM_WINDOWPOSCHANGED 自愈（它只平移）。
        if let Some(max_w) = available_w {
            if snapped_w > max_w {
                snapped_w = max_w;
                super::grid_layout::relayout_for_size(data, snapped_w, content_h, dpi);
            }
        }
        (snapped_w, data.render_padding, data.config_id, data.locked)
    };

    let already_at_target = current_dpi == dpi
        && within_tolerance(cur.left, target[0], 2)
        && within_tolerance(cur.top, target[1], 2)
        && within_tolerance(cur.right.saturating_sub(cur.left), new_w, 2)
        && within_tolerance(cur.bottom.saturating_sub(cur.top), content_h, 2);
    if already_at_target {
        return Some(false);
    }

    // —— 以下在无 data 借用状态下做窗口动作 ——
    // 先挂 DPI 再换算抑制窗口，再搬窗：跨 DPI 的 SetWindowPos 会在返回前同步
    // 收到 WM_DPICHANGED，届时 handler 只刷口径、不改我们已经算好的尺寸。
    if current_dpi != dpi {
        if let Some(data) = unsafe { get_data(hwnd) } {
            data.dpi_suppress_until_ms = glass_engine::now_ms().saturating_add(1000);
            data.dpi_suppress_dpi = dpi;
        }
    }
    update_dcomp_scale(hwnd);
    let outer_left = target[0].saturating_sub(padding);
    let outer_top = target[1].saturating_sub(padding);
    let outer_w = new_w.saturating_add(padding * 2).max(1);
    let outer_h = content_h.saturating_add(padding * 2).max(1);
    let moved = unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            outer_left,
            outer_top,
            outer_w,
            outer_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
    if moved == 0 {
        // 搬动失败：撤销抑制窗口，别让后续正常 DPI 消息被误吞。
        if let Some(data) = unsafe { get_data(hwnd) } {
            data.dpi_suppress_until_ms = 0;
            data.dpi_suppress_dpi = 0;
        }
        log::warn!(
            "显示器对账：SetWindowPos 失败，无法把分类框搬到目标屏 config_id={config_id} target={target:?}"
        );
        return None;
    }

    // 搬完后按**实际落位**重推玻璃几何（落位矩形以窗口现状为准），
    // 「块 == 命中层 == 内容」三者必须恒等对齐。
    let dpi_now = {
        let d = unsafe { GetDpiForWindow(hwnd) };
        if d > 0 {
            d
        } else {
            dpi
        }
    };
    if glass_engine::is_active() && glass_engine::has_box(config_id) {
        if let Some(actual) = get_grid_content_rect(hwnd) {
            glass_engine::push_box_rect(config_id, &actual, dpi_now);
            glass_engine::update_block_geom(
                config_id,
                glass_engine::BlockGeom {
                    win_w: actual.right.saturating_sub(actual.left),
                    title_h: data_title_h(hwnd, scale_for_dpi(40, dpi_now)),
                    dpi: dpi_now,
                    locked,
                },
            );
        }
    }
    // 全量重绘：清内容缓存指纹，让新 DPI 的纹理重新上传（跨屏漂移同款处理）。
    force_full_content_refresh(hwnd);
    Some(true)
}

/// 消费一次「对账搬动的 DPI 再换算抑制」。
///
/// `WM_DPICHANGED` handler 调用：仅当本消息的 new_dpi 与抑制目标一致且未过期时
/// 返回 true（并即时清除窗口），handler 据此跳过系统建议矩形的二次缩放。
/// 同一次搬移只会同步收到一条 DPI 消息，清除后恢复正常处理逻辑。
pub(crate) fn consume_dpi_suppress(hwnd: HWND, new_dpi: u32) -> bool {
    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return false;
    };
    if data.dpi_suppress_until_ms == 0 || data.dpi_suppress_dpi != new_dpi {
        return false;
    }
    let expired = glass_engine::now_ms() > data.dpi_suppress_until_ms;
    data.dpi_suppress_until_ms = 0;
    data.dpi_suppress_dpi = 0;
    !expired
}

/// 重读最新的标题栏高度（WM_DPICHANGED 可能已重算），读取失败时用调用方给的兜底值。
fn data_title_h(hwnd: HWND, fallback: i32) -> i32 {
    unsafe { get_data(hwnd) }
        .map(|data| data.title_h)
        .unwrap_or(fallback)
}

/// 收养「当前实际所在在岗屏」为归属屏，并清除待搬回标记。
///
/// 调用时机 = 用户驱动几何稳定的收尾（建窗完成、玻璃会话结束、`WM_EXITSIZEMOVE`）：
/// 用户把框放到哪块屏，归属就记哪块屏；即使此前因拔屏暂搬（relocated=true），
/// 手动挪动也代表接受当前落位，取消自动搬回。
///
/// 框当前不完整落在任何在岗屏（如仍处于滞留区）→ 不做收养，交给对账处理。
pub(crate) fn adopt_current_monitor(hwnd: HWND) {
    let Some(content) = get_grid_content_rect(hwnd) else {
        return;
    };
    let live = monitor_utils::get_live_monitors();
    if live.is_empty() {
        return;
    }
    let rect: RectI = [content.left, content.top, content.right, content.bottom];
    let Some(origin) = current_origin(rect, &live) else {
        return;
    };
    if let Some(data) = unsafe { get_data(hwnd) } {
        if data.monitor_origin.as_ref() == Some(&origin) && !data.monitor_relocated {
            return;
        }
        log::debug!(
            "归属屏记账更新: device={} rect={:?} relocated=false",
            origin.device,
            origin.rect
        );
        data.monitor_origin = Some(origin);
        data.monitor_relocated = false;
    }
}

/// 启动恢复的落位决策结果（物理像素 + 归属记账）。
pub(crate) struct RestoreGeometry {
    /// 目标内容矩形（虚拟桌面物理像素，已保证整框落在单块在岗屏内）
    pub content: RectI,
    /// 目标屏 DPI（供「物理 → 96 基准逻辑」换算，与 collect_grid_layout 同口径）
    pub dpi: u32,
    /// 写入窗口与后续落盘的归属记账（relocated=true 表示仍待重连搬回）
    pub origin: crate::config::BoxMonitorOrigin,
}

/// 启动恢复时按持久化归属身份换算落位（`restore_layout` 调用）。
///
/// - 归属屏在岗 → 按比例直接落位该屏；若配置带 `relocated=true`，这一步天然
///   就是「拔屏期间重启、副屏已接回 → 自动搬回」；
/// - 归属屏离岗（拔着屏重启）→ 等比映射到离消失屏最近的在岗屏，hint 保留
///   原屏身份与 `relocated=true`，运行期对账在接回时继续自动搬回；
/// - 历史配置（无 monitor）或在岗屏枚举暂空 → 返回 None，调用方走旧坐标 +
///   建窗吸附兜底。
pub(crate) fn plan_restore(box_data: &crate::config::BoxLayoutData) -> Option<RestoreGeometry> {
    let m = box_data.monitor.as_ref()?;
    let live = monitor_utils::get_live_monitors();
    if live.is_empty() {
        return None;
    }
    let origin = MonitorOrigin::from(m);
    let monitors: Vec<MonitorI> = live
        .iter()
        .map(|entry| {
            (
                (entry.bounds.0, entry.bounds.1),
                (entry.bounds.2, entry.bounds.3),
            )
        })
        .collect();

    let (target_idx, relocated_out) = if let Some(idx) = live_index_for_origin(&origin, &live) {
        (idx, false)
    } else {
        // 归属屏离岗：等比映射到「离消失屏最近」的在岗屏，保留原身份等待搬回
        let home_rect: RectI = [
            m.rect[0],
            m.rect[1],
            m.rect[0].saturating_add(m.rect[2]),
            m.rect[1].saturating_add(m.rect[3]),
        ];
        let idx = monitor_index_for_center(home_rect, &monitors)?;
        (idx, true)
    };
    let bounds: BoundsI = live[target_idx].bounds;
    let dpi = live[target_idx].dpi;
    // 持久化尺寸本就是 96 基准逻辑像素（collect_grid_layout 同口径），
    // 直接换算成目标屏物理像素；下限与「装不下等比缩」由 from_relative 兜底。
    let size = [
        scale_for_dpi(box_data.size[0].max(1.0) as i32, dpi),
        scale_for_dpi(box_data.size[1].max(1.0) as i32, dpi),
    ];
    let content = from_relative(m.rel, size, bounds, dpi);
    let origin_out = if relocated_out {
        // 保留「离岗屏」身份（device/rect 不变），运行期对账靠它识别回归；
        // 归属屏不在岗却成功映射 = 事实上的「暂搬」，必须置 relocated=true，
        // 否则重连事件到达时没有「待搬回」语义，框会永远留在别的屏上。
        crate::config::BoxMonitorOrigin {
            relocated: true,
            ..m.clone()
        }
    } else {
        crate::config::BoxMonitorOrigin {
            device: live[target_idx].device.clone(),
            rect: [bounds.0, bounds.1, bounds.2, bounds.3],
            rel: to_relative(content, bounds),
            relocated: false,
        }
    };
    Some(RestoreGeometry {
        content,
        dpi,
        origin: origin_out,
    })
}

#[cfg(test)]
mod tests {
    use super::{next_timer_action, ReconcileOutcome, RECONCILE_MAX_PASSES};
    use crate::box_core::monitor_remap::{min_physical, GLASS_MIN_LOGICAL_H, GLASS_MIN_LOGICAL_W};
    use crate::platform::windows::glass_engine::glass_resize_floor;

    /// 拍状态机：每个显示事件最多收敛 2 拍（首轮 + 1.5s 确认），绝不无限重试。
    #[test]
    fn reconcile_timer_converges_within_max_passes() {
        // 首轮无论 Retry 还是 Settled 都武装一次确认拍
        assert_eq!(
            next_timer_action(0, &ReconcileOutcome::Retry),
            (1, true),
            "首轮有动作必须武装确认拍"
        );
        assert_eq!(
            next_timer_action(0, &ReconcileOutcome::Settled),
            (1, true),
            "首拍即便看似收敛也要确认（枚举可能仍是旧屏集）"
        );
        // 第二拍达到上限：清零停表，不再武装
        assert_eq!(next_timer_action(1, &ReconcileOutcome::Settled), (0, false));
        assert_eq!(next_timer_action(1, &ReconcileOutcome::Retry), (0, false));
        assert!(RECONCILE_MAX_PASSES >= 2, "上限至少两轮是收敛前提");
    }

    /// Busy（用户拖框中）不消耗拍数且持续武装；Abort 清零且停表。
    #[test]
    fn busy_does_not_consume_pass_budget() {
        assert_eq!(next_timer_action(1, &ReconcileOutcome::Busy), (1, true));
        assert_eq!(next_timer_action(0, &ReconcileOutcome::Abort), (0, false));
    }

    /// box_core 的玻璃逻辑下限必须与 `glass_engine::glass_resize_floor` 同源。
    ///
    /// 重定位下限与库 resize 硬下限是同一个约定（11-26 任务 R1：常量逻辑值
    /// 200x140，不按 DPI 二次换算）；两边常量若悄悄漂移，会出现
    /// 「对账把框放到下限、库下一帧又按自己的下限再抬一次」的拉锯。
    #[test]
    fn glass_min_constants_track_resize_floor() {
        let (w, h) = glass_resize_floor(96);
        assert_eq!(
            GLASS_MIN_LOGICAL_W,
            w.round() as i32,
            "box_core 宽下限与库 resize floor 不一致"
        );
        assert_eq!(
            GLASS_MIN_LOGICAL_H,
            h.round() as i32,
            "box_core 高下限与库 resize floor 不一致"
        );
        assert_eq!(min_physical(96), (GLASS_MIN_LOGICAL_W, GLASS_MIN_LOGICAL_H));
    }
}
