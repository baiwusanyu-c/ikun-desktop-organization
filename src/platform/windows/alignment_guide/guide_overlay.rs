//! 对齐参考线 Overlay 模块
//!
//! 两条硬性设计（2026-11-26 改造，对应「参考线变黑」与「跨屏对齐」两个问题）：
//!
//! 1. **每块显示器一个分层窗口**，线段按屏裁剪后再画。
//!    旧实现用一个覆盖整个虚拟桌面的窗口：进程是 Per-Monitor-V2，一个窗口
//!    同时压在 200% 主屏与 100% 副屏上时，DWM 只能按其中一块屏的 DPI 呈现位图，
//!    另一块屏上的线条会被整体拉伸/模糊、位置偏移（跨屏对齐必然踩到）。
//!    现在每个 overlay 完整位于自己的屏内，坐标即物理像素，跨屏线表现为
//!    两段共线线段，视觉连续且无 DPI 歧义。
//! 2. **双色描边**（暗色外描边 + 亮色内芯），不再采样桌面。
//!    旧的「按桌面亮度二选色」读的是 `GetPixel(GetDC(NULL))` = **合成后的整屏**
//!    （包含当时打开的所有窗口），而且只在首次拖拽时后台采一次、结果进程级
//!    终身缓存。实测：主屏上开着白色页面时 avg_luminance=0.93 -> 选中
//!    `rgb(30,40,60)`，视觉就是纯黑（11-25 同一份代码 avg=0.486 -> 白色，
//!    所以用户感觉「突然变黑」）。历史回退点：commit 05e3135 曾把采样移到
//!    启动时，20926b5 删除 gpui_app 时把那个调用一起删了，于是退化成
//!    「第一次拖拽时屏幕上有什么就决定线色」。双色叠加对任何背景都有对比度。

use std::ffi::c_void;
use std::sync::Mutex;

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1Factory};
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;
use windows_numerics::Vector2;

use super::alignment_detect::AlignmentResult;
use crate::platform::windows::ffi_imports::*;
use crate::platform::windows::ffi_types::*;
use crate::platform::windows::monitor_utils;
use crate::platform::windows::render_devices;

/// 参考线内芯（亮）：高对比白，深色桌面上靠它可见
const GUIDE_CORE_COLOR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.95,
};

/// 参考线外描边（暗）：半透明黑，浅色桌面/白色窗口上靠它可见
const GUIDE_HALO_COLOR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.55,
};

/// 内芯线宽（物理像素）
const GUIDE_LINE_CORE_WIDTH: f32 = 2.0;
/// 外描边线宽（物理像素）：比芯宽 3px，两侧各留约 1.5px 暗边
const GUIDE_LINE_HALO_WIDTH: f32 = 5.0;

/// Overlay 窗口的渲染资源（每块屏一套，避免共享 target 的并发/尺寸歧义）
struct GuideOverlayData {
    /// D2D Factory
    d2d_factory: Option<ID2D1Factory>,
    /// DC RenderTarget
    d2d_target: Option<ID2D1DCRenderTarget>,
}

unsafe impl Send for GuideOverlayData {}

/// 一块屏上的 overlay：窗口句柄 + 该屏的屏幕坐标矩形 (x, y, w, h)
struct MonitorOverlay {
    hwnd: HWND,
    monitor: (i32, i32, i32, i32),
}

unsafe impl Send for MonitorOverlay {}

/// 一条已裁到某块屏内的轴对齐线段（监视器本地坐标，int 物理像素）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuideSegment {
    /// true = 水平线（y 固定），false = 垂直线（x 固定）
    pub horizontal: bool,
    /// 固定轴坐标：水平线是 y，垂直线是 x
    pub at: i32,
    /// 沿线方向的起点
    pub from: i32,
    /// 沿线方向的终点（保证 from <= to）
    pub to: i32,
}

impl GuideSegment {
    /// 平移到 overlay 绘制坐标（减去包围盒原点）
    #[must_use]
    fn translated(self, dx: i32, dy: i32) -> Self {
        if self.horizontal {
            Self {
                horizontal: true,
                at: self.at - dy,
                from: self.from - dx,
                to: self.to - dx,
            }
        } else {
            Self {
                horizontal: false,
                at: self.at - dx,
                from: self.from - dy,
                to: self.to - dy,
            }
        }
    }

