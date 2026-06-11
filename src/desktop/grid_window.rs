//! 格子窗口 - 核心组件
//!
//! 每个收纳格子是一个独立的 WS_POPUP 分层窗口。
//! 所有图标在 WM_PAINT 中用 GDI 自绘，使用客户区相对坐标。
//! 拖动标题栏 = SetWindowPos 移动整个窗口，图标自然跟随。

use anyhow::Result;
use std::ffi::c_void;
use std::sync::Mutex;

// ===== Raw Win32 Types =====
type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HDC = *mut c_void;
type HBRUSH = *mut c_void;
type HPEN = *mut c_void;
type HFONT = *mut c_void;
type HGDIOBJ = *mut c_void;
type HCURSOR = *mut c_void;
type HBITMAP = *mut c_void;
type HICON = *mut c_void;
type LPARAM = isize;
type WPARAM = usize;
type LRESULT = isize;
type COLORREF = u32;

#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}
#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    time: u32,
    pt: POINT,
}
#[repr(C)]
struct PAINTSTRUCT {
    hdc: HDC,
    f_erase: i32,
    rc_paint: RECT,
    f_restore: i32,
    f_inc_update: i32,
    rgb_reserved: [u8; 32],
}

#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    lpfn_wnd_proc: Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: HINSTANCE,
    h_icon: *mut c_void,
    h_cursor: HCURSOR,
    hbr_background: HBRUSH,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
    h_icon_sm: *mut c_void,
}

// Constants
const WS_POPUP: u32 = 0x80000000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_EX_LAYERED: u32 = 0x00080000;
const WS_EX_TOOLWINDOW: u32 = 0x00000080;
const WS_EX_TOPMOST: u32 = 0x00000008;
const CS_HREDRAW: u32 = 0x0002;
const CS_VREDRAW: u32 = 0x0001;
const WM_PAINT: u32 = 0x000F;
const WM_DESTROY: u32 = 0x0002;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONUP: u32 = 0x0205;
const LWA_ALPHA: u32 = 0x00000002;
const GWLP_USERDATA: i32 = -21;
const SRCCOPY: u32 = 0x00CC0020;
const TRANSPARENT_MODE: i32 = 1;
const DT_LEFT: u32 = 0x00000000;
const DT_CENTER: u32 = 0x00000001;
const DT_VCENTER: u32 = 0x00000004;
const DT_SINGLELINE: u32 = 0x00000020;
const DT_END_ELLIPSIS: u32 = 0x00008000;
const PS_SOLID: i32 = 0;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOSIZE: u32 = 0x0001;
const IDC_ARROW_ID: u16 = 32512;

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}
fn get_x_lparam(lp: LPARAM) -> i32 {
    (lp & 0xFFFF) as i16 as i32
}
fn get_y_lparam(lp: LPARAM) -> i32 {
    ((lp >> 16) & 0xFFFF) as i16 as i32
}

