//! 分类框遮挡判定。
//!
//! 分类框是 `WS_EX_LAYERED` 桌面窗口，Z-order 被压在所有普通应用窗口之下
//! （见 `wnd_proc` 的 `WM_WINDOWPOSCHANGING` → `HWND_BOTTOM`）。用户把某个
//! 应用全屏或最大化、完全盖住分类框时，玻璃画得再精细也没有人看得见，
//! 但每一帧仍要跑完整条 DXGI 捕获 → D2D 模糊 → D3D 着色 → GDI 回读管线。
//!
//! 本模块只回答一个问题：**这个分类框现在是否被完全遮住了**。
//!
//! 判定刻意保守——只有确认被完全覆盖才返回 `true`，任何不确定情形
//! （枚举没走到自己、候选窗口过多、覆盖递归超深、API 失败）一律返回
//! `false`。宁可多画几帧，也不能让分类框冻在旧画面上。

use std::time::Duration;

use super::ffi_imports::*;
use super::ffi_types::*;

/// 遮挡判定的最小间隔。
///
/// 窗口枚举要遍历所有顶层窗口并逐个查可见性/进程/DWM 状态，不便宜；
/// 而遮挡状态变化很慢（用户切个全屏应用而已），500ms 足够灵敏。
pub(crate) const OCCLUSION_CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// `DwmGetWindowAttribute` 的 `DWMWA_CLOAKED` 属性编号。
const DWMWA_CLOAKED: u32 = 14;

/// 参与覆盖判定的候选窗口上限，超出就直接判"没被遮住"。
///
/// 正常桌面在分类框之上的窗口也就几个到十几个；几百个说明环境异常
/// （或者枚举到了奇怪的东西），此时不值得继续算。
const MAX_CANDIDATE_OCCLUDERS: usize = 64;

/// 覆盖递归的深度上限。
///
/// 真实场景里"一个全屏/最大化窗口盖住分类框"是绝对主流；靠多个窗口
/// 拼出完全遮挡既罕见又难以保证判定可靠，超深就保守放弃。
const MAX_COVER_DEPTH: usize = 8;

#[link(name = "dwmapi")]
extern "system" {
    /// 读取窗口的 DWM 属性；这里只用来查"是否被 cloak"。
    fn DwmGetWindowAttribute(
        hwnd: HWND,
        attribute: u32,
        value: *mut std::ffi::c_void,
        size: u32,
    ) -> i32;
}

/// 枚举过程中的可变状态，通过 `EnumWindows` 的 lParam 传递。
struct CollectState {
    /// 待判定的分类框窗口
    self_hwnd: HWND,
    /// 本进程 ID：自己的窗口一律不算遮挡物
    self_pid: u32,
    /// 分类框的屏幕矩形
    target: RECT,
    /// 已收集到的、位于分类框之上且可能不透明的窗口矩形（与 target 的交集）
    covers: Vec<RECT>,
    /// 枚举是否已经走到（并越过）分类框自己
    reached_self: bool,
    /// 候选窗口太多，放弃判定
    gave_up: bool,
}

/// `EnumWindows` 按 Z-order 自顶向下枚举顶层窗口。
///
/// 走到分类框自己就停：它下面的窗口在 Z 序上盖不住它，不必再看。
unsafe extern "system" fn collect_occluders(hwnd: HWND, lparam: LPARAM) -> i32 {
    let state = &mut *(lparam as *mut CollectState);

    if hwnd == state.self_hwnd {
        state.reached_self = true;
        return 0; // 停止枚举
    }
    if state.covers.len() >= MAX_CANDIDATE_OCCLUDERS {
        state.gave_up = true;
        return 0;
    }

    let target = copy_rect(&state.target);
    if let Some(overlap) = occluder_overlap(hwnd, state.self_pid, &target) {
        state.covers.push(overlap);
    }
    1 // 继续枚举
}

