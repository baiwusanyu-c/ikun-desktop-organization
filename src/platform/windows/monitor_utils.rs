//! 显示器和窗口定位工具函数
//!
//! 提供显示器信息查询、DPI 获取、窗口定位、跨屏边界吸附等功能。
//! 所有函数返回物理像素坐标（虚拟桌面坐标系），不做 DPI 转换；
//! 纯算术判据全部下沉到 `box_core::monitor_geom`（可单测、0 unsafe）。

use super::ffi_imports::*;
use super::ffi_types::*;

/// 获取主屏幕分辨率（物理像素）
pub fn get_screen_resolution() -> (i32, i32) {
    let w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    (w, h)
}

/// 获取主显示器的 DPI
///
/// 使用 (0, 0) 点配合 MONITOR_DEFAULTTOPRIMARY 定位主显示器，
/// 通过 GetDpiForMonitor 查询 DPI。
pub fn get_primary_monitor_dpi() -> u32 {
    unsafe {
        let pt = POINT { x: 0, y: 0 };
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
        if hmon.is_null() {
            return 96;
        }
        // 使用 GetDpiForMonitor (Windows 8.1+) 查询 DPI
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        // MDT_EFFECTIVE_DPI = 0
        let hr = GetDpiForMonitor(hmon, 0, &mut dpi_x, &mut dpi_y);
        if hr == 0 && dpi_x > 0 {
            dpi_x
        } else {
            96
        }
    }
}

/// 获取虚拟桌面边界（包含所有显示器的联合区域）。
///
/// 返回值为主显示器坐标系下的虚拟桌面范围，用于跨显示器场景。
pub fn get_virtual_desktop_bounds() -> (i32, i32, i32, i32) {
    unsafe {
        const SM_XVIRTUALSCREEN: i32 = 76;
        const SM_YVIRTUALSCREEN: i32 = 77;
        const SM_CXVIRTUALSCREEN: i32 = 78;
        const SM_CYVIRTUALSCREEN: i32 = 79;
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let cx = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let cy = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if cx > 0 && cy > 0 {
            (x, y, cx, cy)
        } else {
            fallback_work_area()
        }
    }
}

/// 枚举所有显示器的完整矩形（虚拟桌面物理像素）。
///
/// 供跨屏边界吸附使用：主屏原点恒为 (0,0)，副屏可能在负坐标区。
/// 枚举失败时回退为「主屏幕全屏」，保证调用方永远拿到非空列表。
#[must_use]
pub fn get_monitor_rects() -> Vec<(i32, i32, i32, i32)> {
    // 拖拽热路径（WM_MOVING 可达 125Hz）每次都 EnumDisplayMonitors + 分配 Vec，
    // 违反「高频路径不做事」规范，故加 2s TTL 缓存（线程内）；显示器热插拔由
    // `invalidate_monitor_rects_cache()`（玻璃引擎 MonitorsChanged 事件）主动失效。
    const TTL_MS: u64 = 2000;
    thread_local! {
        static CACHE: std::cell::RefCell<Option<(u64, Vec<(i32, i32, i32, i32)>)>> =
            const { std::cell::RefCell::new(None) };
    }
    let now = super::glass_engine::now_ms();
    if let Some((at, rects)) = CACHE.with(|c| c.borrow().clone()) {
        if now.saturating_sub(at) < TTL_MS && !rects.is_empty() {
            return rects;
        }
    }
    let rects = enumerate_monitor_rects();
    CACHE.with(|c| *c.borrow_mut() = Some((now, rects.clone())));
    rects
}