    /// 线段两个端点（D2D 用）
    #[must_use]
    fn endpoints(&self) -> (Vector2, Vector2) {
        let at = self.at as f32;
        if self.horizontal {
            (
                Vector2 {
                    X: self.from as f32,
                    Y: at,
                },
                Vector2 {
                    X: self.to as f32,
                    Y: at,
                },
            )
        } else {
            (
                Vector2 {
                    X: at,
                    Y: self.from as f32,
                },
                Vector2 {
                    X: at,
                    Y: self.to as f32,
                },
            )
        }
    }
}

static GUIDE_OVERLAY: Mutex<Vec<MonitorOverlay>> = Mutex::new(Vec::new());
static LAST_HIT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static LAST_RENDER: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

static CLASS_NAME: &[u16] = &[
    73, 107, 117, 110, 71, 117, 105, 100, 101, 79, 118, 101, 114, 108, 97, 121, 0,
]; // "IkunGuideOverlay"

/// 会话开始/结束时重置节流与宽限期状态
pub(crate) fn reset_guide_render_state() {
    if let Ok(mut last_hit) = LAST_HIT.lock() {
        *last_hit = None;
    }
    if let Ok(mut last_render) = LAST_RENDER.lock() {
        *last_render = None;
    }
}

/// 取 Overlay 窗口的用户数据
fn get_guide_data(hwnd: HWND) -> Option<&'static mut GuideOverlayData> {
    let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    if data_ptr == 0 {
        return None;
    }
    Some(unsafe { &mut *(data_ptr as *mut GuideOverlayData) })
}

/// Overlay 窗口过程：只处理消息循环所需的最小集合
unsafe extern "system" fn guide_wnd_proc(
    hwnd: HWND,
    msg: u32,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }
        WM_DESTROY => {
            let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
            if data_ptr != 0 {
                let _ = Box::from_raw(data_ptr as *mut GuideOverlayData);
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, _wparam, _lparam) },
    }
}

/// 确保 Overlay 窗口类已注册
fn ensure_class_registered() -> Result<HINSTANCE, &'static str> {
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    if hinstance.is_null() {
        return Err("GetModuleHandleW failed");
    }

    let mut registered = CLASS_REGISTERED.lock().map_err(|_| "Mutex poisoned")?;
    if *registered {
        return Ok(hinstance);
    }

    let wc = WNDCLASSEXW {
        cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfn_wnd_proc: Some(guide_wnd_proc),
        cb_cls_extra: 0,
        cb_wnd_extra: 0,
        h_instance: hinstance,
        h_icon: std::ptr::null_mut(),
        h_cursor: std::ptr::null_mut(),
        hbr_background: std::ptr::null_mut(),
        lpsz_menu_name: std::ptr::null(),
        lpsz_class_name: CLASS_NAME.as_ptr(),
        h_icon_sm: std::ptr::null_mut(),
    };

    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        return Err("RegisterClassExW failed");
    }

    *registered = true;
    log::info!("Guide overlay window class registered");
    Ok(hinstance)
}

/// 初始化 D2D 设备（工厂取渲染线程共享实例，见 `render_devices`）
fn init_d2d_devices(data: &mut GuideOverlayData) {
    if data.d2d_factory.is_none() {
        data.d2d_factory = render_devices::d2d_factory();
    }
    if data.d2d_target.is_none() {
        // dpi=0 => 实测渲染目标 dpiX/dpiY 恒为 96，整套绘制把坐标当物理像素用
        data.d2d_target = render_devices::create_dc_render_target(0.0);
    }
}

