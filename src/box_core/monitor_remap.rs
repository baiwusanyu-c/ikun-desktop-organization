//! 显示器等比重定位纯逻辑（拔屏 / 重连时对分类框做映射）。
//!
//! 平台无关、0 unsafe、可单测；口径与 [`super::monitor_geom`] 完全一致：
//! - 矩形 `RectI = [left, top, right, bottom]`、外接范围 `BoundsI = (ox, oy, w, h)`
//!   均为**虚拟桌面物理像素**（副屏可能在负坐标区）；
//! - `rel` 是**位置**的屏内无量纲比例 `[x, y]`（0.0..=1.0，相对整块屏矩形）。
//!
//! # 尺寸口径（与用户确认的定案，PRD 2026-11-27）
//! **位置按屏内比例等比映射，尺寸保持逻辑口径**：目标屏上的物理尺寸 =
//! 当前物理尺寸 x（目标屏 DPI / 当前窗口 DPI），即 96 基准逻辑尺寸不变——
//! 与手工把分类框跨屏拖拽时 PerMonitorV2 `WM_DPICHANGED` 的换算完全同构。
//! 仅当换算后仍装不下目标屏时，才按长宽比整体等比缩小（保持形状，不剪角）。
//! 因此持久化只需记录位置比例与归属屏身份，**尺寸由窗口现状 / `BoxLayoutData.size`
//! 现算**，不落第二份数据。
//!
//! # 为什么需要它
//! 拔掉副屏（或关掉副屏电源）的瞬间，分类框滞留在已消失的坐标区域：不可见、
//! 不可操控。Windows 不会给这类窗口发「位置变化」消息，既有的
//! `WM_WINDOWPOSCHANGED` / 落盘吸附都不会被触发。因此必须在 `WM_DISPLAYCHANGE`
//! 后主动对账：
//! 1. 滞留 → 按「消失屏记录的屏内比例」映射到最近的在岗屏（允许重叠）；
//! 2. 归属屏回归 → 按保存的比例自动搬回原屏原相对位置；
//! 3. 用户滞留期间手动拖过框（会话收尾重新收养归属屏）→ 取消自动搬回。

use super::monitor_geom::{
    monitor_index_for_center, rect_fits_any_monitor, snap_into_single_monitor, BoundsI, MonitorI,
    RectI,
};

/// 屏内相对**位置** `[rel_x, rel_y]`：框左上角相对归属屏完整矩形的比例
/// （除以屏宽/屏高），0.0..=1.0。尺寸不在这里——尺寸走逻辑口径（见模块头）。
pub type RelF = [f32; 2];

/// 玻璃缩放下限的 96 基准逻辑尺寸（宽），与 `glass_engine::glass_resize_floor`
/// 同源常量（11-26 任务 R1：库逻辑像素硬下限，不再按 DPI 二次换算）。
pub const GLASS_MIN_LOGICAL_W: i32 = 200;
/// 玻璃缩放下限的 96 基准逻辑尺寸（高 = 40 标题 + 100 内容）。
pub const GLASS_MIN_LOGICAL_H: i32 = 140;

/// 在岗显示器（含身份与 DPI），对账/恢复路径的输入表项。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveMonitor {
    /// Win32 设备名（`\\.\DISPLAYn`），空 = 未知
    pub device: String,
    /// 完整屏矩形（虚拟桌面物理像素）
    pub bounds: BoundsI,
    /// 该屏当前生效 DPI（effective，96 基准）
    pub dpi: u32,
}

/// 分类框的归属显示器身份：设备名 + 记录时该屏完整矩形 + 屏内位置比例。
///
/// - `device`：拔插 / 拓扑变化后 Windows 可能复用或重排该名字，匹配时还要
///   校验几何（见 [`origin_matches`]）；
/// - `rect`：记录时该屏的完整矩形——既是「归属屏是否回归」的几何判据，
///   也是屏内比例的分母（映射目标可以不是它）；
/// - `rel`：框左上角在该屏内的位置比例，搬回/重定位的载荷。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonitorOrigin {
    /// 显示器设备名（空串 = 未知，此时仅按几何匹配）
    pub device: String,
    /// 记录时归属屏完整矩形（虚拟桌面物理像素）
    pub rect: BoundsI,
    /// 框在归属屏内的位置比例
    pub rel: RelF,
}

/// 外接范围是否完整包含矩形（含边界贴齐）。
#[must_use]
pub fn bounds_contain_rect(bounds: &BoundsI, rect: &RectI) -> bool {
    let (ox, oy, w, h) = *bounds;
    rect[0] >= ox
        && rect[1] >= oy
        && rect[2] <= ox.saturating_add(w)
        && rect[3] <= oy.saturating_add(h)
}