/// 枚举全部在岗显示器：（设备名, 完整矩形）。低频对账/恢复路径专用。
///
/// 设备名作为「归属屏是否回归」的判据之一；拔插 / 拓扑变化后
/// Windows 可能复用或重排该名字，调用方必须同时校验几何（见
/// `box_core::monitor_remap::origin_matches`）。本函数**不走 TTL 缓存**，
/// 返回实时拓扑：`WM_DISPLAYCHANGE` 对账需要即时的屏集合。
/// 枚举失败返回空表，调用方按「拓扑暂不稳定」延后重试。
#[must_use]
pub fn get_monitors_named() -> Vec<(String, (i32, i32, i32, i32))> {
    unsafe extern "system" fn collect(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        let list = &mut *(data as *mut Vec<(String, (i32, i32, i32, i32))>);
        let mut info = MONITORINFOEXW {
            cb_size: std::mem::size_of::<MONITORINFOEXW>() as u32,
            rc_monitor: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rc_work: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dw_flags: 0,
            sz_device: [0u16; 32],
        };
        let ok = GetMonitorInfoW(
            hmonitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        );
        if ok != 0 {
            let r = info.rc_monitor;
            let w = r.right.saturating_sub(r.left);
            let h = r.bottom.saturating_sub(r.top);
            if w > 0 && h > 0 {
                let end = info
                    .sz_device
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(info.sz_device.len());
                let device = String::from_utf16_lossy(&info.sz_device[..end]);
                list.push((device, (r.left, r.top, w, h)));
            }
        }
        1
    }

    let mut out: Vec<(String, (i32, i32, i32, i32))> = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect),
            &mut out as *mut Vec<(String, (i32, i32, i32, i32))> as LPARAM,
        )
    };
    if ok == 0 {
        log::warn!("带设备名的显示器枚举失败，本轮对账按「拓扑暂不稳定」处理");
        return Vec::new();
    }
    out
}

/// 枚举带身份信息的在岗显示器（设备名 + 屏矩形 + DPI），对账 / 启动恢复的直接输入。
///
/// 低频路径（`WM_DISPLAYCHANGE` 对账拍、`restore_layout`），不做缓存；
/// DPI 取屏中心点的 effective DPI（`GetDpiForMonitor`），非法屏兜底 96。
#[must_use]
pub fn get_live_monitors() -> Vec<crate::box_core::monitor_remap::LiveMonitor> {
    get_monitors_named()
        .into_iter()
        .map(|(device, bounds)| {
            let (x, y, w, h) = bounds;
            let dpi = if w > 0 && h > 0 {
                dpi_at_screen_point(x.saturating_add(w / 2), y.saturating_add(h / 2))
            } else {
                96
            };
            crate::box_core::monitor_remap::LiveMonitor {
                device,
                bounds,
                dpi,
            }
        })
        .collect()
}

/// 让显示器矩形缓存立即失效（显示器集合变化 / DPI 结构调整时调用）。
pub fn invalidate_monitor_rects_cache() {
    thread_local! {
        static CACHE: std::cell::RefCell<Option<(u64, Vec<(i32, i32, i32, i32)>)>> =
            const { std::cell::RefCell::new(None) };
    }
    CACHE.with(|c| *c.borrow_mut() = None);
}

/// 真正的 EnumDisplayMonitors 枚举（不走缓存）。
#[must_use]
fn enumerate_monitor_rects() -> Vec<(i32, i32, i32, i32)> {
    unsafe extern "system" fn collect(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        let list = &mut *(data as *mut Vec<(i32, i32, i32, i32)>);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rcWork: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dwFlags: 0,
        };
        if GetMonitorInfoW(hmonitor, &mut info) != 0 {
            let r = info.rcMonitor;
            let w = r.right.saturating_sub(r.left);
            let h = r.bottom.saturating_sub(r.top);
            if w > 0 && h > 0 {
                list.push((r.left, r.top, w, h));
            }
        }
        1
    }

    let mut out: Vec<(i32, i32, i32, i32)> = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect),
            &mut out as *mut Vec<(i32, i32, i32, i32)> as LPARAM,
        )
    };
    if ok == 0 || out.is_empty() {
        let (x, y, w, h) = fallback_work_area();
        out.push((x, y, w, h));
    }
    out
}

