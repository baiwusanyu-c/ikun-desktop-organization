//! ����ο��� Overlay ����
//!
//! ����һ��ȫ��͸���ķֲ㴰�ڣ����ڻ��ƶ���ο��ߡ�
//! ����ʹ�� WS_EX_TRANSPARENT ��ʽ��ʹ����¼���͸��

use std::ffi::c_void;
use std::sync::Mutex;

use windows::Win32::Graphics::Direct2D::{
    Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_POINT_2F},
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;

use super::alignment_detect::AlignmentResult;
use crate::platform::windows::ffi_imports::*;
use crate::platform::windows::ffi_types::*;

/// �ο��� Overlay ��������
struct GuideOverlayData {
    /// ��������ȣ��������أ�
    width: i32,
    /// �������߶ȣ��������أ�
    height: i32,
    /// D2D Factory
    d2d_factory: Option<ID2D1Factory>,
    /// DC RenderTarget
    d2d_target: Option<ID2D1DCRenderTarget>,
}

unsafe impl Send for GuideOverlayData {}

/// �ο��� Overlay ���ھ����װ
struct GuideOverlayWindow {
    hwnd: HWND,
}

unsafe impl Send for GuideOverlayWindow {}

static GUIDE_OVERLAY: Mutex<Option<GuideOverlayWindow>> = Mutex::new(None);
static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

static CLASS_NAME: &[u16] = &[
    73, 107, 117, 110, 71, 117, 105, 100, 101, 79, 118, 101, 114, 108, 97, 121, 0,
]; // "IkunGuideOverlay"

/// �ο�����ɫ����͸����ɫ (rgba: 0, 120, 255, 200)
const GUIDE_COLOR_R: f32 = 100.0 / 255.0;
const GUIDE_COLOR_G: f32 = 255.0 / 255.0;
const GUIDE_COLOR_B: f32 = 150.0 / 255.0;
const GUIDE_COLOR_A: f32 = 255.0 / 255.0;

/// �ο��߿��
const GUIDE_LINE_WIDTH: f32 = 3.0;

/// ��ȡ��������
fn get_guide_data(hwnd: HWND) -> Option<&'static mut GuideOverlayData> {
    let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    if data_ptr == 0 {
        return None;
    }
    Some(unsafe { &mut *(data_ptr as *mut GuideOverlayData) })
}

/// ���ڹ��̣���������С��Ϣ����
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

/// ȷ����������ע��
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

/// ��ʼ�� D2D �豸
fn init_d2d_devices(data: &mut GuideOverlayData) {
    if data.d2d_factory.is_none() {
        match unsafe {
            D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
        } {
            Ok(factory) => data.d2d_factory = Some(factory),
            Err(e) => {
                log::error!("���� Guide D2D Factory ʧ��: {:?}", e);
                return;
            }
        }
    }

    if data.d2d_target.is_none() {
        if let Some(ref factory) = data.d2d_factory {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            match unsafe { factory.CreateDCRenderTarget(&props) } {
                Ok(target) => {
                    data.d2d_target = Some(target);
                }
                Err(e) => {
                    log::error!("���� Guide D2D DC RenderTarget ʧ��: {:?}", e);
                }
            }
        }
    }
}

/// ��������ʾ Guide Overlay ����
pub fn create_guide_overlay() -> Result<(), &'static str> {
    // ����Ƿ��Ѵ���
    if let Ok(guard) = GUIDE_OVERLAY.lock() {
        if guard.is_some() {
            return Ok(());
        }
    }

    let hinstance = ensure_class_registered()?;

    // ��ȡ��������߽磨����������ʾ����
    let (x, y, width, height) =
        crate::platform::windows::monitor_utils::get_virtual_desktop_bounds();

    // ����͸���ֲ㴰�ڣ���괩͸
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

    // ���ô�������
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


    // ���浽ȫ��
    if let Ok(mut guard) = GUIDE_OVERLAY.lock() {
        *guard = Some(GuideOverlayWindow { hwnd });
    }

    log::info!("Guide overlay �����Ѵ���: {}x{} at ({}, {})", width, height, x, y);
    Ok(())
}