/// 96 基准逻辑像素 → 指定 DPI 物理像素（与平台 `scale_for_dpi` 同式，
/// 在纯逻辑层自带一份，避免 box_core 反向依赖 platform）。
///
/// 非法 DPI（0 或 <96，Windows 实际最小就是 96）统一按 96 基准兜底。
#[must_use]
pub fn logical_to_physical(px: i32, dpi: u32) -> i32 {
    let scaled = (i64::from(px.max(0)) * i64::from(dpi.max(96))) / 96;
    scaled.max(1).min(i64::from(i32::MAX)) as i32
}

/// 目标屏上「保持逻辑口径」的物理尺寸：cur_phys x（target_dpi / cur_dpi）。
///
/// 输入非法（0/负）一律兜到至少 1px；非法 DPI 按 96 基准兜底（Windows
/// 实际最小 DPI 就是 96，与 [`logical_to_physical`] 同约定）。
#[must_use]
pub fn target_physical_size(cur_phys: [i32; 2], cur_dpi: u32, target_dpi: u32) -> [i32; 2] {
    let cd = f64::from(cur_dpi.max(96));
    let td = f64::from(target_dpi.max(96));
    let scale = td / cd;
    let conv = |v: i32| -> i32 {
        let s = f64::from(v.max(0)) * scale;
        if s <= 1.0 {
            1
        } else if s >= f64::from(i32::MAX) {
            i32::MAX
        } else {
            s.round() as i32
        }
    };
    [conv(cur_phys[0]), conv(cur_phys[1])]
}

/// 目标屏 DPI 下的玻璃最小物理尺寸（逻辑下限 200x140 折算）。
#[must_use]
pub fn min_physical(dpi: u32) -> (i32, i32) {
    (
        logical_to_physical(GLASS_MIN_LOGICAL_W, dpi),
        logical_to_physical(GLASS_MIN_LOGICAL_H, dpi),
    )
}

/// 两个外接范围（矩形）的交并比。非法尺寸返回 0.0，全程 saturating 不 panic。
#[must_use]
pub fn iou(a: &BoundsI, b: &BoundsI) -> f64 {
    let (ax, ay, aw, ah) = *a;
    let (bx, by, bw, bh) = *b;
    if aw <= 0 || ah <= 0 || bw <= 0 || bh <= 0 {
        return 0.0;
    }
    let left = i128::from(ax).max(i128::from(bx));
    let top = i128::from(ay).max(i128::from(by));
    let right = (i128::from(ax) + i128::from(aw)).min(i128::from(bx) + i128::from(bw));
    let bottom = (i128::from(ay) + i128::from(ah)).min(i128::from(by) + i128::from(bh));
    let iw = (right - left).max(0);
    let ih = (bottom - top).max(0);
    let inter = iw * ih;
    if inter == 0 {
        return 0.0;
    }
    let union = i128::from(aw) * i128::from(ah) + i128::from(bw) * i128::from(bh) - inter;
    if union <= 0 {
        return 0.0;
    }
    inter as f64 / union as f64
}

/// 把框矩形折算为「归属屏内的位置比例」（只含左上角位置，不含尺寸）。
///
/// 屏尺寸非法（<=0）时返回全零（调用方据此拒绝使用）；各分量夹到 `[0, 1]`。
/// 先转 f64 再相减：虚拟桌面坐标接近 i32 边界时，i32 相减会溢出 panic。
#[must_use]
pub fn to_relative(box_rect: RectI, monitor: BoundsI) -> RelF {
    let (ox, oy, mw, mh) = monitor;
    if mw <= 0 || mh <= 0 {
        return [0.0; 2];
    }
    let clamp01 = |v: f64| v.clamp(0.0, 1.0) as f32;
    [
        clamp01((f64::from(box_rect[0]) - f64::from(ox)) / f64::from(mw)),
        clamp01((f64::from(box_rect[1]) - f64::from(oy)) / f64::from(mh)),
    ]
}

