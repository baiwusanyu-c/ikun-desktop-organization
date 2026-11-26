//! 多显示器几何纯逻辑
//!
//! 分类框（盒子）在跨屏拖拽时的边界与归属计算，平台无关、0 unsafe、可单测。
//!
//! # 坐标口径（与 `platform/windows/monitor_utils` 保持一致）
//! - 全部为**虚拟桌面物理像素**（副屏可能在负坐标区）；
//! - 矩形用 `[left, top, right, bottom]` 表示，只关心位置与宽高，不依赖 Win32 类型；
//! - 显示器用 `((origin_x, origin_y), (width, height))` 表示，可直接由
//!   `MONITORINFO.rcMonitor` / `MonitorSnapshot(origin,size)` 转换。
//!
//! # 为什么需要它
//! 1. 分类框拖拽过去**完全没有边界约束**（`get_virtual_desktop_bounds` 此前零调用），
//!    可以把盒子整块拖出屏幕并落盘，重启后仍然看不见（用户报「漂移出屏幕」）。
//! 2. 液态玻璃块是**单屏模型**（liquid-verre-rs `core::block::rehome_block`：块只归属
//!    中心所在屏的画布，并把中心钳到该屏内，明确「不做跨屏大玻璃」）。宿主在
//!    宿主驱动的几何推进（新建、恢复布局、`WM_MOVING`）里必须先给出「整框留在
//!    单屏内」的合法矩形，否则内容窗口与玻璃块各说各话，跨界瞬间表现为
//!    一半主屏一半副屏的错位残影。

/// 矩形：`[left, top, right, bottom]`（虚拟桌面物理像素）。
pub type RectI = [i32; 4];

/// 显示器：`((origin_x, origin_y), (width, height))`（虚拟桌面物理像素）。
pub type MonitorI = ((i32, i32), (i32, i32));

/// 外接范围：`(origin_x, origin_y, width, height)`（虚拟桌面物理像素）。
pub type BoundsI = (i32, i32, i32, i32);

/// 判断显示器是否有效（宽高必须为正，脏数据直接忽略）。
#[must_use]
pub fn monitor_is_valid(monitor: &MonitorI) -> bool {
    monitor.1 .0 > 0 && monitor.1 .1 > 0
}

/// 安全夹取：上界小于下界时退回下界，避免 `i32::clamp` 因非法范围 panic。
fn clamp_axis(value: i32, min: i32, max: i32) -> i32 {
    value.clamp(min, max.max(min))
}

/// 把矩形**整体**夹进外接范围：只改位置不改尺寸。
///
/// - 正常尺寸：结果完整落在 `bounds` 内；
/// - 矩形比外接范围还大：退化为「左上角对齐范围原点」，至少保证不往负方向漂；
/// - 极端坐标（接近 i32 边界）：全程 saturating 运算，不得溢出 panic。
#[must_use]
pub fn clamp_to_bounds(rect: RectI, bounds: BoundsI) -> RectI {
    let (bx, by, bw, bh) = bounds;
    if bw <= 0 || bh <= 0 {
        return rect;
    }
    let width = rect[2].saturating_sub(rect[0]);
    let height = rect[3].saturating_sub(rect[1]);
    let right = bx.saturating_add(bw);
    let bottom = by.saturating_add(bh);

    let left = if width >= bw {
        bx
    } else {
        clamp_axis(rect[0], bx, right.saturating_sub(width))
    };
    let top = if height >= bh {
        by
    } else {
        clamp_axis(rect[1], by, bottom.saturating_sub(height))
    };
    [
        left,
        top,
        left.saturating_add(width),
        top.saturating_add(height),
    ]
}

/// 按矩形中心选择归属显示器：中心命中优先，未命中取中心距离最近的一块。
///
/// 与库 `rehome_block` 的归屏判据同构（中心所在屏），因此宿主据此摆窗不会与
/// 库的归屏结论长期对立；表为空返回 `None`。
#[must_use]
pub fn monitor_index_for_center(rect: RectI, monitors: &[MonitorI]) -> Option<usize> {
    let (cx, cy) = rect_center(rect)?;
    let mut best: Option<(usize, i128)> = None;
    for (i, m) in monitors.iter().enumerate() {
        if !monitor_is_valid(m) {
            continue;
        }
        let (ox, oy) = m.0;
        let (w, h) = m.1;
        let inside = cx >= ox && cx < ox.saturating_add(w) && cy >= oy && cy < oy.saturating_add(h);
        // 先转 i64 再相减：虚拟桌面坐标接近 i32 边界时，i32 相减会 panic。
        let dx = i64::from(cx) - i64::from(ox);
        let dy = i64::from(cy) - i64::from(oy);
        // 距离用「到矩形的最短向量」平方，避免屏与屏之间选到意外的那块。
        // 用 i128 保存平方和：极端坐标下 i64 也可能溢出。
        let outside_x = (-dx).max(dx - i64::from(w)).max(0) as i128;
        let outside_y = (-dy).max(dy - i64::from(h)).max(0) as i128;
        let score = outside_x * outside_x + outside_y * outside_y;
        // 命中（score=0 且 inside）优先于仅距离最近。
        let rank = if inside { score } else { score + 1 };
        match best {
            Some((_, best_score)) if best_score <= rank => {}
            _ => best = Some((i, rank)),
        }
    }
    best.map(|(i, _)| i)
}