extern "system" {
    fn GetModuleHandleW(lpModuleName: *const u16) -> HINSTANCE;
    fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        dwExStyle: u32,
        lpClassName: *const u16,
        lpWindowName: *const u16,
        dwStyle: u32,
        x: i32,
        y: i32,
        nWidth: i32,
        nHeight: i32,
        hWndParent: HWND,
        hMenu: *mut c_void,
        hInstance: HINSTANCE,
        lpParam: *mut c_void,
    ) -> HWND;
    fn SetLayeredWindowAttributes(hwnd: HWND, crKey: COLORREF, bAlpha: u8, dwFlags: u32) -> i32;
    fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> i32;
    fn SetWindowPos(
        hWnd: HWND,
        hWndInsertAfter: HWND,
        X: i32,
        Y: i32,
        cx: i32,
        cy: i32,
        uFlags: u32,
    ) -> i32;
    fn DestroyWindow(hWnd: HWND) -> i32;
    fn DefWindowProcW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn SetWindowLongPtrW(hWnd: HWND, nIndex: i32, dwNewLong: isize) -> isize;
    fn GetWindowLongPtrW(hWnd: HWND, nIndex: i32) -> isize;
    fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
    fn InvalidateRect(hWnd: HWND, lpRect: *const RECT, bErase: i32) -> i32;
    fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> i32;
    fn SetCapture(hWnd: HWND) -> HWND;
    fn ReleaseCapture() -> i32;
    fn GetCursorPos(lpPoint: *mut POINT) -> i32;
    fn ScreenToClient(hWnd: HWND, lpPoint: *mut POINT) -> i32;

    // GDI
    fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> i32;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn CreateCompatibleBitmap(hdc: HDC, cx: i32, cy: i32) -> HBITMAP;
    fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    fn DeleteObject(ho: HGDIOBJ) -> i32;
    fn DeleteDC(hdc: HDC) -> i32;
    fn BitBlt(
        hdc: HDC,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdcSrc: HDC,
        x1: i32,
        y1: i32,
        rop: u32,
    ) -> i32;
    fn FillRect(hDC: HDC, lprc: *const RECT, hbr: HBRUSH) -> i32;
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn CreatePen(iStyle: i32, cWidth: i32, color: COLORREF) -> HPEN;
    fn GetStockObject(i: i32) -> HGDIOBJ;
    fn RoundRect(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32, w: i32, h: i32) -> i32;
    fn SetBkMode(hdc: HDC, iBkMode: i32) -> i32;
    fn SetTextColor(hdc: HDC, color: COLORREF) -> COLORREF;
    fn CreateFontW(
        cHeight: i32,
        cWidth: i32,
        cEscapement: i32,
        cOrientation: i32,
        cWeight: i32,
        bItalic: i32,
        bUnderline: i32,
        bStrikeOut: i32,
        iCharSet: i32,
        iOutPrecision: i32,
        iClipPrecision: i32,
        iQuality: i32,
        iPitchAndFamily: i32,
        pszFaceName: *const u16,
    ) -> HFONT;
    fn DrawTextW(hdc: HDC, lpchText: *const u16, cchText: i32, lprc: *mut RECT, format: u32)
        -> i32;
    fn DrawIconEx(
        hdc: HDC,
        xLeft: i32,
        yTop: i32,
        hIcon: HICON,
        cxWidth: i32,
        cyWidth: i32,
        istepIfAniCur: u32,
        hbrFlickerFreeDraw: HBRUSH,
        diFlags: u32,
    ) -> i32;
}

const DI_NORMAL: u32 = 0x0003;
const NULL_BRUSH: i32 = 5;
const SW_SHOW: i32 = 5;

// ===== 数据结构 =====

/// 格子内的文件项
pub struct GridFileItem {
    pub file_path: String,
    pub display_name: String,
    pub icon_handle: HICON,
    /// 相对于窗口客户区的坐标
    pub client_x: i32,
    pub client_y: i32,
}

unsafe impl Send for GridFileItem {}

/// 格子窗口内部数据（存储在 GWLP_USERDATA）
struct GridWindowData {
    title: String,
    items: Vec<GridFileItem>,
    is_dragging: bool,
    drag_start_x: i32,
    drag_start_y: i32,
    window_x: i32,
    window_y: i32,
    grid_id: usize, // 用于回调
}

unsafe impl Send for GridWindowData {}

/// 格子窗口创建参数
pub struct GridCreateParams {
    pub id: usize,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub items: Vec<GridFileItem>,
}

/// 格子窗口句柄包装
pub struct GridWindow {
    pub hwnd: HWND,
    pub id: usize,
}

unsafe impl Send for GridWindow {}

// 全局格子列表（用于管理）
static GRID_WINDOWS: Mutex<Vec<GridWindow>> = Mutex::new(Vec::new());

// 窗口类是否已注册
static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

static CLASS_NAME: &[u16] = &[
    68, 101, 115, 107, 116, 111, 112, 79, 114, 103, 71, 114, 105, 100, 0,
]; // "DesktopOrgGrid"