/// 把分类框内容矩形吸附到「完整留在单块显示器内」（跨屏拖拽边界的唯一判据）。
///
/// 语义（对应 2026-11-26 用户诉求「可以在主屏副屏之间拖拽切换，但不能拖出屏幕」）：
/// - 以矩形中心选择归属屏（与液态玻璃库 `rehome_block` 的归屏判据同构）；
/// - 结果整框落在该屏内 → 跨界时不会停在「一半主屏一半副屏」的位置；
/// - 屏容不下该尺寸时，只保证左上角不越界，并再夹一次虚拟桌面边界。
///
/// 返回夹好的 `RECT`（物理像素），只改位置不改尺寸。
#[must_use]
pub fn snap_content_rect_to_single_monitor(content: &RECT) -> RECT {
    let monitors: Vec<((i32, i32), (i32, i32))> = get_monitor_rects()
        .into_iter()
        .map(|(ox, oy, w, h)| ((ox, oy), (w, h)))
        .collect();
    let rect = [content.left, content.top, content.right, content.bottom];
    let bounds = get_virtual_desktop_bounds();
    let snapped = crate::box_core::monitor_geom::snap_into_single_monitor(rect, &monitors, bounds);
    RECT {
        left: snapped[0],
        top: snapped[1],
        right: snapped[2],
        bottom: snapped[3],
    }
}

/// 兜底：把内容矩形整体塞进归属显示器（必要时**允许缩小**）。
///
/// 仅在「DPI 变大导致分类框物理尺寸超过目标屏」这种极端组合下才会真正改尺寸，
/// 否则会原样返回（位置与宽高都不动）。不做这件事的话，跨屏 DPI 过渡时
/// 玻璃块会比画布还大，用户看到的就是一块被切掉的玻璃。
#[must_use]
pub fn fit_content_rect_into_monitor(content: &RECT) -> RECT {
    let rect = [content.left, content.top, content.right, content.bottom];
    let Some(monitor) = monitor_rect_for_content_center(content) else {
        return RECT {
            left: content.left,
            top: content.top,
            right: content.right,
            bottom: content.bottom,
        };
    };
    let fitted = crate::box_core::monitor_geom::fit_into_bounds(
        rect,
        (monitor.0, monitor.1, monitor.2, monitor.3),
    );
    RECT {
        left: fitted[0],
        top: fitted[1],
        right: fitted[2],
        bottom: fitted[3],
    }
}

/// 内容矩形中心所属（或最近）的显示器矩形。
#[must_use]
pub fn monitor_rect_for_content_center(content: &RECT) -> Option<(i32, i32, i32, i32)> {
    let monitors = get_monitor_rects();
    let mapped: Vec<((i32, i32), (i32, i32))> =
        monitors.iter().map(|m| ((m.0, m.1), (m.2, m.3))).collect();
    let index = crate::box_core::monitor_geom::monitor_index_for_center(
        [content.left, content.top, content.right, content.bottom],
        &mapped,
    )?;
    monitors.get(index).copied()
}

/// 外框矩形 → 吸附后的外框矩形（拖拽热路径的一次性入口）。
///
/// 只做「位置」吸附：可见内容必须完整留在单块显示器内，外框随之平移、尺寸恒不变；
/// 内容已合法时原样返回（幂等，因此可与 `WM_WINDOWPOSCHANGED` 的自愈吸附共存）。
/// 返回 `(吸附后的外框, 是否发生位移)`，调用方据此决定是否重定位窗口。
#[must_use]
pub fn snap_outer_rect_for_drag(outer: &RECT, padding: i32) -> (RECT, bool) {
    let content = RECT {
        left: outer.left.saturating_add(padding),
        top: outer.top.saturating_add(padding),
        right: outer.right.saturating_sub(padding),
        bottom: outer.bottom.saturating_sub(padding),
    };
    let snapped = snap_content_rect_to_single_monitor(&content);
    if snapped.left == content.left && snapped.top == content.top {
        return (
            RECT {
                left: outer.left,
                top: outer.top,
                right: outer.right,
                bottom: outer.bottom,
            },
            false,
        );
    }
    (
        RECT {
            left: snapped.left.saturating_sub(padding),
            top: snapped.top.saturating_sub(padding),
            right: snapped.right.saturating_add(padding),
            bottom: snapped.bottom.saturating_add(padding),
        },
        true,
    )
}