/// 创建/重建每屏一个的透明分层 Overlay 窗口（鼠标穿透、不抢焦点、不进任务栏）
///
/// 显示器布局变了（热插拔 / 改分辨率）时自动销毁重建，否则新屏上永远没有参考线。
pub fn create_guide_overlay() -> Result<(), &'static str> {
    let monitors = monitor_utils::get_monitor_rects();
    if monitors.is_empty() {
        return Err("没有可用显示器，跳过参考线 overlay");
    }

    let same_layout = GUIDE_OVERLAY
        .lock()
        .map(|list| !list.is_empty() && list.iter().map(|o| o.monitor).eq(monitors.iter().copied()))
        .unwrap_or(false);
    if same_layout {
        return Ok(());
    }

    let hinstance = ensure_class_registered()?;

    // 布局变了先清掉旧窗口，避免留下指向已拔掉屏幕的死窗口
    destroy_guide_overlay();

    for &monitor in &monitors {
        let (x, y, width, height) = monitor;
        if width <= 0 || height <= 0 {
            continue;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                CLASS_NAME.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                x,
                y,
                width,
                height,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            log::error!("Guide overlay 创建失败: monitor={:?}", monitor);
            continue;
        }
        let data = Box::new(GuideOverlayData {
            d2d_factory: None,
            d2d_target: None,
        });
        let data_ptr = Box::into_raw(data);
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
            // 初始隐藏：只有真的命中对齐才按包围盒摆出来
            ShowWindow(hwnd, SW_HIDE);
        }
        if let Ok(mut guard) = GUIDE_OVERLAY.lock() {
            guard.push(MonitorOverlay { hwnd, monitor });
        }
    }

    log::info!(
        "Guide overlay 已创建: {} 块屏 {:?}",
        monitors.len(),
        monitors
    );
    Ok(())
}

/// 销毁所有 Overlay 窗口
pub fn destroy_guide_overlay() {
    let list = GUIDE_OVERLAY
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default();
    if list.is_empty() {
        return;
    }
    for overlay in &list {
        unsafe {
            DestroyWindow(overlay.hwnd);
        }
    }
    log::info!("Guide overlay 窗口已销毁: {} 个", list.len());
}

/// 隐藏参考线（不销毁窗口）
pub fn hide_guide_lines() {
    if let Ok(guard) = GUIDE_OVERLAY.lock() {
        for overlay in guard.iter() {
            unsafe {
                ShowWindow(overlay.hwnd, SW_HIDE);
            }
        }
    }
}

/// 把一条线段裁到某块屏内，返回监视器本地坐标；完全不相交返回 None
///
/// `monitor` = (x, y, w, h) 屏幕坐标；`horizontal` 线固定 y、x 取 [x0,x1]，
/// 垂直线固定 x、y 取 [y0,y1]。**必须**保留 at 轴在屏内的判定：跨屏线会被
/// 拆成两段共线线段，各自画在自己屏的窗口里，DWM 才没有 DPI 歧义。
#[must_use]
pub fn clip_segment_to_monitor(
    horizontal: bool,
    at: i32,
    from: i32,
    to: i32,
    monitor: (i32, i32, i32, i32),
) -> Option<GuideSegment> {
    let (mx, my, mw, mh) = monitor;
    if mw <= 0 || mh <= 0 {
        return None;
    }
    let (mx1, my1) = (mx + mw, my + mh);
    let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
    if horizontal {
        // 水平线：y 必须落在屏内，x 与屏求交
        if at < my || at >= my1 {
            return None;
        }
        let a = lo.max(mx);
        let b = hi.min(mx1);
        if a >= b {
            return None;
        }
        Some(GuideSegment {
            horizontal: true,
            at: at - my,
            from: a - mx,
            to: b - mx,
        })
    } else {
        if at < mx || at >= mx1 {
            return None;
        }
        let a = lo.max(my);
        let b = hi.min(my1);
        if a >= b {
            return None;
        }
        Some(GuideSegment {
            horizontal: false,
            at: at - mx,
            from: a - my,
            to: b - my,
        })
    }
}

/// 某块屏本轮要画的线段（监视器本地坐标）
///
/// 水平线在 `line_start.0 .. line_end.0` 上变化、固定轴是 y(=position)；
/// 垂直线在 `line_start.1 .. line_end.1` 上变化、固定轴是 x(=position)。
/// 两个方向都支持 => 同一轮里水平与垂直参考线可以同时出现（两两对齐常见）。
#[must_use]
pub fn segments_for_monitor(
    result: &AlignmentResult,
    monitor: (i32, i32, i32, i32),
) -> Vec<GuideSegment> {
    let mut out = Vec::new();
    for m in &result.horizontal_matches {
        if let Some(s) =
            clip_segment_to_monitor(true, m.position, m.line_start.0, m.line_end.0, monitor)
        {
            out.push(s);
        }
    }
    for m in &result.vertical_matches {
        if let Some(s) =
            clip_segment_to_monitor(false, m.position, m.line_start.1, m.line_end.1, monitor)
        {
            out.push(s);
        }
    }
    out
}