/// ���� Guide Overlay ����
pub fn destroy_guide_overlay() {
    if let Ok(mut guard) = GUIDE_OVERLAY.lock() {
        if let Some(overlay) = guard.take() {
            unsafe {
                DestroyWindow(overlay.hwnd);
            }
            log::info!("Guide overlay ����������");
        }
    }
}

/// ���²ο�����ʾ
///
/// # ����
/// - `alignment`: ��������
/// - `work_area_offset`: ���������Ͻ�ƫ�ƣ������������꣩
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

    // ����Ƿ��ж���
    let has_lines = !alignment.horizontal_matches.is_empty() || !alignment.vertical_matches.is_empty();

    if !has_lines {
        // ���ش���
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        return;
    }

    // ��ʼ�� D2D �豸
    init_d2d_devices(data);

    let (Some(ref _factory), Some(ref target)) = (&data.d2d_factory, &data.d2d_target) else {
        log::warn!("Guide D2D �豸δ��ʼ��");
        return;
    };

    let w = data.width;
    let h = data.height;
    if w <= 0 || h <= 0 {
        return;
    }

    // ���� DIB ����Ⱦ
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

    // �� DC �� D2D target
    let bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };

    if unsafe { target.BindDC(GdiHDC(mem_dc), &bind_rect) }.is_err() {
        log::error!("Guide BindDC ʧ��");
        unsafe {
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        return;
    }

    // ��ʼ����
    unsafe { target.BeginDraw() };

    // ���Ϊȫ͸��
    unsafe {
        target.Clear(Some(&windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));
    }

    // �����ο��߻���
    let guide_color = windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
        r: GUIDE_COLOR_R,
        g: GUIDE_COLOR_G,
        b: GUIDE_COLOR_B,
        a: GUIDE_COLOR_A,
    };

    let brush = unsafe { target.CreateSolidColorBrush(&guide_color, None) };

    if let Ok(ref brush) = brush {
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

        // ����ˮƽ�ο���
        for m in &alignment.horizontal_matches {
            let y = (m.position - work_area_offset.1) as f32;
            let x0 = (m.line_start.0 - work_area_offset.0) as f32;
            let x1 = (m.line_end.0 - work_area_offset.0) as f32;
            let p0 = D2D_POINT_2F { x: x0, y };
            let p1 = D2D_POINT_2F { x: x1, y };

            if let Ok(ref style) = stroke_style {
                unsafe {
                    target.DrawLine(p0, p1, brush, GUIDE_LINE_WIDTH, Some(style));
                }
            } else {
                unsafe {
                    target.DrawLine(p0, p1, brush, GUIDE_LINE_WIDTH, None);
                }
            }
        }

        // ���ƴ�ֱ�ο���
        for m in &alignment.vertical_matches {
            let x = (m.position - work_area_offset.0) as f32;
            let y0 = (m.line_start.1 - work_area_offset.1) as f32;
            let y1 = (m.line_end.1 - work_area_offset.1) as f32;
            let p0 = D2D_POINT_2F { x, y: y0 };
            let p1 = D2D_POINT_2F { x, y: y1 };

            if let Ok(ref style) = stroke_style {
                unsafe {
                    target.DrawLine(p0, p1, brush, GUIDE_LINE_WIDTH, Some(style));
                }
            } else {
                unsafe {
                    target.DrawLine(p0, p1, brush, GUIDE_LINE_WIDTH, None);
                }
            }
        }
    }

    // ��������
    let mut tag1 = 0;
    let mut tag2 = 0;
    if let Err(e) = unsafe { target.EndDraw(Some(&mut tag1), Some(&mut tag2)) } {
        log::error!("Guide D2D EndDraw ʧ��: {:?}", e);
    }

    // ���·ֲ㴰��
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

        // ��ʾ����
        ShowWindow(hwnd, SW_SHOWNA);
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);

        // ����
        SelectObject(mem_dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }
}

/// ���زο���
pub fn hide_guide_lines() {
    if let Ok(guard) = GUIDE_OVERLAY.lock() {
        if let Some(overlay) = guard.as_ref() {
            unsafe {
                ShowWindow(overlay.hwnd, SW_HIDE);
            }
        }
    }
}

/// ��� Guide Overlay �Ƿ��Ѵ���
pub fn is_guide_overlay_created() -> bool {
    GUIDE_OVERLAY.lock().map(|g| g.is_some()).unwrap_or(false)
}