/// 若 `hwnd` 是一个可能不透明的遮挡窗口，返回它与 `target` 的交集。
///
/// 排除掉的几类：不可见、已最小化、本进程自己的窗口、被 DWM cloak 的窗口
/// （UWP 挂起 / 虚拟桌面切换中，`IsWindowVisible` 仍为真但实际没画出来）、
/// 以及鼠标穿透的分层窗口（各种悬浮小工具、输入法候选条）。
unsafe fn occluder_overlap(hwnd: HWND, self_pid: u32, target: &RECT) -> Option<RECT> {
    if IsWindowVisible(hwnd) == 0 {
        return None;
    }
    if IsIconic(hwnd) != 0 {
        return None;
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 || pid == self_pid {
        return None;
    }

    let mut cloaked: u32 = 0;
    let hr = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut u32 as *mut std::ffi::c_void,
        std::mem::size_of::<u32>() as u32,
    );
    if hr == 0 && cloaked != 0 {
        return None;
    }

    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if ex_style & WS_EX_TRANSPARENT != 0 && ex_style & WS_EX_LAYERED != 0 {
        return None;
    }

    let mut rect = empty_rect();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return None;
    }
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return None;
    }
    intersect(&rect, target)
}

fn empty_rect() -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }
}

/// `RECT` 没有 `Copy`，需要显式逐字段拷贝。
fn copy_rect(rect: &RECT) -> RECT {
    RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn intersect(a: &RECT, b: &RECT) -> Option<RECT> {
    let left = a.left.max(b.left);
    let top = a.top.max(b.top);
    let right = a.right.min(b.right);
    let bottom = a.bottom.min(b.bottom);
    (right > left && bottom > top).then_some(RECT {
        left,
        top,
        right,
        bottom,
    })
}

/// 虚拟屏幕的四个 `GetSystemMetrics` 索引。
const SM_XVIRTUALSCREEN: i32 = 76;
const SM_YVIRTUALSCREEN: i32 = 77;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;

/// 把矩形夹进边界内；完全落在边界外时返回一个空矩形（right<=left）。
///
/// 纯函数，便于单测：夹错方向会让「看得见」的部分被当成看不见，
/// 分类框就会冻在旧画面上，而这种错误不会有任何报错。
fn clamp_rect_to(rect: &RECT, bounds: &RECT) -> RECT {
    let left = rect.left.max(bounds.left);
    let top = rect.top.max(bounds.top);
    let right = rect.right.min(bounds.right);
    let bottom = rect.bottom.min(bounds.bottom);
    RECT {
        left,
        top,
        // 交叉时收敛成空矩形，而不是反过来变成负尺寸
        right: right.max(left),
        bottom: bottom.max(top),
    }
}

/// 把分类框外框夹到**虚拟屏幕**（所有显示器的外接矩形）范围内。
///
/// # 为什么必须先夹一次，否则遮挡判定形同虚设
///
/// 分类框可以被拖到部分伸出屏幕外的位置。实测就有 5 个框的右边缘在
/// x=5154，而所在副屏的右边界是 5040 —— 有 114 物理像素在屏幕外。
/// 此时即便别的应用把**屏幕上看得见的那部分**盖得严严实实，
/// 覆盖率也只有 71%~84%：剩下那块是屏幕外根本不存在的区域，
/// 永远不可能被任何窗口覆盖。`fully_covered` 于是永远返回 `false`，
/// `遮挡状态切换` 一条都不会打，分类框照旧每帧跑完整条
/// DXGI 取帧 → D2D 模糊 → D3D 着色 → GDI 回读 → 提交管线。
/// 实测这条管线每渲染一次就在显卡用户态驱动里留下约 22KB 不还的
/// 已提交内存（Rust 堆全程恒定 6.4MB），27.7 次/秒就是 **36MB/分**。
///
/// # 为什么夹到虚拟屏幕是安全的
///
/// 虚拟屏幕**之外**的像素一定不落在任何显示器上，夹掉它不可能把
/// 「其实看得见」的部分误判成看不见 —— 判定只会变保守的反面，
/// 也就是更容易判成遮挡，而被夹掉的区域本来就没人看得见。
///
/// 反过来不成立：虚拟屏幕的外接矩形里可能包含显示器之间的空隙，
/// 那部分仍会被当作可见。所以这个夹取是**偏保守**的，只会少省一点，
/// 不会误判。要精确到显示器并集得枚举所有 monitor，收益不抵复杂度。
fn clamp_to_virtual_screen(target: &RECT) -> RECT {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if vw <= 0 || vh <= 0 {
        // 拿不到虚拟屏幕尺寸就不夹，保持原有的保守行为
        return copy_rect(target);
    }
    let bounds = RECT {
        left: vx,
        top: vy,
        right: vx + vw,
        bottom: vy + vh,
    };
    clamp_rect_to(target, &bounds)
}

fn same_rect(a: &RECT, b: &RECT) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

/// `target` 减去 `inter` 之后剩下的部分，最多 4 条边角。
fn remainder_pieces(target: &RECT, inter: &RECT) -> Vec<RECT> {
    let mut pieces = Vec::with_capacity(4);
    if inter.left > target.left {
        pieces.push(RECT {
            left: target.left,
            top: target.top,
            right: inter.left,
            bottom: target.bottom,
        });
    }
    if inter.right < target.right {
        pieces.push(RECT {
            left: inter.right,
            top: target.top,
            right: target.right,
            bottom: target.bottom,
        });
    }
    let mid_left = inter.left.max(target.left);
    let mid_right = inter.right.min(target.right);
    if inter.top > target.top {
        pieces.push(RECT {
            left: mid_left,
            top: target.top,
            right: mid_right,
            bottom: inter.top,
        });
    }
    if inter.bottom < target.bottom {
        pieces.push(RECT {
            left: mid_left,
            top: inter.bottom,
            right: mid_right,
            bottom: target.bottom,
        });
    }
    pieces
}

/// `target` 是否被 `covers` 完全覆盖。
///
/// 递归拆分：取第一个与 `target` 相交的矩形切掉一块，剩下的边角必须由
/// 后续矩形继续覆盖。每层至少消耗一个矩形，因此深度天然受 `covers` 长度限制，
/// 再用 `MAX_COVER_DEPTH` 兜底防止极端组合下指数展开。
fn fully_covered(target: &RECT, covers: &[RECT], depth: usize) -> bool {
    if depth > MAX_COVER_DEPTH {
        return false;
    }
    let Some(index) = covers
        .iter()
        .position(|cover| intersect(cover, target).is_some())
    else {
        return false;
    };
    let Some(overlap) = intersect(&covers[index], target) else {
        return false;
    };
    if same_rect(&overlap, target) {
        return true;
    }
    let rest = &covers[index + 1..];
    remainder_pieces(target, &overlap)
        .iter()
        .all(|piece| fully_covered(piece, rest, depth + 1))
}

/// 判断 `hwnd` 这个分类框是否被其他应用的窗口完全遮住。
///
/// `target` 传分类框的**完整外框矩形**（含外投影留白），而不是内容矩形：
/// 只要有任意一部分还露在外面，就该继续渲染。
pub(crate) fn is_window_occluded(hwnd: HWND, target: &RECT) -> bool {
    if target.right <= target.left || target.bottom <= target.top {
        return false;
    }

    // 只在「屏幕上真的看得见的那部分」上做覆盖判定，理由见
    // clamp_to_virtual_screen 的说明。整框都在屏幕外时一个像素都
    // 看不见，等同于被完全遮挡。
    let visible = clamp_to_virtual_screen(target);
    if visible.right <= visible.left || visible.bottom <= visible.top {
        return true;
    }
    // 之后一律用夹取后的可见矩形：中心点快路径、遮挡物交集、覆盖判定
    let target = &visible;

    // 快路径：中心点上最顶层的窗口就是自己 → 什么都没盖住，
    // 不用枚举全部顶层窗口。
    //
    // 分类框被压在 Z-order 最底（HWND_BOTTOM），一旦被别的应用盖住，
    // WindowFromPoint 就会返回那个应用的窗口。而「没被遮挡」是
    // 绝大多数时间的常态，没有这个快路径的话每次判定都要把所有
    // 顶层窗口走一遍并逐个查进程/DWM/样式，实测能吃掉
    // 百分之几的 CPU。快路径不命中时无条件回退到完整枚举，
    // 不会漏判。
    let center = POINT {
        x: target.left + (target.right - target.left) / 2,
        y: target.top + (target.bottom - target.top) / 2,
    };
    if unsafe { WindowFromPoint(center) } == hwnd {
        return false;
    }

    let mut state = CollectState {
        self_hwnd: hwnd,
        self_pid: unsafe { GetCurrentProcessId() },
        target: copy_rect(target),
        covers: Vec::new(),
        reached_self: false,
        gave_up: false,
    };

    unsafe {
        EnumWindows(
            Some(collect_occluders),
            &mut state as *mut CollectState as LPARAM,
        );
    }

    // 没枚举到自己：窗口可能正在销毁，或者 Z 序状态异常，不敢下结论
    if !state.reached_self || state.gave_up {
        return false;
    }
    fully_covered(target, &state.covers, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    /// 夹取必须只砍掉屏幕外的部分，屏幕内的一个像素都不能少。
    ///
    /// 砍多了 = 把看得见的区域当成看不见 = 分类框冻在旧画面上，
    /// 这是本模块最危险的错误方向，必须有测试钉死。
    #[test]
    fn clamp_rect_keeps_every_visible_pixel() {
        let bounds = rect(0, 0, 5040, 2080);
        // 完全在内 → 原样返回
        assert!(same_rect(
            &clamp_rect_to(&rect(100, 100, 400, 400), &bounds),
            &rect(100, 100, 400, 400)
        ));
        // 右边越界 → 只把右边夹到边界，其余不动
        assert!(
            same_rect(
                &clamp_rect_to(&rect(4560, 365, 5154, 865), &bounds),
                &rect(4560, 365, 5040, 865)
            ),
            "实测场景：框右边缘 5154 超出副屏右边界 5040"
        );
        // 左边/上边越界 → 夹到边界
        assert!(same_rect(
            &clamp_rect_to(&rect(-200, -300, 400, 400), &bounds),
            &rect(0, 0, 400, 400)
        ));
        // 完全在外 → 空矩形，且不能出现 right < left 的负尺寸
        let outside = clamp_rect_to(&rect(6000, 3000, 6400, 3400), &bounds);
        assert!(
            outside.right <= outside.left && outside.bottom <= outside.top,
            "屏幕外必须夹成空矩形，实际 ({},{})-({},{})",
            outside.left,
            outside.top,
            outside.right,
            outside.bottom
        );
        assert!(
            outside.right >= outside.left && outside.bottom >= outside.top,
            "不允许出现负尺寸"
        );
    }

    /// 副屏在虚拟屏幕坐标系里原点不是 (0,0)，夹取必须按虚拟屏幕原点算，
    /// 不能假设从 0 开始。
    #[test]
    fn clamp_rect_handles_offset_virtual_screen() {
        // 双屏：主屏 (0,0)-(3120,2080)，副屏 (3120,0)-(5040,1080)
        let bounds = rect(-1920, 0, 5040, 2080);
        assert!(
            same_rect(
                &clamp_rect_to(&rect(-2000, 100, -100, 300), &bounds),
                &rect(-1920, 100, -100, 300)
            ),
            "负坐标侧越界也要夹到虚拟屏幕左边界"
        );
        assert!(same_rect(
            &clamp_rect_to(&rect(4560, 365, 5154, 865), &bounds),
            &rect(4560, 365, 5040, 865)
        ));
    }

    /// 端到端：框有 114px 伸出屏幕外、可见部分被一个窗口完全盖住时，
    /// 必须判成遮挡。这正是"用户全程盖着分类框、内存却照样暴涨"的现场。
    #[test]
    fn offscreen_sliver_does_not_block_occlusion() {
        let box_rect = rect(4560, 365, 5154, 865); // 右边缘超出屏幕 114px
        let visible = clamp_to_virtual_screen(&box_rect);
        // 本机虚拟屏幕宽 5040；若测试机不同，这里退化成"至少夹掉了一部分或全都在屏内"
        if visible.right > visible.left {
            // 一个窗口把可见部分盖满（RECT 不是 Copy，显式复制一份进 covers）
            let mut covers = vec![copy_rect(&visible)];
            assert!(
                fully_covered(&visible, &covers, 0),
                "可见部分被盖满就必须判成遮挡，visible=({},{})-({},{})",
                visible.left,
                visible.top,
                visible.right,
                visible.bottom
            );
            // 而用未夹取的原始外框判定必然失败 —— 这就是修复前的 bug
            covers = vec![rect(4560, 365, 5040, 865)];
            assert!(
                !fully_covered(&box_rect, &covers, 0),
                "不夹取时屏幕外那条永远盖不住，复现修复前的误判"
            );
        }
    }

    #[test]
    fn single_fullscreen_window_covers_the_box() {
        let target = rect(100, 100, 700, 600);
        // 典型场景：一个全屏/最大化窗口盖住整个分类框
        assert!(fully_covered(&target, &[rect(0, 0, 2560, 1440)], 0));
        // 刚好贴合边界也算完全覆盖
        assert!(fully_covered(&target, &[rect(100, 100, 700, 600)], 0));
    }

    #[test]
    fn partial_overlap_is_not_occlusion() {
        let target = rect(100, 100, 700, 600);
        // 只盖住左半边
        assert!(!fully_covered(&target, &[rect(0, 0, 400, 1440)], 0));
        // 只盖住一个角
        assert!(!fully_covered(
            &target,
            &[rect(0, 0, 200, 200), rect(600, 500, 2560, 1440)],
            0
        ));
        // 完全不相交（另一个显示器上的窗口）
        assert!(!fully_covered(&target, &[rect(3000, 0, 5040, 1080)], 0));
        assert!(!fully_covered(&target, &[], 0));
    }

    #[test]
    fn two_windows_can_jointly_cover_the_box() {
        let target = rect(100, 100, 700, 600);
        // 左右两半各一个窗口
        assert!(fully_covered(
            &target,
            &[rect(0, 0, 400, 1440), rect(400, 0, 2560, 1440)],
            0
        ));
        // 上下两半
        assert!(fully_covered(
            &target,
            &[rect(0, 0, 2560, 350), rect(0, 350, 2560, 1440)],
            0
        ));
    }

    #[test]
    fn coverage_order_does_not_matter() {
        let target = rect(100, 100, 700, 600);
        // RECT 没有 Copy，每次都现场构造
        let left = || rect(0, 0, 400, 1440);
        let right = || rect(400, 0, 2560, 1440);
        assert!(fully_covered(&target, &[left(), right()], 0));
        assert!(fully_covered(&target, &[right(), left()], 0));
        // 中间夹一个无关窗口也不影响
        assert!(fully_covered(
            &target,
            &[left(), rect(5000, 5000, 5100, 5100), right()],
            0
        ));
    }

    #[test]
    fn intersect_and_remainder_are_consistent() {
        let target = rect(100, 100, 700, 600);
        let inter = rect(300, 200, 500, 400);
        let overlap = intersect(&inter, &target).expect("相交");
        assert!(same_rect(&overlap, &inter));

        // 四块边角加上交集，面积必须等于原矩形
        let pieces = remainder_pieces(&target, &inter);
        assert_eq!(pieces.len(), 4);
        let area = |r: &RECT| (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
        let total: i64 = pieces.iter().map(area).sum::<i64>() + area(&inter);
        assert_eq!(total, area(&target));
        // 每块都不得与交集重叠
        for piece in &pieces {
            assert!(intersect(piece, &inter).is_none(), "边角与交集重叠了");
        }
    }

    #[test]
    fn degenerate_targets_are_never_occluded() {
        // is_window_occluded 对空矩形直接返回 false，不触发窗口枚举
        assert!(!is_window_occluded(std::ptr::null_mut(), &rect(0, 0, 0, 0)));
        assert!(!is_window_occluded(
            std::ptr::null_mut(),
            &rect(10, 10, 5, 5)
        ));
    }

    #[test]
    fn recursion_is_depth_bounded() {
        let target = rect(0, 0, 1000, 1000);
        // 一堆细条：即使能拼满，也必须在深度上限内安全返回
        let strips: Vec<RECT> = (0..60)
            .map(|i| rect(0, i * 17, 1000, i * 17 + 17))
            .collect();
        let _ = fully_covered(&target, &strips, 0);
        assert!(MAX_COVER_DEPTH < strips.len());
    }
}