/// 线段包围盒（同一批线段必须同属一块屏）：返回 (x0, y0, x1, y1)，空集合返回 None
///
/// ⚠ **不得在这里做退化判定**：只有一条线时包围盒天然是一维的
/// （横线 y0 == y1、竖线 x0 == x1）。旧实现把 `x0 >= x1 || y0 >= y1` 直接判提前，
/// 导致「只有 A-B 一组命中」时整条线被丢掉，必须 A-B、A-C 同时命中才看得到
/// （2026-11-26 用户复验反馈）。退化与否的判定必须放在**加了 margin 之后**的调用方。
#[must_use]
pub fn bounds_of(segs: &[GuideSegment]) -> Option<(i32, i32, i32, i32)> {
    let mut x0 = i32::MAX;
    let mut y0 = i32::MAX;
    let mut x1 = i32::MIN;
    let mut y1 = i32::MIN;
    for s in segs {
        let (a, b) = (s.from.min(s.to), s.from.max(s.to));
        if s.horizontal {
            x0 = x0.min(a);
            x1 = x1.max(b);
            y0 = y0.min(s.at);
            y1 = y1.max(s.at);
        } else {
            x0 = x0.min(s.at);
            x1 = x1.max(s.at);
            y0 = y0.min(a);
            y1 = y1.max(b);
        }
    }
    if segs.is_empty() {
        return None;
    }
    // 即使 x0 == x1（单条横线）或 y0 == y1（单条竖线）也必须返回：
    // 调用方会先加 margin 再判退化，单组对齐同样要能看到线。
    Some((x0, y0, x1, y1))
}

/// overlay 窗口的屏幕坐标 = 监视器原点 + 包围盒原点
#[must_use]
pub fn overlay_window_position(monitor: (i32, i32, i32, i32), bbox: (i32, i32)) -> (i32, i32) {
    (monitor.0 + bbox.0, monitor.1 + bbox.1)
}

/// 更新参考线：每块屏各自裁剪、各自摆窗口、各自绘制
pub fn update_guide_lines(alignment: &AlignmentResult) {
    // 先在锁内抄出窗口清单：绘制路径里有 SetWindowPos / UpdateLayeredWindow 等
    // 会重入窗口过程的同步调用，绝不能持锁跨过去（与分类框同一套借用纪律）。
    let overlays: Vec<(HWND, (i32, i32, i32, i32))> = match GUIDE_OVERLAY.lock() {
        Ok(g) => g.iter().map(|o| (o.hwnd, o.monitor)).collect(),
        Err(_) => return,
    };
    if overlays.is_empty() {
        return;
    }

    let has_lines =
        !alignment.horizontal_matches.is_empty() || !alignment.vertical_matches.is_empty();
    let now = std::time::Instant::now();
    if !has_lines {
        // 没命中时保留上一次画面 300ms 宽限期，避免拖拽过程中线闪烁
        let should_hide = match LAST_HIT.lock() {
            Ok(g) => g
                .map(|t| now.duration_since(t).as_millis() >= 300)
                .unwrap_or(true),
            Err(_) => true,
        };
        if should_hide {
            for (hwnd, _) in &overlays {
                unsafe {
                    ShowWindow(*hwnd, SW_HIDE);
                }
            }
        }
        return;
    }
    if let Ok(mut g) = LAST_HIT.lock() {
        *g = Some(now);
    }
    // 渲染节流 33ms（约 30Hz 跟手）
    let due = match LAST_RENDER.lock() {
        Ok(mut g) => {
            let d = g
                .map(|t| now.duration_since(t).as_millis() >= 33)
                .unwrap_or(true);
            if d {
                *g = Some(now);
            }
            d
        }
        Err(_) => true,
    };
    if !due {
        return;
    }

    let margin = (GUIDE_LINE_HALO_WIDTH / 2.0).ceil() as i32 + 4;
    for (hwnd, monitor) in &overlays {
        let segs = segments_for_monitor(alignment, *monitor);
        let Some((bx0, by0, bx1, by1)) = bounds_of(&segs) else {
            // 本屏本轮没有线段（例如两框都在另一块屏上）：只隐藏，不影响别的屏
            unsafe {
                ShowWindow(*hwnd, SW_HIDE);
            }
            continue;
        };
        // 先加 margin 再判退化：单条横线 / 单条竖线的包围盒本身就是一维的，
        // 不加边距会被判成退化直接隐藏 = 对齐了也不显示（2026-11-03 用户报根因）。
        let (_, _, mw, mh) = *monitor;
        let x0 = (bx0 - margin).max(0);
        let y0 = (by0 - margin).max(0);
        let x1 = (bx1 + margin).min(mw);
        let y1 = (by1 + margin).min(mh);
        if x0 >= x1 || y0 >= y1 {
            unsafe {
                ShowWindow(*hwnd, SW_HIDE);
            }
            continue;
        }
        // 平移进窗口坐标，并把窗口严格夹在本屏内（溢到别的屏就重新引入 DPI 歧义）
        let local: Vec<GuideSegment> = segs.iter().copied().map(|s| s.translated(x0, y0)).collect();
        let (wx, wy) = overlay_window_position(*monitor, (x0, y0));
        let (w, h) = (x1 - x0, y1 - y0);
        unsafe {
            SetWindowPos(*hwnd, HWND_TOPMOST, wx, wy, w, h, SWP_NOACTIVATE);
        }
        unsafe {
            render_overlay(*hwnd, &local, w, h);
        }
    }
}

