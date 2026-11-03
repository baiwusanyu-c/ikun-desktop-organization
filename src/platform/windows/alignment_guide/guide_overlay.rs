//! 对齐参考线 Overlay 模块
//!
//! 创建一个全屏透明的分层窗口，用于绘制移动/缩放时的对齐参考线。
//! 窗口带 WS_EX_TRANSPARENT，鼠标事件直接穿透到下层窗口。

use std::ffi::c_void;
use std::sync::Mutex;

use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1Factory};
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;
use windows_numerics::Vector2;

use super::alignment_detect::AlignmentResult;
use crate::platform::windows::ffi_imports::*;
use crate::platform::windows::ffi_types::*;
use crate::platform::windows::render_devices;

/// 桌面颜色采样网格大小（NxN 个采样点）
/// 10x10=100 个采样点足够估算屏幕中心区域亮度，同时保持启动时间 < 50ms
const SAMPLE_GRID_SIZE: i32 = 10;

/// 参考线 Overlay 的窗口数据
struct GuideOverlayData {
    /// 画布宽度（物理像素）
    width: i32,
    /// 画布高度（物理像素）
    height: i32,
    /// D2D Factory
    d2d_factory: Option<ID2D1Factory>,
    /// DC RenderTarget
    d2d_target: Option<ID2D1DCRenderTarget>,
}

unsafe impl Send for GuideOverlayData {}

/// 参考线 Overlay 的窗口句柄包装
struct GuideOverlayWindow {
    hwnd: HWND,
}

unsafe impl Send for GuideOverlayWindow {}

/// 采样屏幕指定位置的像素颜色 (返回 RGB)
fn sample_screen_color(screen_dc: HDC, x: i32, y: i32) -> (u8, u8, u8) {
    let pixel = unsafe { GetPixel(screen_dc, x, y) };
    // GetPixel returns COLORREF: 0x00BBGGRR
    let r = (pixel & 0xFF) as u8;
    let g = ((pixel >> 8) & 0xFF) as u8;
    let b = ((pixel >> 16) & 0xFF) as u8;
    (r, g, b)
}

/// 计算颜色的相对亮度 (0.0 = 黑色, 1.0 = 白色)
/// 使用 W3C 标准公式: L = 0.2126*R + 0.7152*G + 0.0722*B
fn relative_luminance(r: u8, g: u8, b: u8) -> f32 {
    let r_linear = (r as f32 / 255.0).powf(2.2);
    let g_linear = (g as f32 / 255.0).powf(2.2);
    let b_linear = (b as f32 / 255.0).powf(2.2);
    0.2126 * r_linear + 0.7152 * g_linear + 0.0722 * b_linear
}

/// 根据背景亮度计算对比色参考线颜色
/// 返回 D2D1_COLOR_F
/// Sample desktop luminance once at startup, return cached guide color.
/// Call init_guide_color() once before first render.
pub fn init_guide_color() {
    use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

    log::info!("[GuideColor] Starting desktop color sampling...");

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    if screen_dc.is_null() {
        log::error!("[GuideColor] GetDC failed, using default white");
        let fallback = D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.95,
        };
        if let Ok(mut guard) = CACHED_GUIDE_COLOR.lock() {
            *guard = Some(fallback);
        }
        return;
    }

    let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    log::info!("[GuideColor] Screen size: {}x{}", sw, sh);

    let sample_count = SAMPLE_GRID_SIZE;
    let mut total_luminance = 0.0f32;
    let mut valid = 0i32;

    for i in 0..sample_count {
        for j in 0..sample_count {
            let x = sw / 4 + (sw / 2) * i / (sample_count - 1);
            let y = sh / 4 + (sh / 2) * j / (sample_count - 1);
            let (r, g, b) = sample_screen_color(screen_dc, x, y);
            let lum = relative_luminance(r, g, b);
            if i == 0 && j == 0 {
                log::info!(
                    "[GuideColor] First sample at ({},{}): rgb=({},{},{}) lum={:.3}",
                    x,
                    y,
                    r,
                    g,
                    b,
                    lum
                );
            }
            total_luminance += lum;
            valid += 1;
        }
    }

    unsafe { ReleaseDC(std::ptr::null_mut(), screen_dc) };

    let avg = if valid > 0 {
        total_luminance / valid as f32
    } else {
        0.5
    };

    let color = if avg > 0.5 {
        // Light background -> dark guide line
        D2D1_COLOR_F {
            r: 30.0 / 255.0,
            g: 40.0 / 255.0,
            b: 60.0 / 255.0,
            a: 0.95,
        }
    } else {
        // Dark background -> white guide line
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.95,
        }
    };

    if let Ok(mut guard) = CACHED_GUIDE_COLOR.lock() {
        *guard = Some(color);
    }
    log::info!(
        "[GuideColor] DONE: avg_luminance={:.3}, selected=rgb({:.0},{:.0},{:.0})",
        avg,
        color.r * 255.0,
        color.g * 255.0,
        color.b * 255.0
    );
}