fn ensure_class_registered() -> HINSTANCE {
    let mut registered = CLASS_REGISTERED.lock().unwrap();
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

    if !*registered {
        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfn_wnd_proc: Some(grid_wnd_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: hinstance,
            h_icon: std::ptr::null_mut(),
            h_cursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as *const u16) },
            hbr_background: std::ptr::null_mut(),
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: CLASS_NAME.as_ptr(),
            h_icon_sm: std::ptr::null_mut(),
        };
        unsafe {
            RegisterClassExW(&wc);
        }
        *registered = true;
        log::info!("格子窗口类已注册");
    }

    hinstance
}

/// 创建一个格子窗口
pub fn create_grid_window(params: GridCreateParams) -> Result<HWND> {
    let hinstance = ensure_class_registered();

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_POPUP | WS_VISIBLE,
            params.x,
            params.y,
            params.width,
            params.height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    };

    if hwnd.is_null() {
        anyhow::bail!("CreateWindowExW 创建格子窗口失败");
    }

    // 设置半透明（整体 alpha）
    unsafe {
        SetLayeredWindowAttributes(hwnd, 0, 230, LWA_ALPHA);
    }

    // 存储数据
    let data = Box::new(GridWindowData {
        title: params.title,
        items: params.items,
        is_dragging: false,
        drag_start_x: 0,
        drag_start_y: 0,
        window_x: params.x,
        window_y: params.y,
        grid_id: params.id,
    });
    let data_ptr = Box::into_raw(data);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
    }

    unsafe {
        ShowWindow(hwnd, SW_SHOW);
    }

    // 注册到全局列表
    if let Ok(mut grids) = GRID_WINDOWS.lock() {
        grids.push(GridWindow {
            hwnd,
            id: params.id,
        });
    }

    log::info!("格子窗口已创建: id={} hwnd={:?}", params.id, hwnd);
    Ok(hwnd)
}

/// 销毁所有格子窗口
pub fn destroy_all_grid_windows() {
    if let Ok(mut grids) = GRID_WINDOWS.lock() {
        for grid in grids.drain(..) {
            unsafe {
                // WM_DESTROY 会负责释放 GridWindowData 和图标
                // 这里只需要销毁窗口
                DestroyWindow(grid.hwnd);
            }
        }
    }
    log::info!("所有格子窗口已销毁");
}

/// 获取格子数量
pub fn get_grid_count() -> usize {
    GRID_WINDOWS.lock().map(|g| g.len()).unwrap_or(0)
}

// ===== 窗口过程 =====