/// 把线段画进单个 overlay 窗口并提交给分层窗口（DIB + UpdateLayeredWindow）
unsafe fn render_overlay(hwnd: HWND, segs: &[GuideSegment], w: i32, h: i32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let Some(data) = get_guide_data(hwnd) else {
        log::warn!("Guide overlay 缺少窗口数据");
        return;
    };
    init_d2d_devices(data);
    let (Some(ref factory), Some(ref target)) = (&data.d2d_factory, &data.d2d_target) else {
        log::warn!("Guide D2D 设备未初始化");
        return;
    };

    let screen_dc = GetDC(std::ptr::null_mut());
    let mem_dc = CreateCompatibleDC(screen_dc);
    if mem_dc.is_null() {
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        return;
    }

    let bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: w,
        bi_height: -h,
        bi_planes: 1,
        bi_bit_count: 32,
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };

    let mut bits: *mut Pixel32 = std::ptr::null_mut();
    let dib = CreateDIBSection(
        mem_dc,
        &bi as *const BITMAPINFOHEADER as *const c_void,
        DIB_RGB_COLORS,
        &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
        std::ptr::null_mut(),
        0,
    );
    if dib.is_null() || bits.is_null() {
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        return;
    }

    let old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);
    let bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    if target.BindDC(GdiHDC(mem_dc), &bind_rect).is_err() {
        log::error!("Guide BindDC 失败");
        SelectObject(mem_dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        return;
    }

    target.BeginDraw();
    // 整屏清成全透明（背景必须透明，否则就是一块黑屏盖住桌面）
    target.Clear(Some(&D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    }));

    let stroke_style = factory.CreateStrokeStyle(
        &windows::Win32::Graphics::Direct2D::D2D1_STROKE_STYLE_PROPERTIES {
            startCap: windows::Win32::Graphics::Direct2D::D2D1_CAP_STYLE_FLAT,
            endCap: windows::Win32::Graphics::Direct2D::D2D1_CAP_STYLE_FLAT,
            dashCap: windows::Win32::Graphics::Direct2D::D2D1_CAP_STYLE_FLAT,
            lineJoin: windows::Win32::Graphics::Direct2D::D2D1_LINE_JOIN_MITER,
            miterLimit: 10.0,
            dashStyle: windows::Win32::Graphics::Direct2D::D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        },
        None,
    );
    let halo_brush = target.CreateSolidColorBrush(&GUIDE_HALO_COLOR, None);
    let core_brush = target.CreateSolidColorBrush(&GUIDE_CORE_COLOR, None);

    if let (Ok(ref hb), Ok(ref cb)) = (&halo_brush, &core_brush) {
        // 两遍绘制：先把所有暗色描边画完，再画所有亮色内芯。
        // 单遍「一条线画完芯再画下一条」会被后一条线的描边盖住前一条线的芯，
        // 交叉点（同时水平和垂直命中）就会出现缺口。
        for s in segs {
            let (p0, p1) = s.endpoints();
            match &stroke_style {
                Ok(ref style) => target.DrawLine(p0, p1, hb, GUIDE_LINE_HALO_WIDTH, Some(style)),
                _ => target.DrawLine(p0, p1, hb, GUIDE_LINE_HALO_WIDTH, None),
            }
        }
        for s in segs {
            let (p0, p1) = s.endpoints();
            match &stroke_style {
                Ok(ref style) => target.DrawLine(p0, p1, cb, GUIDE_LINE_CORE_WIDTH, Some(style)),
                _ => target.DrawLine(p0, p1, cb, GUIDE_LINE_CORE_WIDTH, None),
            }
        }
    }

    let mut tag1 = 0;
    let mut tag2 = 0;
    if let Err(e) = target.EndDraw(Some(&mut tag1), Some(&mut tag2)) {
        log::error!("Guide D2D EndDraw 失败: {:?}", e);
    }

    let size = POINT { x: w, y: h };
    let src_pt = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 255,
        alpha_format: AC_SRC_ALPHA,
    };
    // LWA_ALPHA(0x2) 与 ULW_ALPHA 同值：按 DIB 的每像素 alpha 合成（预乘 BGRA）
    let _ = UpdateLayeredWindow(
        hwnd,
        screen_dc,
        std::ptr::null(),
        &size,
        mem_dc,
        &src_pt,
        0,
        &blend,
        LWA_ALPHA,
    );
    ShowWindow(hwnd, SW_SHOWNA);

    SelectObject(mem_dc, old_bmp);
    DeleteObject(dib as HGDIOBJ);
    DeleteDC(mem_dc);
    ReleaseDC(std::ptr::null_mut(), screen_dc);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::windows::alignment_guide::alignment_detect::AlignmentMatch;

    fn mon(x: i32, y: i32, w: i32, h: i32) -> (i32, i32, i32, i32) {
        (x, y, w, h)
    }

    fn hline(at: i32, x0: i32, x1: i32) -> AlignmentMatch {
        AlignmentMatch {
            position: at,
            line_start: (x0, at),
            line_end: (x1, at),
        }
    }

    fn vline(at: i32, y0: i32, y1: i32) -> AlignmentMatch {
        AlignmentMatch {
            position: at,
            line_start: (at, y0),
            line_end: (at, y1),
        }
    }

    #[test]
    fn clip_keeps_segment_fully_inside_monitor_in_local_coords() {
        let s = clip_segment_to_monitor(true, 120, 3000, 3400, mon(3120, 0, 1920, 1080)).unwrap();
        assert_eq!(
            s,
            GuideSegment {
                horizontal: true,
                at: 120,
                from: 0,
                to: 280
            }
        );
    }

    #[test]
    fn clip_splits_one_line_across_two_monitors_into_two_collinear_segments() {
        // 跨屏横线：y=50，x 从主屏 2000 延伸到副屏 3600
        let primary = mon(0, 0, 3120, 2080);
        let secondary = mon(3120, 0, 1920, 1080);
        let a = clip_segment_to_monitor(true, 50, 2000, 3600, primary).unwrap();
        let b = clip_segment_to_monitor(true, 50, 2000, 3600, secondary).unwrap();
        assert_eq!((a.at, a.from, a.to), (50, 2000, 3120));
        assert_eq!((b.at, b.from, b.to), (50, 0, 480));
        // 两段接起来正好覆盖原始跨度（无缝 = 视觉上是一条连续的线）
        assert_eq!(a.to - a.from + (b.to - b.from), 3600 - 2000);
    }

    #[test]
    fn clip_returns_none_when_segment_misses_monitor() {
        // 横线在副屏高度之外（副屏只有 1080 高）
        assert!(
            clip_segment_to_monitor(true, 1500, 3200, 4000, mon(3120, 0, 1920, 1080)).is_none()
        );
        // 竖线落在屏右侧之外（x=150 > 宽 100）
        assert!(clip_segment_to_monitor(false, 150, 10, 200, mon(0, 0, 100, 400)).is_none());
        // 竖线在负坐标屏之外
        assert!(clip_segment_to_monitor(false, -50, 10, 200, mon(0, 0, 100, 400)).is_none());
        // 退化线段（零长度）不画
        assert!(clip_segment_to_monitor(true, 10, 100, 100, mon(0, 0, 400, 400)).is_none());
    }

    #[test]
    fn clip_handles_negative_virtual_origin_monitor() {
        // 副屏挂在主屏左侧：origin 为负
        let s = clip_segment_to_monitor(false, -1000, 60, 400, mon(-1920, 0, 1920, 1080)).unwrap();
        assert_eq!(
            s,
            GuideSegment {
                horizontal: false,
                at: 920,
                from: 60,
                to: 400
            }
        );
    }

    #[test]
    fn bounds_and_position_keep_overlay_inside_its_monitor() {
        let segs = vec![
            GuideSegment {
                horizontal: true,
                at: 20,
                from: 10,
                to: 300,
            },
            GuideSegment {
                horizontal: false,
                at: 300,
                from: 20,
                to: 250,
            },
        ];
        let (x0, y0, x1, y1) = bounds_of(&segs).unwrap();
        assert_eq!((x0, y0, x1, y1), (10, 20, 300, 250));
        assert_eq!(
            overlay_window_position(mon(3120, 0, 1920, 1080), (x0, y0)),
            (3130, 20)
        );
        assert_eq!(bounds_of(&[]), None);
    }

    /// 用户复验反馈的回归：**只有一组 A-B 命中也必须出线**。
    /// 旧实现在 bounds_of 里就把一维包围盒判成退化丢掉，导致
    /// 「非要 A-B、A-C 同时命中才看得到参考线」。
    #[test]
    fn single_horizontal_pair_still_produces_a_line() {
        let segs = vec![GuideSegment {
            horizontal: true,
            at: 64,
            from: 100,
            to: 900,
        }];
        // 包围盒是一维的（y0 == y1），但必须返回 Some，由调用方加 margin 后决定
        assert_eq!(bounds_of(&segs), Some((100, 64, 900, 64)));

        // 单条竖线同理
        let v = vec![GuideSegment {
            horizontal: false,
            at: 300,
            from: 40,
            to: 640,
        }];
        assert_eq!(bounds_of(&v), Some((300, 40, 300, 640)));

        // 模拟调用方：加 margin 后不再退化，确实会画出来
        let margin = (GUIDE_LINE_HALO_WIDTH / 2.0).ceil() as i32 + 4;
        let (x0, y0, x1, y1) = bounds_of(&segs).unwrap();
        let box_ = (x0 - margin, y0 - margin, x1 + margin, y1 + margin);
        assert!(
            box_.0 < box_.2 && box_.1 < box_.3,
            "单组命中不得被判退化: {box_:?}"
        );
    }

    /// 两组命中（A-B 与 A-C）同时存在时，两条线都要保留
    #[test]
    fn multiple_pairs_are_all_kept() {
        let segs = vec![
            GuideSegment {
                horizontal: true,
                at: 64,
                from: 100,
                to: 900,
            },
            GuideSegment {
                horizontal: true,
                at: 260,
                from: 120,
                to: 700,
            },
            GuideSegment {
                horizontal: false,
                at: 500,
                from: 64,
                to: 260,
            },
        ];
        assert_eq!(bounds_of(&segs), Some((100, 64, 900, 260)));
    }

    #[test]
    fn segments_for_monitor_covers_both_axes_and_skips_other_monitors() {
        let mut result = AlignmentResult::default();
        result.horizontal_matches.push(hline(400, 100, 3500));
        result.vertical_matches.push(vline(2000, 100, 500));
        let primary = segments_for_monitor(&result, mon(0, 0, 3120, 2080));
        let secondary = segments_for_monitor(&result, mon(3120, 0, 1920, 1080));
        assert_eq!(primary.len(), 2, "主屏应拿到横线(裁剪后)与竖线");
        assert_eq!(secondary.len(), 1, "副屏只该拿到横线超出 3120 的那一段");
        assert!(secondary[0].horizontal);
        assert_eq!(secondary[0].from, 0);
    }

    #[test]
    fn overlay_position_keeps_negative_monitor_origin() {
        assert_eq!(
            overlay_window_position(mon(-1920, -200, 1920, 1080), (5, 7)),
            (-1915, -193)
        );
    }
}