/// 把矩形吸附到「完整留在单块显示器内」。
///
/// - 中心命中的屏能装下该尺寸：结果整框落在该屏内（跨界停靠在目标屏一侧）；
/// - 屏装不下该尺寸（超大框）：只保证不越出该屏左上边界，交给 `bounds` 兜底；
/// - `monitors` 为空或全部无效：退回 `clamp_to_bounds(rect, bounds)`。
///
/// 返回值永远再经一次 `clamp_to_bounds`，即「绝不允许整框跑到虚拟桌面之外」。
#[must_use]
pub fn snap_into_single_monitor(rect: RectI, monitors: &[MonitorI], bounds: BoundsI) -> RectI {
    let Some(index) = monitor_index_for_center(rect, monitors) else {
        return clamp_to_bounds(rect, bounds);
    };
    let monitor = monitors[index];
    let width = rect[2].saturating_sub(rect[0]);
    let height = rect[3].saturating_sub(rect[1]);
    let (ox, oy) = monitor.0;
    let (mw, mh) = monitor.1;
    let right = ox.saturating_add(mw);
    let bottom = oy.saturating_add(mh);

    let left = if width >= mw {
        ox
    } else {
        clamp_axis(rect[0], ox, right.saturating_sub(width))
    };
    let top = if height >= mh {
        oy
    } else {
        clamp_axis(rect[1], oy, bottom.saturating_sub(height))
    };
    clamp_to_bounds(
        [
            left,
            top,
            left.saturating_add(width),
            top.saturating_add(height),
        ],
        bounds,
    )
}

/// 矩形是否已经完整落在某块有效显示器内（用于判断「无需干预」）。
#[must_use]
pub fn rect_fits_any_monitor(rect: RectI, monitors: &[MonitorI]) -> bool {
    monitors.iter().any(|m| {
        monitor_is_valid(m)
            && rect[0] >= m.0 .0
            && rect[1] >= m.0 .1
            && rect[2] <= m.0 .0.saturating_add(m.1 .0)
            && rect[3] <= m.0 .1.saturating_add(m.1 .1)
    })
}

/// 把矩形整体塞进一个外接范围（必要时**允许缩小**），并保持左上角不越界。
///
/// 与 [`clamp_to_bounds`] 的区别：后者承诺不改尺寸（放不下就只保证左上角合法），
/// 本函数用于「DPI 变大后分类框比目标屏还大」这种真的装不下的场合——
/// 宁可缩小也要保证整框可见，否则玻璃块会比画布大，用户看到一块被切掉的玻璃。
#[must_use]
pub fn fit_into_bounds(rect: RectI, bounds: BoundsI) -> RectI {
    let (bx, by, bw, bh) = bounds;
    if bw <= 0 || bh <= 0 {
        return rect;
    }
    let width = (rect[2].saturating_sub(rect[0])).min(bw).max(1);
    let height = (rect[3].saturating_sub(rect[1])).min(bh).max(1);
    let left = clamp_axis(rect[0], bx, bx.saturating_add(bw).saturating_sub(width));
    let top = clamp_axis(rect[1], by, by.saturating_add(bh).saturating_sub(height));
    [
        left,
        top,
        left.saturating_add(width),
        top.saturating_add(height),
    ]
}