/// 按屏内位置比例 + 显式物理尺寸，在目标屏上还原框矩形（映射的唯一出口）。
///
/// - `size` 传入的是**目标屏口径**的物理尺寸（用 [`target_physical_size`]
///   或启动路径的 96 逻辑 × 目标 DPI 现算）；
/// - 尺寸先抬到目标 DPI 的玻璃下限（[`min_physical`]，屏比下限还小则屏优先），
///   超过屏尺寸则按长宽比整体等比缩小（不剪角）；
/// - 左上角 = 屏原点 + 比例 x 屏尺寸，再经 `fit_into_bounds` 兜底整体装入目标屏；
/// - 同屏 + 同尺寸往返恒等（±1px 取整）。
#[must_use]
pub fn from_relative(rel: RelF, size: [i32; 2], monitor: BoundsI, dpi: u32) -> RectI {
    let (ox, oy, mw, mh) = monitor;
    if mw <= 0 || mh <= 0 {
        return [ox, oy, ox.saturating_add(1), oy.saturating_add(1)];
    }
    let r = |v: f32| {
        if v.is_finite() {
            f64::from(v).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let (floor_w, floor_h) = min_physical(dpi);
    // 屏比玻璃下限还小的极端显示模式：屏尺寸优先，clamp 下界不得大于上界
    let floor_w = floor_w.max(1).min(mw);
    let floor_h = floor_h.max(1).min(mh);
    let mut w = size[0].max(floor_w).max(1);
    let mut h = size[1].max(floor_h).max(1);
    // 装不下：按长宽比整体等比缩小（先宽后高的木桶系数），保证整框可见
    let fit = (f64::from(mw) / f64::from(w))
        .min(f64::from(mh) / f64::from(h))
        .min(1.0);
    if fit < 1.0 {
        w = (f64::from(w) * fit).round().max(1.0) as i32;
        h = (f64::from(h) * fit).round().max(1.0) as i32;
    }
    let x = ox.saturating_add(
        (r(rel[0]) * f64::from(mw))
            .round()
            .clamp(0.0, f64::from(mw)) as i32,
    );
    let y = oy.saturating_add(
        (r(rel[1]) * f64::from(mh))
            .round()
            .clamp(0.0, f64::from(mh)) as i32,
    );
    super::monitor_geom::fit_into_bounds([x, y, x.saturating_add(w), y.saturating_add(h)], monitor)
}

/// 归属身份与某块在岗显示器是否匹配。
///
/// 匹配规则（满足其一即可）：
/// 1. 几何强匹配：矩形 IoU >= 0.75（设备名可能被 Windows 重排，几何可信时优先）；
/// 2. 设备名相同且分辨率相同（用户在显示设置里挪动屏幕排列 → 几何 IoU 不达标，
///    但仍应认定为同一台显示器，允许搬回）；
/// 3. 设备名相同且屏原点相同（同一台显示器改了分辨率 → 位置不变，照样搬回）。
///
/// 故意不接受「设备名相同但位置、分辨率都不同」：拔错口 / 换屏时宁可
/// 不搬回（框留在在岗屏可操作），也不把框砸到身份存疑的屏上。
#[must_use]
pub fn origin_matches(origin: &MonitorOrigin, device: &str, bounds: &BoundsI) -> bool {
    let geo_strong = iou(&origin.rect, bounds) >= 0.75;
    let device_same = !origin.device.is_empty() && device.eq_ignore_ascii_case(&origin.device);
    let size_same = origin.rect.2 == bounds.2 && origin.rect.3 == bounds.3;
    let origin_same = origin.rect.0 == bounds.0 && origin.rect.1 == bounds.1;
    geo_strong || (device_same && size_same) || (device_same && origin_same)
}

/// 在在岗屏列表中查找归属身份的索引（[`origin_matches`] 的第一个命中）。
#[must_use]
pub fn live_index_for_origin(origin: &MonitorOrigin, live: &[LiveMonitor]) -> Option<usize> {
    live.iter()
        .position(|m| origin_matches(origin, &m.device, &m.bounds))
}

/// 框当前完整落在哪块在岗屏上（用于刷新归属记账 / 判断「无需干预」）。
#[must_use]
pub fn current_origin(box_rect: RectI, live: &[LiveMonitor]) -> Option<MonitorOrigin> {
    live.iter()
        .find(|m| bounds_contain_rect(&m.bounds, &box_rect))
        .map(|m| MonitorOrigin {
            device: m.device.clone(),
            rect: m.bounds,
            rel: to_relative(box_rect, m.bounds),
        })
}

/// 单个分类框的对账决策。
#[derive(Debug, Clone, PartialEq)]
pub enum RelocatePlan {
    /// 在岗屏集合为空（热插拔瞬间枚举尚未稳定）或无法给出安全落点：
    /// 什么都不做，由调用方安排复查拍。
    Skip,
    /// 框已完整落在在岗屏内：仅刷新归属记账（`origin` 与现状一致时调用方不落盘）。
    /// `relocated == true` 表示归属屏尚未回归、框仍在临时落位上等待搬回，
    /// 此时 `origin` 原样保留（不覆盖归属身份）。
    Settled {
        origin: MonitorOrigin,
        relocated: bool,
    },
    /// 归属屏回归且此前被拔屏暂搬：等比搬回 `rect`（目标屏 = `target`），
    /// 并收养刷新后的 `origin`、清除 relocated 状态。
    ReturnHome {
        target: usize,
        rect: RectI,
        origin: MonitorOrigin,
    },
    /// 框滞留在已消失的屏上：按归属屏记录的屏内位置比例映射到最近的在岗屏
    /// （`target` + `rect`，尺寸走逻辑口径）。归属身份保持不变，调用方置
    /// relocated = true，等归属屏回归时自动搬回。
    Relocate { target: usize, rect: RectI },
    /// 滞留但没有归属信息（历史配置）：退回「整框吸附进最近在岗屏」（只平移，
    /// 不做 DPI 换算），并收养新的归属记账，无待搬回语义。
    Snap { rect: RectI, origin: MonitorOrigin },
}

/// 对一个分类框当前状态做对账决策（纯函数）。
///
/// - `box_rect`：框当前的内容矩形（虚拟桌面物理像素）；
/// - `origin` / `relocated`：该框记录的归属屏身份、是否处于「等待搬回」状态；
/// - `live`：在岗屏列表（设备名 + 矩形 + DPI）；
/// - `bounds`：虚拟桌面外接范围（`Snap` 分支兜底用）；
/// - `cur_dpi`：窗口当前 DPI（尺寸逻辑口径换算的分母）。
///
/// 尺寸口径见模块头：位置按比例、尺寸保持逻辑（目标物理 = 当前物理 x
/// 目标DPI/当前DPI），与手工跨屏拖拽的 `WM_DPICHANGED` 行为同构。
/// 「完整落在某在岗屏内」判据与既有拖拽边界共用 [`rect_fits_any_monitor`]。
#[must_use]
pub fn plan_for_box(
    box_rect: RectI,
    origin: Option<&MonitorOrigin>,
    relocated: bool,
    live: &[LiveMonitor],
    bounds: BoundsI,
    cur_dpi: u32,
) -> RelocatePlan {
    if live.is_empty() {
        return RelocatePlan::Skip;
    }
    let monitors: Vec<MonitorI> = live
        .iter()
        .map(|m| ((m.bounds.0, m.bounds.1), (m.bounds.2, m.bounds.3)))
        .collect();
    let cur_size = [
        box_rect[2].saturating_sub(box_rect[0]).max(1),
        box_rect[3].saturating_sub(box_rect[1]).max(1),
    ];

    if rect_fits_any_monitor(box_rect, &monitors) {
        // 待搬回且归属屏已回归 → 等比搬回
        if relocated {
            if let Some(o) = origin {
                if let Some(idx) = live_index_for_origin(o, live) {
                    let home = &live[idx];
                    let size = target_physical_size(cur_size, cur_dpi, home.dpi);
                    let rect = from_relative(o.rel, size, home.bounds, home.dpi);
                    let origin = MonitorOrigin {
                        device: home.device.clone(),
                        rect: home.bounds,
                        rel: to_relative(rect, home.bounds),
                    };
                    return RelocatePlan::ReturnHome {
                        target: idx,
                        rect,
                        origin,
                    };
                }
                // 归属屏还没回来，框当前落位合法：保留身份等待，不覆盖记账
                return RelocatePlan::Settled {
                    origin: o.clone(),
                    relocated: true,
                };
            }
        }
        // 归属屏已离岗、但框当前落位合法 —— 典型场景是「拔屏瞬间 Windows
        // 自动把窗口救援到剩余屏」（系统自己的搬移，不走我们的对账）。
        // 这不是用户的重新摆放意图：必须保留原屏身份并置 relocated=true 等待
        // 重连搬回；只有用户驱动的会话收尾才允许收养新屏（adopt_current_monitor）。
        // （2026-11-27 复测：此前这里直接收养在岗屏，导致重连后永远不搬回。）
        if !relocated {
            if let Some(o) = origin {
                if live_index_for_origin(o, live).is_none() {
                    return RelocatePlan::Settled {
                        origin: o.clone(),
                        relocated: true,
                    };
                }
            }
        }
        // 常规：刷新归属记账（无变化时调用方自行跳过落盘）
        if let Some(cur) = current_origin(box_rect, live) {
            return RelocatePlan::Settled {
                origin: cur,
                relocated: false,
            };
        }
        // fits 与 current_origin 判据等价，理论不可达；防御性跳过
        return RelocatePlan::Skip;
    }

    if let Some(o) = origin {
        // 滞留：以归属屏矩形为源，按位置比例映射到「离消失屏最近」的在岗屏
        let home_rect: RectI = [
            o.rect.0,
            o.rect.1,
            o.rect.0.saturating_add(o.rect.2),
            o.rect.1.saturating_add(o.rect.3),
        ];
        let target = monitor_index_for_center(home_rect, &monitors).unwrap_or(0);
        let dst = &live[target];
        let size = target_physical_size(cur_size, cur_dpi, dst.dpi);
        let rect = from_relative(o.rel, size, dst.bounds, dst.dpi);
        RelocatePlan::Relocate { target, rect }
    } else {
        // 滞留且无身份（历史数据）：按框自身中心吸附进最近在岗屏（只平移）
        let rect = snap_into_single_monitor(box_rect, &monitors, bounds);
        let origin = current_origin(rect, live).unwrap_or_default();
        RelocatePlan::Snap { rect, origin }
    }
}

/// 持久化形态 → 运行时形态（`config::BoxMonitorOrigin` 是磁盘结构，
/// 本模块的 `MonitorOrigin` 是纯逻辑结构，边界处一次性转换）。
impl From<&crate::config::BoxMonitorOrigin> for MonitorOrigin {
    fn from(m: &crate::config::BoxMonitorOrigin) -> Self {
        MonitorOrigin {
            device: m.device.clone(),
            rect: (m.rect[0], m.rect[1], m.rect[2], m.rect[3]),
            rel: [m.rel[0], m.rel[1]],
        }
    }
}

/// 运行时形态 → 持久化形态（不含 relocated，由调用方按窗口记账补齐）。
impl From<&MonitorOrigin> for crate::config::BoxMonitorOrigin {
    fn from(o: &MonitorOrigin) -> Self {
        crate::config::BoxMonitorOrigin {
            device: o.device.clone(),
            rect: [o.rect.0, o.rect.1, o.rect.2, o.rect.3],
            rel: [o.rel[0], o.rel[1]],
            relocated: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 主屏 1920x1080 @(0,0) 96 DPI；右侧副屏 2560x1440 @(1920,0) 96 DPI。
    fn live_dual() -> Vec<LiveMonitor> {
        vec![
            LiveMonitor {
                device: "\\\\.\\DISPLAY1".to_string(),
                bounds: (0, 0, 1920, 1080),
                dpi: 96,
            },
            LiveMonitor {
                device: "\\\\.\\DISPLAY2".to_string(),
                bounds: (1920, 0, 2560, 1440),
                dpi: 96,
            },
        ]
    }

    fn origin_on_secondary() -> MonitorOrigin {
        let secondary = (1920, 0, 2560, 1440);
        MonitorOrigin {
            device: "\\\\.\\DISPLAY2".to_string(),
            rect: secondary,
            rel: to_relative([2000, 100, 2560, 700], secondary),
        }
    }

    #[test]
    fn position_roundtrip_same_monitor_is_near_identity() {
        let monitor = (1920, 0, 2560, 1440);
        let rect = [2000, 100, 2560, 700];
        let rel = to_relative(rect, monitor);
        let size = [560, 600];
        let back = from_relative(rel, size, monitor, 96);
        for (got, want) in back.iter().zip(rect.iter()) {
            assert!(
                (got - want).abs() <= 2,
                "往返应恒等（±2px）: {back:?} vs {rect:?}"
            );
        }
    }

    #[test]
    fn position_maps_by_ratio_and_size_stays() {
        // 位置比例 0.25 映射到不同尺寸屏；尺寸显式传入、原样保持
        let src = (0, 0, 2560, 1440);
        let rel = to_relative([640, 360, 1280, 1080], src);
        let dst = (0, 0, 1920, 1080);
        let mapped = from_relative(rel, [640, 720], dst, 96);
        assert_eq!(mapped[0], 480, "x 比例 0.25 -> 1920*0.25");
        assert_eq!(mapped[1], 270, "y 比例 0.25 -> 1080*0.25");
        assert_eq!(mapped[2] - mapped[0], 640, "尺寸保持（不随屏大小缩放）");
        assert_eq!(mapped[3] - mapped[1], 720);
        assert!(bounds_contain_rect(&dst, &mapped), "映射结果必须整框在屏内");
    }

    #[test]
    fn size_follows_logical_baseline_across_dpi() {
        // 96 DPI 屏上 560x600 物理 = 560x600 逻辑；映射到 192 DPI 屏应变成
        // 1120x1200 物理（逻辑不变），与手工跨屏拖拽的 WM_DPICHANGED 同构
        assert_eq!(target_physical_size([560, 600], 96, 192), [1120, 1200]);
        assert_eq!(target_physical_size([1120, 1200], 192, 96), [560, 600]);
        assert_eq!(target_physical_size([400, 400], 144, 144), [400, 400]);
        // 非法输入兜底不 panic
        assert_eq!(target_physical_size([0, -5], 0, 0), [1, 1]);
    }

    #[test]
    fn from_relative_clamps_to_min_and_shrinks_aspect() {
        let monitor = (0, 0, 800, 600);
        // 尺寸小于玻璃下限 -> 抬到 200x140；贴右下边缘 -> fit_into_bounds 平移回来
        let rect = from_relative([0.99, 0.95], [10, 10], monitor, 96);
        assert_eq!(rect[2] - rect[0], 200, "宽应被抬到逻辑下限 200");
        assert_eq!(rect[3] - rect[1], 140, "高应被抬到逻辑下限 140");
        assert!(bounds_contain_rect(&monitor, &rect));
        // 比屏还大 -> 按长宽比等比缩（2000x1200 -> 800x480，5:3 保持）
        let big = from_relative([0.0, 0.0], [2000, 1200], monitor, 96);
        assert_eq!(big[2] - big[0], 800);
        assert_eq!(big[3] - big[1], 480, "必须保持长宽比而不是单轴裁切");
        assert!(bounds_contain_rect(&monitor, &big));
    }

    #[test]
    fn min_physical_scales_with_dpi() {
        assert_eq!(min_physical(96), (200, 140));
        assert_eq!(min_physical(192), (400, 280));
        assert_eq!(min_physical(0), (200, 140), "非法 DPI 按 96 兜底口径");
    }

    #[test]
    fn degenerate_inputs_never_panic() {
        assert_eq!(to_relative([0, 0, 100, 100], (0, 0, 0, 1080)), [0.0; 2]);
        let rect = from_relative([f32::NAN, 0.5], [100, 100], (0, 0, -10, 1080), 96);
        assert!(
            rect[2] > rect[0] && rect[3] > rect[1],
            "退化矩形也不得零面积"
        );
        // 屏比下限还小：屏尺寸优先，不得出现 min > max 的 clamp panic
        let tiny = from_relative([0.5, 0.5], [10, 10], (0, 0, 120, 100), 96);
        assert!(bounds_contain_rect(&(0, 0, 120, 100), &tiny));
    }

    #[test]
    fn extreme_to_relative_does_not_overflow() {
        let rel = to_relative(
            [i32::MAX - 10, i32::MIN, i32::MAX, i32::MIN + 100],
            (i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        );
        assert!(rel[0].is_finite() && rel[1].is_finite() && rel[0] >= 0.0 && rel[0] <= 1.0);
    }

    #[test]
    fn iou_basics() {
        let a = (0, 0, 100, 100);
        assert!((iou(&a, &a) - 1.0).abs() < f64::EPSILON);
        assert_eq!(iou(&a, &(1000, 1000, 100, 100)), 0.0);
        assert!((iou(&a, &(0, 0, 200, 100)) - 0.5).abs() < 1e-9);
        assert_eq!(iou(&a, &(0, 0, 0, 100)), 0.0);
    }

    #[test]
    fn origin_matches_rules() {
        let o = origin_on_secondary();
        // 设备名 + 矩形全等
        assert!(origin_matches(
            &o,
            "\\\\.\\DISPLAY2",
            &(1920, 0, 2560, 1440)
        ));
        // 设备名大小写不敏感
        assert!(origin_matches(
            &o,
            "\\\\.\\display2",
            &(1920, 0, 2560, 1440)
        ));
        // 同一块屏被挪了排列位置（设备同、分辨率同、原点不同）→ 认定回归
        assert!(origin_matches(
            &o,
            "\\\\.\\DISPLAY2",
            &(-640, 0, 2560, 1440)
        ));
        // 同一块屏改了分辨率但原点没变（设备同、尺寸不同）→ 认定回归
        assert!(origin_matches(
            &o,
            "\\\\.\\DISPLAY2",
            &(1920, 0, 2560, 1600)
        ));
        // 设备名相同但位置、分辨率都不同（疑似别的屏占了名额）→ 拒绝
        assert!(!origin_matches(&o, "\\\\.\\DISPLAY2", &(0, 0, 1920, 1080)));
        // 设备名变了但几何强匹配（Windows 重排名字）→ 接受
        assert!(origin_matches(
            &o,
            "\\\\.\\DISPLAY3",
            &(1920, 0, 2560, 1440)
        ));
        // 设备名为空（历史兜底）时只认几何强匹配
        let no_name = MonitorOrigin {
            device: String::new(),
            ..o.clone()
        };
        assert!(origin_matches(
            &no_name,
            "\\\\.\\DISPLAY9",
            &(1920, 0, 2560, 1440)
        ));
        assert!(!origin_matches(
            &no_name,
            "\\\\.\\DISPLAY9",
            &(-640, 0, 2560, 1440)
        ));
    }

    #[test]
    fn plan_skips_when_no_live_monitors() {
        let rect = [2000, 100, 2560, 700];
        let o = origin_on_secondary();
        let plan = plan_for_box(rect, Some(&o), false, &[], (0, 0, 1920, 1080), 96);
        assert_eq!(plan, RelocatePlan::Skip);
    }

    #[test]
    fn plan_settled_refreshes_origin_when_box_on_live_monitor() {
        let live = live_dual();
        let rect = [2000, 100, 2560, 700];
        let plan = plan_for_box(rect, None, false, &live, (0, 0, 4480, 1440), 96);
        match plan {
            RelocatePlan::Settled { origin, relocated } => {
                assert_eq!(origin.device, "\\\\.\\DISPLAY2");
                assert!(!relocated);
                assert!(bounds_contain_rect(&origin.rect, &rect));
                let back = from_relative(origin.rel, [560, 600], origin.rect, 96);
                for (g, w) in back.iter().zip(rect.iter()) {
                    assert!((g - w).abs() <= 2);
                }
            }
            other => panic!("应给出 Settled，实际 {other:?}"),
        }
    }

    #[test]
    fn plan_relocates_stranded_box_position_by_ratio_size_logical() {
        // 副屏拔掉后只剩主屏；框还停在副屏坐标，归属记录指向副屏
        let live = vec![LiveMonitor {
            device: "\\\\.\\DISPLAY1".to_string(),
            bounds: (0, 0, 1920, 1080),
            dpi: 96,
        }];
        let o = origin_on_secondary();
        let rect = [2000, 100, 2560, 700];
        let plan = plan_for_box(rect, Some(&o), false, &live, (0, 0, 1920, 1080), 96);
        match plan {
            RelocatePlan::Relocate {
                target,
                rect: new_rect,
            } => {
                assert_eq!(target, 0);
                // 源屏内位置比例 (2000-1920)/2560 ≈ 0.03125 -> 1920*0.03125 = 60
                assert_eq!(new_rect[0], 60);
                assert_eq!(new_rect[1], 75);
                // 同 DPI：物理尺寸原样保持（560x600），不再随屏分辨率缩放
                assert_eq!(new_rect[2] - new_rect[0], 560);
                assert_eq!(new_rect[3] - new_rect[1], 600);
                assert!(bounds_contain_rect(&live[0].bounds, &new_rect));
            }
            other => panic!("应给出 Relocate，实际 {other:?}"),
        }
    }

    #[test]
    fn plan_relocate_scales_size_by_target_dpi_logical_baseline() {
        // 主屏 192 DPI（1560x1040 逻辑口径下的物理 3120x2080 简化为 1920x1080 测试）
        let live = vec![LiveMonitor {
            device: "\\\\.\\DISPLAY1".to_string(),
            bounds: (0, 0, 1920, 1080),
            dpi: 192,
        }];
        let o = origin_on_secondary();
        let rect = [2000, 100, 2560, 700]; // 560x600 @96 DPI
        let plan = plan_for_box(rect, Some(&o), false, &live, (0, 0, 1920, 1080), 96);
        match plan {
            RelocatePlan::Relocate { rect: new_rect, .. } => {
                // 逻辑尺寸保持：560x600@96 -> 1120x1200@192；高 1200 > 1080
                // -> 按长宽比等比缩到 [1008, 1080]（0.9 系数）
                assert_eq!(new_rect[2] - new_rect[0], 1008);
                assert_eq!(new_rect[3] - new_rect[1], 1080);
                assert!(bounds_contain_rect(&live[0].bounds, &new_rect));
            }
            other => panic!("应给出 Relocate，实际 {other:?}"),
        }
    }

    #[test]
    fn plan_relocate_while_already_relocated_keeps_waiting() {
        // 已处于等待搬回状态，又因改动滞留：仍按归属比例重定位，不清 pending
        let live = vec![LiveMonitor {
            device: "\\\\.\\DISPLAY1".to_string(),
            bounds: (0, 0, 1920, 1080),
            dpi: 96,
        }];
        let o = origin_on_secondary();
        let rect = [3000, 1300, 3600, 1900]; // 超出主屏范围
        let plan = plan_for_box(rect, Some(&o), true, &live, (0, 0, 1920, 1080), 96);
        assert!(matches!(plan, RelocatePlan::Relocate { target: 0, .. }));
    }

    #[test]
    fn plan_returns_home_when_monitor_back_and_relocated() {
        let live = live_dual();
        let o = origin_on_secondary();
        // 框当前合法停在主屏（拔屏时被映射过来的 560x600 落位）
        let rect = [60, 75, 620, 675];
        let plan = plan_for_box(rect, Some(&o), true, &live, (0, 0, 4480, 1440), 96);
        match plan {
            RelocatePlan::ReturnHome {
                target,
                rect: back,
                origin: new_origin,
            } => {
                assert_eq!(target, 1);
                // 应回到副屏上记录的位置（尺寸同 DPI 原样，位置精确复原）
                assert_eq!(back[0], 2000);
                assert_eq!(back[1], 100);
                assert_eq!(back[2] - back[0], 560);
                assert_eq!(back[3] - back[1], 600);
                assert_eq!(new_origin.device, "\\\\.\\DISPLAY2");
                assert_eq!(new_origin.rect, (1920, 0, 2560, 1440));
            }
            other => panic!("应给出 ReturnHome，实际 {other:?}"),
        }
    }

    #[test]
    fn plan_does_not_return_home_after_user_readopted_primary() {
        // 用户场景（2026-11-27 明确验收语义）：拔屏后框降级到主屏，用户
        // 在主屏**主动挪动过**某个框（会话收尾已收养主屏、清 relocated）；
        // 副屏重连时这个框**不得**再搬回副屏，保持用户选定的落位。
        let live = live_dual();
        let adopted = MonitorOrigin {
            device: "\\\\.\\DISPLAY1".to_string(),
            rect: (0, 0, 1920, 1080),
            rel: to_relative([500, 300, 1060, 900], (0, 0, 1920, 1080)),
        };
        let rect = [500, 300, 1060, 900];
        let plan = plan_for_box(rect, Some(&adopted), false, &live, (0, 0, 4480, 1440), 96);
        match plan {
            RelocatePlan::Settled { origin, relocated } => {
                assert_eq!(origin.device, "\\\\.\\DISPLAY1", "归属应保持用户收养的主屏");
                assert!(!relocated, "用户已重新摆放，不得有待搬回期待");
            }
            other => panic!("副屏重连不得搬回用户挪过的框，实际 {other:?}"),
        }
    }

    #[test]
    fn plan_keeps_waiting_when_home_still_gone() {
        // relocated=true 但归属屏没回来，框现在合法停在主屏：保留身份、不搬
        let live = vec![LiveMonitor {
            device: "\\\\.\\DISPLAY1".to_string(),
            bounds: (0, 0, 1920, 1080),
            dpi: 96,
        }];
        let o = origin_on_secondary();
        let rect = [60, 75, 620, 675];
        let plan = plan_for_box(rect, Some(&o), true, &live, (0, 0, 1920, 1080), 96);
        match plan {
            RelocatePlan::Settled { origin, relocated } => {
                assert!(relocated);
                assert_eq!(origin, o);
            }
            other => panic!("应保留等待状态，实际 {other:?}"),
        }
    }

    #[test]
    fn plan_settled_keeps_home_and_marks_waiting_when_system_rescued_box() {
        // 拔屏后 Windows 自动把框「救援」到主屏合法位置，relocated 还没置过：
        // 对账不得就此收养主屏——必须保留副屏身份并标记待搬回，重连才能自动搬回。
        let live = vec![LiveMonitor {
            device: "\\\\.\\DISPLAY1".to_string(),
            bounds: (0, 0, 3120, 2080),
            dpi: 192,
        }];
        let o = origin_on_secondary();
        let rescued = [100, 100, 660, 700]; // 主屏内的合法位置
        let plan = plan_for_box(rescued, Some(&o), false, &live, (0, 0, 3120, 2080), 192);
        match plan {
            RelocatePlan::Settled { origin, relocated } => {
                assert_eq!(origin, o, "归属身份必须保留为离岗的副屏");
                assert!(relocated, "系统救援落位 = 暂搬，必须标记待搬回");
            }
            other => panic!("应保留身份等待搬回，实际 {other:?}"),
        }
    }

    #[test]
    fn plan_falls_back_to_snap_without_origin() {
        // 历史配置无归属信息：只平移吸附，收养新归属、不带搬回期待
        let live = vec![LiveMonitor {
            device: "\\\\.\\DISPLAY1".to_string(),
            bounds: (0, 0, 1920, 1080),
            dpi: 96,
        }];
        let rect = [2000, 100, 2560, 700];
        let plan = plan_for_box(rect, None, false, &live, (0, 0, 1920, 1080), 96);
        match plan {
            RelocatePlan::Snap {
                rect: snapped,
                origin,
            } => {
                assert!(bounds_contain_rect(&live[0].bounds, &snapped));
                assert_eq!(snapped[2] - snapped[0], 560, "吸附只平移不改尺寸");
                assert_eq!(origin.device, "\\\\.\\DISPLAY1");
            }
            other => panic!("应给出 Snap，实际 {other:?}"),
        }
    }

    #[test]
    fn plan_snap_shrinks_oversized_historical_box_into_monitor() {
        // 历史框比在岗屏还宽：snap 分支至少保证左上角合法、不越虚拟桌面
        let live = vec![LiveMonitor {
            device: "\\\\.\\DISPLAY1".to_string(),
            bounds: (0, 0, 1920, 1080),
            dpi: 96,
        }];
        let plan = plan_for_box(
            [-200, -100, 4000, 3000],
            None,
            false,
            &live,
            (0, 0, 1920, 1080),
            96,
        );
        match plan {
            RelocatePlan::Snap { rect, .. } => {
                assert!(rect[0] >= 0 && rect[1] >= 0, "超大框左上角必须被夹回");
            }
            other => panic!("应给出 Snap，实际 {other:?}"),
        }
    }

    #[test]
    fn config_roundtrip_of_origin_struct() {
        let o = origin_on_secondary();
        let persisted = crate::config::BoxMonitorOrigin::from(&o);
        let back = MonitorOrigin::from(&persisted);
        assert_eq!(o, back, "config 形态往返必须无损");
    }

    #[test]
    fn extreme_coordinates_do_not_panic() {
        let live = vec![LiveMonitor {
            device: "\\\\.\\DISPLAY1".to_string(),
            bounds: (0, 0, 1920, 1080),
            dpi: 96,
        }];
        let o = MonitorOrigin {
            device: "\\\\.\\DISPLAYX".to_string(),
            rect: (i32::MAX - 100, i32::MIN, 100, 100),
            rel: [1.0, 1.0],
        };
        let plan = plan_for_box(
            [i32::MAX - 50, i32::MIN + 10, i32::MAX, i32::MIN + 90],
            Some(&o),
            true,
            &live,
            (0, 0, 1920, 1080),
            96,
        );
        assert!(
            matches!(
                plan,
                RelocatePlan::Relocate { .. } | RelocatePlan::Settled { .. }
            ),
            "极端坐标不得 panic，实际 {plan:?}"
        );
    }
}