unsafe extern "system" fn grid_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            GetClientRect(hwnd, &mut client);
            let w = client.right;
            let h = client.bottom;

            // 双缓冲
            let mem_dc = CreateCompatibleDC(hdc);
            let mem_bmp = CreateCompatibleBitmap(hdc, w, h);
            let old_bmp = SelectObject(mem_dc, mem_bmp as HGDIOBJ);

            // 背景色
            let bg_color = rgb(30, 35, 50);
            let bg_brush = CreateSolidBrush(bg_color);
            let bg_rect = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            FillRect(mem_dc, &bg_rect, bg_brush);
            DeleteObject(bg_brush as HGDIOBJ);

            // 圆角边框
            let border_pen = CreatePen(PS_SOLID, 2, rgb(80, 100, 150));
            let old_pen = SelectObject(mem_dc, border_pen as HGDIOBJ);
            let null_brush = GetStockObject(NULL_BRUSH);
            let old_brush = SelectObject(mem_dc, null_brush);
            RoundRect(mem_dc, 0, 0, w, h, 16, 16);
            SelectObject(mem_dc, old_pen);
            SelectObject(mem_dc, old_brush);
            DeleteObject(border_pen as HGDIOBJ);

            // 获取数据
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let data = &*data_ptr;

                // 标题栏背景
                let title_h: i32 = 32;
                let title_brush = CreateSolidBrush(rgb(40, 50, 70));
                let title_rect = RECT {
                    left: 1,
                    top: 1,
                    right: w - 1,
                    bottom: title_h,
                };
                FillRect(mem_dc, &title_rect, title_brush);
                DeleteObject(title_brush as HGDIOBJ);

                // 标题文字
                let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
                let title_font =
                    CreateFontW(15, 0, 0, 0, 700, 0, 0, 0, 1, 0, 0, 0, 0, font_name.as_ptr());
                let old_font = SelectObject(mem_dc, title_font as HGDIOBJ);
                SetBkMode(mem_dc, TRANSPARENT_MODE);
                SetTextColor(mem_dc, rgb(220, 230, 255));

                let title_text: Vec<u16> = format!("📁 {} ({})\0", data.title, data.items.len())
                    .encode_utf16()
                    .collect();
                let mut title_draw = RECT {
                    left: 10,
                    top: 0,
                    right: w - 10,
                    bottom: title_h,
                };
                DrawTextW(
                    mem_dc,
                    title_text.as_ptr(),
                    -1,
                    &mut title_draw,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );

                // 图标列表
                let icon_font =
                    CreateFontW(12, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0, font_name.as_ptr());
                SelectObject(mem_dc, icon_font as HGDIOBJ);
                SetTextColor(mem_dc, rgb(200, 210, 220));

                for item in &data.items {
                    // 绘制图标（32x32）
                    if !item.icon_handle.is_null() {
                        DrawIconEx(
                            mem_dc,
                            item.client_x,
                            item.client_y,
                            item.icon_handle,
                            32,
                            32,
                            0,
                            std::ptr::null_mut(),
                            DI_NORMAL,
                        );
                    }

                    // 绘制文件名（图标下方）
                    let name_text: Vec<u16> =
                        format!("{}\0", item.display_name).encode_utf16().collect();
                    let mut name_rect = RECT {
                        left: item.client_x - 10,
                        top: item.client_y + 34,
                        right: item.client_x + 42,
                        bottom: item.client_y + 56,
                    };
                    DrawTextW(
                        mem_dc,
                        name_text.as_ptr(),
                        -1,
                        &mut name_rect,
                        DT_CENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                    );
                }

                SelectObject(mem_dc, old_font);
                DeleteObject(title_font as HGDIOBJ);
                DeleteObject(icon_font as HGDIOBJ);
            }

            // 复制到屏幕
            BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
            SelectObject(mem_dc, old_bmp);
            DeleteObject(mem_bmp as HGDIOBJ);
            DeleteDC(mem_dc);
            EndPaint(hwnd, &ps);
            0
        }

        WM_LBUTTONDOWN => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let data = &mut *data_ptr;
                let mx = get_x_lparam(lparam);
                let my = get_y_lparam(lparam);

                // 只在标题栏区域（y < 32）启动拖拽
                if my < 32 {
                    data.is_dragging = true;
                    let mut cursor = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut cursor);
                    data.drag_start_x = cursor.x;
                    data.drag_start_y = cursor.y;
                    SetCapture(hwnd);
                }
            }
            0
        }

        WM_MOUSEMOVE => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let data = &mut *data_ptr;
                if data.is_dragging {
                    let mut cursor = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut cursor);
                    let dx = cursor.x - data.drag_start_x;
                    let dy = cursor.y - data.drag_start_y;
                    let new_x = data.window_x + dx;
                    let new_y = data.window_y + dy;
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        new_x,
                        new_y,
                        0,
                        0,
                        SWP_NOZORDER | SWP_NOSIZE,
                    );
                }
            }
            0
        }

        WM_LBUTTONUP => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let data = &mut *data_ptr;
                if data.is_dragging {
                    data.is_dragging = false;
                    ReleaseCapture();

                    // 更新保存的窗口位置
                    let mut cursor = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut cursor);
                    let dx = cursor.x - data.drag_start_x;
                    let dy = cursor.y - data.drag_start_y;
                    data.window_x += dx;
                    data.window_y += dy;

                    log::debug!(
                        "格子 {} 移动到新位置: ({}, {})",
                        data.grid_id,
                        data.window_x,
                        data.window_y
                    );
                }
            }
            0
        }

        WM_DESTROY => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let data = Box::from_raw(data_ptr);
                for item in &data.items {
                    if !item.icon_handle.is_null() {
                        crate::desktop::file_icon::destroy_icon(item.icon_handle);
                    }
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