/// 把分类框内容矩形夹进虚拟桌面（只防「整块跑到所有屏之外」，不改尺寸）。
///
/// 用于库会话跟随：会话期间块矩形是几何真源，宿主只能做这一层兜底，
/// 不得按自己的判据把块拽回单屏内（否则会与库会话锚点争抢，见契约文档）。
#[must_use]
pub fn clamp_content_rect_to_desktop(content: &RECT) -> RECT {
    let rect = [content.left, content.top, content.right, content.bottom];
    let bounds = get_virtual_desktop_bounds();
    let clamped = crate::box_core::monitor_geom::clamp_to_bounds(rect, bounds);
    RECT {
        left: clamped[0],
        top: clamped[1],
        right: clamped[2],
        bottom: clamped[3],
    }
}

/// 屏幕点当前落在第几块显示器（虚拟桌面外返回 `None`）。
///
/// 用于会话收尾的一致性判定：块（视觉真源）与内容窗口分居两屏时必须纠偏。
#[must_use]
pub fn monitor_index_of_point(x: i32, y: i32) -> Option<usize> {
    get_monitor_rects().iter().position(|(ox, oy, w, h)| {
        x >= *ox && x < ox.saturating_add(*w) && y >= *oy && y < oy.saturating_add(*h)
    })
}

/// 读取系统拖拽启动阈值（取水平/垂直较大者，非法值返回 0 交由调用方兜底）。
#[must_use]
pub fn system_drag_threshold_px() -> i32 {
    let cx = unsafe { GetSystemMetrics(SM_CXDRAG) };
    let cy = unsafe { GetSystemMetrics(SM_CYDRAG) };
    cx.max(cy).max(0)
}

/// 按窗口 DPI 折算后的拖拽启动阈值（物理像素）。
#[must_use]
pub fn drag_threshold_for_dpi(dpi: u32) -> i32 {
    crate::box_core::drag_logic::drag_threshold_physical_px(system_drag_threshold_px(), dpi)
}

/// 回退：主屏幕全屏范围
fn fallback_work_area() -> (i32, i32, i32, i32) {
    let (w, h) = get_screen_resolution();
    (0, 0, w, h)
}