/// 矩形中心点（空矩形返回 `None`）。
#[must_use]
pub fn rect_center(rect: RectI) -> Option<(i32, i32)> {
    let width = rect[2].checked_sub(rect[0])?;
    let height = rect[3].checked_sub(rect[1])?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some((rect[0] + width / 2, rect[1] + height / 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 主屏 1920x1080 @(0,0)，副屏 2560x1440 在右侧（模拟常见左右双屏）。
    fn dual_monitors() -> Vec<MonitorI> {
        vec![((0, 0), (1920, 1080)), ((1920, 0), (2560, 1440))]
    }

    fn virtual_desktop() -> BoundsI {
        (0, 0, 4480, 1440)
    }

    #[test]
    fn center_inside_primary_is_index_zero() {
        assert_eq!(
            monitor_index_for_center([100, 100, 500, 400], &dual_monitors()),
            Some(0)
        );
    }

    #[test]
    fn center_on_secondary_is_index_one() {
        assert_eq!(
            monitor_index_for_center([2000, 100, 2400, 400], &dual_monitors()),
            Some(1)
        );
    }

    #[test]
    fn center_outside_all_monitors_picks_nearest() {
        // 中心 x=4600 落在所有屏之外 → 取最近的副屏
        assert_eq!(
            monitor_index_for_center([4400, 100, 4800, 400], &dual_monitors()),
            Some(1)
        );
    }

    #[test]
    fn empty_monitor_table_is_none() {
        assert_eq!(monitor_index_for_center([0, 0, 10, 10], &[]), None);
    }

    #[test]
    fn invalid_monitor_entries_are_ignored() {
        let monitors = [((0, 0), (0, 0)), ((1000, 0), (800, 600))];
        assert_eq!(
            monitor_index_for_center([1100, 100, 1200, 200], &monitors),
            Some(1)
        );
    }

    #[test]
    fn clamp_keeps_size_and_moves_only() {
        let clamped = clamp_to_bounds([4400, 100, 4800, 400], virtual_desktop());
        assert_eq!(clamped, [4080, 100, 4480, 400]);
        assert_eq!(clamped[2] - clamped[0], 400, "宽度必须原样保留");
        assert_eq!(clamped[3] - clamped[1], 300, "高度必须原样保留");
    }

    #[test]
    fn clamp_handles_negative_secondary_monitor() {
        // 副屏在左侧（原点是负 x），往左拖出界必须被夹回来
        let bounds = (-1920, 0, 3840, 1080);
        let clamped = clamp_to_bounds([-2200, 10, -1800, 210], bounds);
        assert_eq!(clamped, [-1920, 10, -1520, 210]);
    }

    #[test]
    fn clamp_of_oversized_rect_aligns_to_origin() {
        let clamped = clamp_to_bounds([-500, -500, 9000, 9000], (0, 0, 1920, 1080));
        assert_eq!(&clamped[0..2], &[0, 0]);
        assert_eq!(clamped[2] - clamped[0], 9500, "超大框只挪不改尺寸");
    }

    #[test]
    fn clamp_survives_extreme_coordinates() {
        let clamped = clamp_to_bounds([i32::MIN, i32::MIN, i32::MAX, i32::MAX], (0, 0, 1920, 1080));
        assert!(clamped[0] >= 0 && clamped[1] >= 0, "极端值必须被夹回边界内");
        // 超大矩形只保证左上角合法，且不得因为减法溢出得到比原值更小的错误结果
        assert_eq!(clamped[0], 0);
        assert_eq!(clamped[1], 0);
        assert!(clamped[2] >= clamped[0] && clamped[3] >= clamped[1]);
    }

    #[test]
    fn snap_straddling_box_moves_it_wholly_to_center_monitor() {
        // 中心在副屏（x 中心 = 2000），框左半边压在主屏 → 整框贴到副屏左边缘
        let snapped =
            snap_into_single_monitor([1800, 100, 2200, 400], &dual_monitors(), virtual_desktop());
        assert_eq!(snapped, [1920, 100, 2320, 400]);
        assert!(rect_fits_any_monitor(snapped, &dual_monitors()));
    }

    #[test]
    fn snap_box_past_right_edge_pulls_it_back_inside_secondary() {
        let snapped =
            snap_into_single_monitor([4300, 100, 4700, 400], &dual_monitors(), virtual_desktop());
        assert_eq!(snapped, [4080, 100, 4480, 400]);
        assert!(rect_fits_any_monitor(snapped, &dual_monitors()));
    }

    #[test]
    fn snap_already_inside_monitor_is_identity() {
        let rect = [300, 200, 900, 700];
        assert_eq!(
            snap_into_single_monitor(rect, &dual_monitors(), virtual_desktop()),
            rect
        );
    }

    #[test]
    fn snap_without_monitors_falls_back_to_bounds() {
        let snapped = snap_into_single_monitor([4400, 100, 4800, 400], &[], virtual_desktop());
        assert_eq!(snapped, [4080, 100, 4480, 400]);
    }

    #[test]
    fn snap_preserves_size_for_oversized_box() {
        // 框比主屏宽：只要求左上角不越界，且仍在虚拟桌面内
        let wide = [-100, 0, 5000, 500];
        let snapped = snap_into_single_monitor(wide, &dual_monitors(), (-1000, 0, 6000, 1080));
        assert_eq!(snapped[2] - snapped[0], 5100, "超大框不得被压缩");
        let bounds = (-1000, 0, 6000, 1080);
        assert!(
            snapped[0] >= bounds.0
                && snapped[1] >= bounds.1
                && snapped[2] <= bounds.0 + bounds.2
                && snapped[3] <= bounds.1 + bounds.3,
            "吸附结果必须仍在虚拟桌面内: {snapped:?}"
        );
    }

    #[test]
    fn snap_negative_secondary_monitor_keeps_box_inside_screen() {
        let monitors = [((0, 0), (1920, 1080)), ((-1920, 0), (1920, 1080))];
        let bounds = (-1920, 0, 3840, 1080);
        let snapped = snap_into_single_monitor([-2000, 100, -1600, 400], &monitors, bounds);
        assert_eq!(snapped, [-1920, 100, -1520, 400]);
        assert!(rect_fits_any_monitor(snapped, &monitors));
    }

    #[test]
    fn snap_is_idempotent_and_single_monitor_works() {
        let straddling = [1800, 100, 2200, 400];
        let once = snap_into_single_monitor(straddling, &dual_monitors(), virtual_desktop());
        let twice = snap_into_single_monitor(once, &dual_monitors(), virtual_desktop());
        assert_eq!(once, twice);

        let single = [((0, 0), (1920, 1080))];
        let snapped = snap_into_single_monitor([-100, -50, 300, 200], &single, (0, 0, 1920, 1080));
        assert_eq!(snapped, [0, 0, 400, 250]);
    }

    #[test]
    fn monitor_center_handles_extreme_coordinates_without_overflow() {
        let monitors = [((i32::MIN, i32::MIN), (i32::MAX, i32::MAX))];
        assert_eq!(
            monitor_index_for_center(
                [i32::MAX - 10, i32::MAX - 10, i32::MAX, i32::MAX],
                &monitors
            ),
            Some(0)
        );
    }

    #[test]
    fn fits_any_monitor_detects_straddle() {
        assert!(!rect_fits_any_monitor(
            [1800, 100, 2200, 400],
            &dual_monitors()
        ));
        assert!(rect_fits_any_monitor(
            [1920, 100, 2320, 400],
            &dual_monitors()
        ));
    }

    #[test]
    fn fit_into_bounds_shrinks_only_when_needed() {
        let bounds = (1920, 0, 2560, 1440);
        // 正常尺寸：只挪位置，不缩尺寸
        let small = fit_into_bounds([100, 100, 600, 500], bounds);
        assert_eq!(small, [1920, 100, 2420, 500]);
        // 比屏还宽：必须缩到屏宽（否则玻璃块比画布大 = 被切掉）
        let huge = fit_into_bounds([1000, 0, 9000, 4000], bounds);
        assert_eq!(huge[2] - huge[0], 2560, "宽度应被夹到屏宽");
        assert_eq!(huge[3] - huge[1], 1440, "高度应被夹到屏高");
        assert!(
            huge[0] >= 1920 && huge[2] <= 1920 + 2560,
            "必须整框落在副屏内"
        );
    }

    #[test]
    fn fit_into_bounds_handles_negative_origin_monitor() {
        let bounds = (-2560, 0, 2560, 1440);
        let fitted = fit_into_bounds([-100, 10, 2900, 1450], bounds);
        assert_eq!(fitted[0], -2560);
        assert_eq!(fitted[2] - fitted[0], 2560);
        assert_eq!(fitted[3] - fitted[1], 1440);
    }

    #[test]
    fn fit_into_bounds_with_invalid_bounds_is_identity() {
        let rect = [10, 20, 30, 40];
        assert_eq!(fit_into_bounds(rect, (0, 0, 0, 100)), rect);
        assert_eq!(fit_into_bounds(rect, (0, 0, 100, -5)), rect);
    }

    #[test]
    fn rect_center_rejects_degenerate_rects() {
        assert_eq!(rect_center([10, 10, 10, 20]), None);
        assert_eq!(rect_center([10, 10, 5, 5]), None);
        assert_eq!(rect_center([0, 0, 100, 100]), Some((50, 50)));
    }
}