/// Get cached guide color; lazy-initializes if not yet set
fn get_guide_color() -> windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
    use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
    {
        if let Ok(guard) = CACHED_GUIDE_COLOR.lock() {
            if let Some(c) = *guard {
                return c;
            }
        }
    }
    // 渲染路径绝不同步采样：实测首次拖拽时在此卡死整条
    // 渲染线程（2026-09-14）。改为后台线程采样，本帧先用默认色。
    static INIT_SPAWNED: std::sync::Once = std::sync::Once::new();
    INIT_SPAWNED.call_once(|| {
        std::thread::spawn(init_guide_color);
    });
    // Ultimate fallback: white
    D2D1_COLOR_F {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 0.95,
    }
}

static CACHED_GUIDE_COLOR: Mutex<Option<windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F>> =
    Mutex::new(None);

static GUIDE_OVERLAY: Mutex<Option<GuideOverlayWindow>> = Mutex::new(None);
static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

static CLASS_NAME: &[u16] = &[
    73, 107, 117, 110, 71, 117, 105, 100, 101, 79, 118, 101, 114, 108, 97, 121, 0,
]; // "IkunGuideOverlay"

/// 参考线默认颜色：半透明蓝 (rgba: 0, 120, 255, 200)

/// 参考线线宽（物理像素）
const GUIDE_LINE_WIDTH: f32 = 3.0;

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

/// 初始化 D2D 设备
///
/// 工厂取渲染线程共享实例（见 `render_devices`），参考线窗口只保留
/// 自己的 DC 渲染目标；旧实现在这里再建一个 `ID2D1Factory`，
/// 等于又多一个独立 D3D11 设备（约 50MB 私有内存）。
fn init_d2d_devices(data: &mut GuideOverlayData) {
    if data.d2d_factory.is_none() {
        data.d2d_factory = render_devices::d2d_factory();
    }
    if data.d2d_target.is_none() {
        data.d2d_target = render_devices::create_dc_render_target(0.0);
    }
}

/// 创建并显示 Guide Overlay（全屏分层窗）
pub fn create_guide_overlay() -> Result<(), &'static str> {
    // 已经建过就直接复用，不重复创建
    if let Ok(guard) = GUIDE_OVERLAY.lock() {
        if guard.is_some() {
            return Ok(());
        }
    }

    let hinstance = ensure_class_registered()?;

    // 虚拟桌面边界（覆盖全部显示器的并集）
    let (x, y, width, height) =
        crate::platform::windows::monitor_utils::get_virtual_desktop_bounds();

    // 创建透明分层窗口：鼠标穿透、不抢焦点、不进任务栏
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
        return Err("CreateWindowExW failed for guide overlay");
    }

    // 挂载窗口数据
    let data = Box::new(GuideOverlayData {
        width,
        height,
        d2d_factory: None,
        d2d_target: None,
    });
    let data_ptr = Box::into_raw(data);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
    }

    // 存入全局单例
    if let Ok(mut guard) = GUIDE_OVERLAY.lock() {
        *guard = Some(GuideOverlayWindow { hwnd });
    }

    log::info!(
        "Guide overlay 窗口已创建: {}x{} at ({}, {})",
        width,
        height,
        x,
        y
    );
    Ok(())
}