/// 获取指定屏幕坐标处的 DPI 值（用于多显示器不同 DPI 场景）
pub fn dpi_at_screen_point(screen_x: i32, screen_y: i32) -> u32 {
    use std::ffi::c_void;

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    type HMONITOR = *mut c_void;

    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    const MDT_EFFECTIVE_DPI: u32 = 0;

    extern "system" {
        fn MonitorFromPoint(pt: POINT, dwFlags: u32) -> HMONITOR;
        fn GetDpiForMonitor(
            hmonitor: HMONITOR,
            dpiType: u32,
            dpiX: *mut u32,
            dpiY: *mut u32,
        ) -> i32;
    }

    let pt = POINT {
        x: screen_x,
        y: screen_y,
    };
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return 96;
    }
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } == 0 {
        dpi_x
    } else {
        96
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_screen_resolution_returns_positive() {
        let (w, h) = get_screen_resolution();
        assert!(w > 0, "屏幕宽度应为正数");
        assert!(h > 0, "屏幕高度应为正数");
    }

    #[test]
    fn test_get_primary_monitor_dpi_returns_valid() {
        let dpi = get_primary_monitor_dpi();
        assert!(dpi >= 96, "DPI 应至少为 96");
        assert!(dpi <= 400, "DPI 不应超过 400");
    }

    #[test]
    fn test_fallback_work_area() {
        let (x, y, w, h) = fallback_work_area();
        assert_eq!(x, 0);
        assert_eq!(y, 0);
        assert!(w > 0);
        assert!(h > 0);
    }

    #[test]
    fn test_get_virtual_desktop_bounds_valid() {
        let (_x, _y, w, h) = get_virtual_desktop_bounds();
        assert!(w > 0, "虚拟桌面宽度应为正数");
        assert!(h > 0, "虚拟桌面高度应为正数");
    }

    #[test]
    fn monitor_rects_is_never_empty() {
        let rects = get_monitor_rects();
        assert!(!rects.is_empty(), "显示器枚举必须有兜底结果");
        assert!(
            rects.iter().all(|(_, _, w, h)| *w > 0 && *h > 0),
            "返回的显示器宽高必须为正"
        );
    }

    #[test]
    fn snap_rect_never_changes_size() {
        let input = RECT {
            left: 120,
            top: 80,
            right: 620,
            bottom: 480,
        };
        let out = snap_content_rect_to_single_monitor(&input);
        assert_eq!(out.right - out.left, 500, "吸附只改位置不改宽度");
        assert_eq!(out.bottom - out.top, 400, "吸附只改位置不改高度");
    }

    #[test]
    fn clamp_rect_keeps_rect_inside_desktop() {
        let far_out = RECT {
            left: i32::MAX / 2,
            top: i32::MAX / 2,
            right: i32::MAX / 2 + 300,
            bottom: i32::MAX / 2 + 200,
        };
        let out = clamp_content_rect_to_desktop(&far_out);
        let (bx, by, bw, bh) = get_virtual_desktop_bounds();
        assert!(out.left >= bx && out.top >= by, "必须夹回虚拟桌面内");
        assert!(out.right <= bx + bw && out.bottom <= by + bh);
        assert_eq!(out.right - out.left, 300);
    }

    #[test]
    fn snap_outer_rect_is_idempotent_and_size_preserving() {
        let outer = RECT {
            left: 40,
            top: 30,
            right: 640,
            bottom: 530,
        };
        let once = snap_outer_rect_for_drag(&outer, 0);
        let twice = snap_outer_rect_for_drag(&once.0, 0);
        assert_eq!(once.0.left, twice.0.left, "吸附必须幂等");
        assert_eq!(once.0.top, twice.0.top, "吸附必须幂等");
        assert_eq!(once.0.right - once.0.left, 600, "只改位置不改宽");
        assert_eq!(once.0.bottom - once.0.top, 500, "只改位置不改高");
    }

    #[test]
    fn monitors_named_is_non_empty_with_valid_rects() {
        let list = get_monitors_named();
        assert!(!list.is_empty(), "在岗显示器枚举不应为空");
        assert!(
            list.iter().all(|(_, (_, _, w, h))| *w > 0 && *h > 0),
            "设备名枚举返回的显示器宽高必须为正"
        );
        // 与无设备名枚举口径一致（同一拓扑的屏数相同）
        assert_eq!(
            list.len(),
            get_monitor_rects().len(),
            "两种枚举路径的屏数应一致"
        );
    }

    #[test]
    fn monitor_rects_cache_is_stable() {
        let a = get_monitor_rects();
        let b = get_monitor_rects();
        assert_eq!(a, b, "缓存命中不得改变结果");
        invalidate_monitor_rects_cache();
        assert_eq!(a, get_monitor_rects(), "失效后重新枚举应得到同一集合");
    }

    #[test]
    fn system_drag_threshold_is_sane() {
        let px = system_drag_threshold_px();
        assert!((0..=64).contains(&px), "系统拖拽阈值异常: {px}");
        // 即便系统值取不到，DPI 折算也必须落在安全区间
        let t = crate::box_core::drag_logic::drag_threshold_physical_px(px, 192);
        assert!((5..=16).contains(&t), "DPI 折算阈值越界: {t}");
    }
}