/// 销毁 Guide Overlay（释放全屏分层窗与 DIB）
pub fn destroy_guide_overlay() {
    if let Ok(mut guard) = GUIDE_OVERLAY.lock() {
        if let Some(overlay) = guard.take() {
            unsafe {
                DestroyWindow(overlay.hwnd);
            }
            log::info!("Guide overlay 窗口已销毁");
        }
    }
}

/// 更新参考线显示
///
/// # 参数
/// - `alignment`: 对齐检测结果（横线 / 竖线集合）
/// - `work_area_offset`: 虚拟桌面左上角偏移（把屏幕坐标转成本窗坐标）
pub fn update_guide_lines(alignment: &AlignmentResult, work_area_offset: (i32, i32)) {
    let guard = match GUIDE_OVERLAY.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    let overlay = match guard.as_ref() {
        Some(o) => o,
        None => return,
    };

    let hwnd = overlay.hwnd;
    let data = match get_guide_data(hwnd) {
        Some(d) => d,
        None => return,
    };

    // 本轮没有任何对齐命中 -> 保留上一帧画面 300ms 再隐藏。
    // 检测每 16ms 跑一次，但移动框矩形有 1~2 拍延迟，命中是间歇性的：
    // 立刻隐藏 = 参考线闪一帧就消失，肉眼看不到（2026-11-03 用户报「不显示」）。
    let has_lines =
        !alignment.horizontal_matches.is_empty() || !alignment.vertical_matches.is_empty();

    if !has_lines {
        use std::time::Instant;
        static LAST_HIT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        let now = Instant::now();
        let should_hide = match LAST_HIT.lock() {
            Ok(g) => g
                .map(|t| now.duration_since(t).as_millis() >= 300)
                .unwrap_or(true),
            Err(_) => true,
        };
        if should_hide {
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
            }
        }
        // should_hide=false 时直接 return：不调 UpdateLayeredWindow，
        // 分层窗保留上一帧位图 = 参考线继续可见。
        return;
    }
    // 有命中：刷新宽限期时间戳
    {
        use std::time::Instant;
        static LAST_HIT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        if let Ok(mut g) = LAST_HIT.lock() {
            *g = Some(Instant::now());
        }
    }

    // 初始化 D2D 设备（首帧懒建）
    // 参考线更新节流 33ms：每次鼠标移动都做全屏 5040x2080
    // DIB 重建 + 全屏 ULW 会拖垮 DWM（实测拖拽掉帧主因），30Hz 足够跟手。
    {
        use std::time::Instant;
        static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        let now = Instant::now();
        let due = match LAST.lock() {
            Ok(mut g) => {
                let d = g
                    .map(|t| now.duration_since(t).as_millis() >= 100)
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
    }
    // 参考线包围盒：overlay 窗口只覆盖线条区域（加少量边距），
    // ULW 表面从全屏 5040x2080(42MB) 降到约百 KB 级，DWM 合成开销
    // 与面积成正比，这是拖拽掉帧的根因（2026-09-15 实测）。
    let margin = GUIDE_LINE_WIDTH as i32 + 4;
    let mut bx0 = i32::MAX;
    let mut by0 = i32::MAX;
    let mut bx1 = i32::MIN;
    let mut by1 = i32::MIN;
    for m in &alignment.horizontal_matches {
        let y = m.position - work_area_offset.1;
        let x0 = m.line_start.0 - work_area_offset.0;
        let x1 = m.line_end.0 - work_area_offset.0;
        bx0 = bx0.min(x0).min(x1);
        bx1 = bx1.max(x0).max(x1);
        by0 = by0.min(y);
        by1 = by1.max(y);
    }
    for m in &alignment.vertical_matches {
        let x = m.position - work_area_offset.0;
        let y0 = m.line_start.1 - work_area_offset.1;
        let y1 = m.line_end.1 - work_area_offset.1;
        bx0 = bx0.min(x);
        bx1 = bx1.max(x);
        by0 = by0.min(y0).min(y1);
        by1 = by1.max(y0).max(y1);
    }
    if bx0 >= bx1 || by0 >= by1 {
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        return;
    }
    bx0 -= margin;
    by0 -= margin;
    bx1 += margin;
    by1 += margin;
    let off_x = bx0;
    let off_y = by0;
    let bw = bx1 - bx0;
    let bh = by1 - by0;
    unsafe {
        SetWindowPos(hwnd, HWND_TOPMOST, bx0, by0, bw, bh, SWP_NOACTIVATE);
    }
    init_d2d_devices(data);

    let (Some(ref _factory), Some(ref target)) = (&data.d2d_factory, &data.d2d_target) else {
        log::warn!("Guide D2D 设备未初始化");
        return;
    };

    let w = bw;
    let h = bh;
    if w <= 0 || h <= 0 {
        return;
    }

    // 准备 DIB 并渲染
    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    let mem_dc = unsafe { CreateCompatibleDC(screen_dc) };
    if mem_dc.is_null() {
        unsafe { ReleaseDC(std::ptr::null_mut(), screen_dc) };
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
    let dib = unsafe {
        CreateDIBSection(
            mem_dc,
            &bi as *const BITMAPINFOHEADER as *const c_void,
            DIB_RGB_COLORS,
            &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
            std::ptr::null_mut(),
            0,
        )
    };

    if dib.is_null() || bits.is_null() {
        unsafe {
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        return;
    }

    let old_bmp = unsafe { SelectObject(mem_dc, dib as HGDIOBJ) };

    // 把内存 DC 绑成 D2D 渲染目标
    let bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };

    if unsafe { target.BindDC(GdiHDC(mem_dc), &bind_rect) }.is_err() {
        log::error!("Guide BindDC 失败");
        unsafe {
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        return;
    }

    // 开始绘制
    unsafe { target.BeginDraw() };

    // 整屏清成全透明
    unsafe {
        target.Clear(Some(
            &windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
        ));
    }

    // 画参考线本体
    // 动态对比色参考线：根据背景亮度自动选择颜色
    let stroke_style = unsafe {
        _factory.CreateStrokeStyle(
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
        )
    };

    // 绘制水平参考线（动态对比色）
    for m in &alignment.horizontal_matches {
        let y = (m.position - work_area_offset.1 - off_y) as f32;
        let x0 = (m.line_start.0 - work_area_offset.0 - off_x) as f32;
        let x1 = (m.line_end.0 - work_area_offset.0 - off_x) as f32;
        let p0 = Vector2 { X: x0, Y: y };
        let p1 = Vector2 { X: x1, Y: y };

        if let Ok(ref dynamic_brush) =
            unsafe { target.CreateSolidColorBrush(&get_guide_color(), None) }
        {
            if let Ok(ref style) = stroke_style {
                unsafe {
                    target.DrawLine(p0, p1, dynamic_brush, GUIDE_LINE_WIDTH, Some(style));
                }
            } else {
                unsafe {
                    target.DrawLine(p0, p1, dynamic_brush, GUIDE_LINE_WIDTH, None);
                }
            }
        }
    }

    // 绘制垂直参考线（动态对比色）
    for m in &alignment.vertical_matches {
        let x = (m.position - work_area_offset.0 - off_x) as f32;
        let y0 = (m.line_start.1 - work_area_offset.1 - off_y) as f32;
        let y1 = (m.line_end.1 - work_area_offset.1 - off_y) as f32;
        let p0 = Vector2 { X: x, Y: y0 };
        let p1 = Vector2 { X: x, Y: y1 };

        if let Ok(ref dynamic_brush) =
            unsafe { target.CreateSolidColorBrush(&get_guide_color(), None) }
        {
            if let Ok(ref style) = stroke_style {
                unsafe {
                    target.DrawLine(p0, p1, dynamic_brush, GUIDE_LINE_WIDTH, Some(style));
                }
            } else {
                unsafe {
                    target.DrawLine(p0, p1, dynamic_brush, GUIDE_LINE_WIDTH, None);
                }
            }
        }
    }

    // 结束绘制
    let mut tag1 = 0;
    let mut tag2 = 0;
    if let Err(e) = unsafe { target.EndDraw(Some(&mut tag1), Some(&mut tag2)) } {
        log::error!("Guide D2D EndDraw 失败: {:?}", e);
    }

    // 把画好的位图提交给分层窗口
    let size = POINT { x: w, y: h };
    let src_pt = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 255,
        alpha_format: AC_SRC_ALPHA,
    };

    unsafe {
        UpdateLayeredWindow(
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

        // 显示窗口
        ShowWindow(hwnd, SW_SHOWNA);
        // 清理 GDI 对象
        SelectObject(mem_dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }
}

/// 隐藏参考线（不销毁窗口）
pub fn hide_guide_lines() {
    if let Ok(guard) = GUIDE_OVERLAY.lock() {
        if let Some(overlay) = guard.as_ref() {
            unsafe {
                ShowWindow(overlay.hwnd, SW_HIDE);
            }
        }
    }
}

/// 判断 Guide Overlay 是否已创建
pub fn is_guide_overlay_created() -> bool {
    GUIDE_OVERLAY.lock().map(|g| g.is_some()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_luminance_black() {
        let l = relative_luminance(0, 0, 0);
        assert!(l < 0.01, "Black should have near-zero luminance, got {}", l);
    }

    #[test]
    fn test_relative_luminance_white() {
        let l = relative_luminance(255, 255, 255);
        assert!(l > 0.99, "White should have near-1.0 luminance, got {}", l);
    }

    #[test]
    fn test_relative_luminance_gray() {
        let l = relative_luminance(128, 128, 128);
        assert!(
            l > 0.15 && l < 0.35,
            "Gray 128 should have moderate luminance, got {}",
            l
        );
    }

    #[test]
    fn test_relative_luminance_green_bias() {
        // Green contributes most to luminance (0.7152 weight)
        let l_green = relative_luminance(0, 255, 0);
        let l_red = relative_luminance(255, 0, 0);
        let l_blue = relative_luminance(0, 0, 255);
        assert!(l_green > l_red, "Green should be brighter than red");
        assert!(l_green > l_blue, "Green should be brighter than blue");
    }

    #[test]
    fn test_sample_grid_size_sufficient() {
        // SAMPLE_GRID_SIZE x SAMPLE_GRID_SIZE 采样点覆盖屏幕中心区域，
        // 在估算亮度和保持启动速度之间取得平衡
        let total_samples = SAMPLE_GRID_SIZE * SAMPLE_GRID_SIZE;
        assert!(
            total_samples >= 100,
            "At least 100 samples needed for reliable brightness estimation, got {}",
            total_samples
        );
        assert!(
            total_samples <= 2500,
            "At most 2500 samples to keep startup time reasonable, got {}",
            total_samples
        );
    }

    #[test]
    fn test_sample_grid_size_is_positive() {
        assert!(
            SAMPLE_GRID_SIZE > 1,
            "Grid size must be > 1 to avoid division by zero in sampling loop"
        );
    }
}
