//! 格子窗口 - 核心组件
//!
//! 每个收纳格子是一个独立的 WS_POPUP 分层窗口。
//! 内部嵌入 SysListView32 控件渲染图标，原生支持：
//! - 双击打开文件
//! - 右键上下文菜单
//! - 框选多个图标
//! - 键盘导航
//!
//! 拖动标题栏 = WM_NCHITTEST 返回 HTCAPTION，Windows 原生移动。
//! 边缘缩放 = WM_NCHITTEST 返回 HTLEFT/HTRIGHT/...，Windows 原生缩放。

use anyhow::Result;
use std::ffi::c_void;
use std::sync::Mutex;

// ===== Raw Win32 Types =====
type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HDC = *mut c_void;
type HBRUSH = *mut c_void;
#[allow(dead_code)]
type HPEN = *mut c_void;
type HFONT = *mut c_void;
type HGDIOBJ = *mut c_void;
type HCURSOR = *mut c_void;
type HBITMAP = *mut c_void;
type HICON = *mut c_void;
type HMENU = *mut c_void;
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
#[derive(Clone, Copy)]
struct POINT {
    x: i32,
    y: i32,
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

#[repr(C)]
struct MINMAXINFO {
    pt_reserved: POINT,
    pt_max_size: POINT,
    pt_max_position: POINT,
    pt_min_track_size: POINT,
    pt_max_track_size: POINT,
}

// ===== Window Styles =====
const WS_POPUP: u32 = 0x80000000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_THICKFRAME: u32 = 0x00040000;
const WS_EX_LAYERED: u32 = 0x00080000;
const WS_EX_TOOLWINDOW: u32 = 0x00000080;
#[allow(dead_code)]
const WS_EX_TOPMOST: u32 = 0x00000008;

// ===== Class Styles =====
const CS_HREDRAW: u32 = 0x0002;
const CS_VREDRAW: u32 = 0x0001;
const CS_DBLCLKS: u32 = 0x0008;

// ===== Window Messages =====
const WM_PAINT: u32 = 0x000F;
const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_NCHITTEST: u32 = 0x0084;
const WM_SETCURSOR: u32 = 0x0020;
const WM_GETMINMAXINFO: u32 = 0x0024;
// WM_LBUTTONDOWN, WM_LBUTTONDBLCLK, WM_RBUTTONUP defined below

// ===== Hit Test Constants =====
const HTCLIENT: i32 = 1;
const HTCAPTION: i32 = 2;
const HTLEFT: i32 = 10;
const HTRIGHT: i32 = 11;
const HTTOP: i32 = 12;
const HTTOPLEFT: i32 = 13;
const HTTOPRIGHT: i32 = 14;
const HTBOTTOM: i32 = 15;
const HTBOTTOMLEFT: i32 = 16;
const HTBOTTOMRIGHT: i32 = 17;

// ===== Cursor IDs =====
const IDC_ARROW_ID: u16 = 32512;
const IDC_SIZENWSE_ID: u16 = 32642;
const IDC_SIZENESW_ID: u16 = 32643;
const IDC_SIZEWE_ID: u16 = 32644;
const IDC_SIZENS_ID: u16 = 32645;

// ===== GDI Constants =====
const TRANSPARENT_MODE: i32 = 1;
const DT_LEFT: u32 = 0x00000000;
const DT_VCENTER: u32 = 0x00000004;
const DT_SINGLELINE: u32 = 0x00000020;
#[allow(dead_code)]
const LWA_ALPHA: u32 = 0x00000002;
#[allow(dead_code)]
const LWA_COLORKEY: u32 = 0x00000001;
const SW_SHOW: i32 = 5;
const SWP_NOZORDER: u32 = 0x0004;
const GWLP_USERDATA: i32 = -21;

// ===== DrawIconEx Flags =====
const DI_NORMAL: u32 = 0x0003;

// ===== Menu Constants =====
const MF_STRING: u32 = 0x00000000;
const MF_SEPARATOR: u32 = 0x00000800;
const TPM_RETURNCMD: u32 = 0x0100;

// ===== System Metrics =====
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;

// ===== Alpha Blend Constants =====
const AC_SRC_OVER: u8 = 0x00;
const AC_SRC_ALPHA: u8 = 0x01;

// ===== DrawText Format =====
const DT_CENTER: u32 = 0x00000001;
const DT_NOPREFIX: u32 = 0x00000800;
const DT_END_ELLIPSIS: u32 = 0x00008000;

// ===== Mouse Messages =====
const WM_LBUTTONDOWN: u32 = 0x0201;
#[allow(dead_code)]
const WM_LBUTTONUP: u32 = 0x0202;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_RBUTTONUP: u32 = 0x0205;

#[repr(C)]
#[derive(Clone, Copy)]
struct BLENDFUNCTION {
    blend_op: u8,
    blend_flags: u8,
    source_constant_alpha: u8,
    alpha_format: u8,
}

#[repr(C)]
struct BITMAPINFOHEADER {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

/// 32-bit ARGB pixel
#[repr(C)]
#[derive(Clone, Copy)]
struct Pixel32 {
    b: u8,
    g: u8,
    r: u8,
    a: u8,
}

// ===== Helpers =====
fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}
fn get_x_lparam(lp: LPARAM) -> i32 {
    (lp & 0xFFFF) as i16 as i32
}
fn get_y_lparam(lp: LPARAM) -> i32 {
    ((lp >> 16) & 0xFFFF) as i16 as i32
}
fn scale_for_dpi(px: i32, dpi: u32) -> i32 {
    (px * dpi as i32) / 96
}

// ===== FFI Declarations =====
#[allow(dead_code)]
extern "system" {
    // Window management
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
    fn SetCursor(hCursor: HCURSOR) -> HCURSOR;
    fn InvalidateRect(hWnd: HWND, lpRect: *const RECT, bErase: i32) -> i32;
    fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> i32;
    fn GetWindowRect(hWnd: HWND, lpRect: *mut RECT) -> i32;
    #[allow(dead_code)]
    fn ScreenToClient(hWnd: HWND, lpPoint: *mut POINT) -> i32;
    fn GetCursorPos(lpPoint: *mut POINT) -> i32;
    #[allow(dead_code)]
    fn SetCapture(hWnd: HWND) -> HWND;
    #[allow(dead_code)]
    fn ReleaseCapture() -> i32;
    fn MoveWindow(hWnd: HWND, X: i32, Y: i32, nWidth: i32, nHeight: i32, bRepaint: i32) -> i32;
    fn GetDpiForWindow(hwnd: HWND) -> u32;
    fn SendMessageW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn GetDC(hWnd: HWND) -> HDC;
    fn ReleaseDC(hWnd: HWND, hDC: HDC) -> i32;
    fn UpdateLayeredWindow(
        hWnd: HWND,
        hdcDst: HDC,
        pptDst: *const POINT,
        psize: *const POINT,
        hdcSrc: HDC,
        pptSrc: *const POINT,
        crKey: COLORREF,
        pblend: *const BLENDFUNCTION,
        dwFlags: u32,
    ) -> i32;
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

    // GDI
    fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> i32;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn CreateCompatibleBitmap(hdc: HDC, cx: i32, cy: i32) -> HBITMAP;
    fn CreateDIBSection(
        hdc: HDC,
        lpbmi: *const c_void,
        usage: u32,
        ppvBits: *mut *mut c_void,
        hSection: *mut c_void,
        offset: u32,
    ) -> HBITMAP;
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

    // System
    fn GetSystemMetrics(nIndex: i32) -> i32;

    // Shell
    fn ShellExecuteW(
        hwnd: HWND,
        lpOperation: *const u16,
        lpFile: *const u16,
        lpParameters: *const u16,
        lpDirectory: *const u16,
        nShowCmd: i32,
    ) -> HINSTANCE;

    // Menu
    fn CreatePopupMenu() -> HMENU;
    fn AppendMenuW(hMenu: HMENU, uFlags: u32, uIDNewItem: usize, lpNewItem: *const u16) -> i32;
    fn TrackPopupMenu(
        hMenu: HMENU,
        uFlags: u32,
        x: i32,
        y: i32,
        nReserved: i32,
        hwnd: HWND,
        prcRect: *const RECT,
    ) -> i32;
    fn DestroyMenu(hMenu: HMENU) -> i32;
}

// ===== 公共数据结构 =====

/// 格子内的文件项
pub struct GridFileItem {
    /// 文件路径（.lnk 或实际路径，用于 ShellExecuteW 打开）
    pub file_path: String,
    /// 显示名称
    pub display_name: String,
    /// 系统图标句柄（调用者负责提供，格子窗口负责释放）
    pub icon_handle: HICON,
}

unsafe impl Send for GridFileItem {}

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

/// 格子窗口内部数据（存储在 GWLP_USERDATA）
struct GridWindowData {
    title: String,
    items: Vec<GridFileItem>,
    #[allow(dead_code)]
    grid_id: usize,
    title_h: i32,
    /// 单元格宽度
    cell_w: i32,
    /// 单元格高度
    cell_h: i32,
    /// 列数
    cols: i32,
    /// 图标尺寸
    icon_size: i32,
    /// 当前选中项索引（-1 = 无选中）
    selected_item: i32,
}

unsafe impl Send for GridWindowData {}

/// 格子窗口句柄包装
pub struct GridWindow {
    pub hwnd: HWND,
    pub id: usize,
}

unsafe impl Send for GridWindow {}

// 全局格子列表
static GRID_WINDOWS: Mutex<Vec<GridWindow>> = Mutex::new(Vec::new());
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
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
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
        log::info!("格子窗口类已注册（含 CS_DBLCLKS）");
    }

    hinstance
}

/// 获取主屏幕分辨率（物理像素）
pub fn get_screen_resolution() -> (i32, i32) {
    let w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    (w, h)
}

/// 创建一个格子窗口
pub fn create_grid_window(params: GridCreateParams) -> Result<HWND> {
    let hinstance = ensure_class_registered();

    // 先用初始 DPI 创建窗口，创建后查询实际 DPI 并重新计算尺寸
    let initial_dpi: u32 = 96;
    let initial_w = scale_for_dpi(params.width, initial_dpi);
    let initial_h = scale_for_dpi(params.height, initial_dpi);

    // 1. 创建主窗口（WS_EX_LAYERED 用于 UpdateLayeredWindow 逐像素 alpha）
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_POPUP | WS_VISIBLE | WS_THICKFRAME,
            params.x,
            params.y,
            initial_w,
            initial_h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    };

    if hwnd.is_null() {
        anyhow::bail!("CreateWindowExW 创建格子窗口失败");
    }

    // 查询实际 DPI 并重新计算尺寸
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        log::warn!("GetDpiForWindow 返回 0，使用默认 DPI 96");
    }
    let dpi = if dpi > 0 { dpi } else { 96 };

    let scaled_w = scale_for_dpi(params.width, dpi);
    let scaled_h = scale_for_dpi(params.height, dpi);
    let title_h = scale_for_dpi(40, dpi);
    let icon_size = scale_for_dpi(48, dpi);

    // 计算网格布局参数
    let icon_label_h = scale_for_dpi(20, dpi);
    let cell_spacing = scale_for_dpi(16, dpi);
    let cell_w = icon_size + cell_spacing;
    let cell_h = icon_size + icon_label_h + cell_spacing;
    let body_w = scaled_w - scale_for_dpi(16, dpi); // 左右各 8px 内边距
    let cols = (body_w / cell_w).max(1);

    // 如果 DPI 不是 96，调整窗口尺寸
    if dpi != initial_dpi {
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                params.x,
                params.y,
                scaled_w,
                scaled_h,
                SWP_NOZORDER,
            );
        }
        log::info!(
            "DPI 自适应: {} -> 窗口尺寸 {}x{}, title_h={}, icon_size={}, cols={}",
            dpi,
            scaled_w,
            scaled_h,
            title_h,
            icon_size,
            cols
        );
    }

    // 不再使用 SetLayeredWindowAttributes — 改用 UpdateLayeredWindow 逐像素 alpha
    // 这样可以实现背景半透明但文字/图标完全不透明

    // 2. 存储窗口数据
    let data = Box::new(GridWindowData {
        title: params.title,
        items: params.items,
        grid_id: params.id,
        title_h,
        cell_w,
        cell_h,
        cols,
        icon_size,
        selected_item: -1,
    });
    let data_ptr = Box::into_raw(data);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
    }

    unsafe {
        ShowWindow(hwnd, SW_SHOW);
    }

    // 3. 首次渲染（使用 UpdateLayeredWindow 设置分层窗口内容）
    render_layered_window(hwnd);

    // 4. 注册到全局列表
    if let Ok(mut grids) = GRID_WINDOWS.lock() {
        grids.push(GridWindow {
            hwnd,
            id: params.id,
        });
    }

    log::info!(
        "格子窗口已创建: id={} items={} cols={} dpi={}",
        params.id,
        unsafe { &*data_ptr }.items.len(),
        cols,
        dpi
    );
    Ok(hwnd)
}

/// 渲染分层窗口内容（使用 UpdateLayeredWindow 实现逐像素 alpha）
///
/// 背景 rgba(1,1,1,0.15) = alpha 38/255 ≈ 15% 不透明
/// 标题文字和图标以完全不透明（alpha=255）绘制，不受背景透明度影响
///
/// 重要：使用 GetWindowRect（而非 GetClientRect）获取尺寸。
/// 对 WS_POPUP|WS_THICKFRAME 分层窗口，UpdateLayeredWindow 的 psize 参数
/// 会直接设置窗口尺寸。如果 DIB 使用 GetClientRect（可能小于窗口尺寸），
/// 每次渲染都会让窗口缩小一个 sizing border 宽度，导致持续缩小的反馈循环。
fn render_layered_window(hwnd: HWND) {
    // 递归保护：防止 UpdateLayeredWindow 触发 WM_SIZE → 再次进入本函数
    use std::sync::atomic::{AtomicBool, Ordering};
    static RENDERING: AtomicBool = AtomicBool::new(false);
    if RENDERING.swap(true, Ordering::SeqCst) {
        return; // 已在渲染中，跳过
    }

    let data = unsafe {
        match get_data(hwnd) {
            Some(d) => d,
            None => {
                RENDERING.store(false, Ordering::SeqCst);
                return;
            }
        }
    };

    // 使用 GetWindowRect 获取完整窗口尺寸（含 sizing border）
    // 对 WS_POPUP 分层窗口，这就是 UpdateLayeredWindow 需要的正确尺寸
    let mut wr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut wr) };
    let w = wr.right - wr.left;
    let h = wr.bottom - wr.top;
    if w <= 0 || h <= 0 {
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    log::debug!(
        "渲染分层窗口: window_rect={}x{}, title_h={}",
        w,
        h,
        data.title_h
    );

    // 创建 32 位 ARGB DIB Section（用于逐像素 alpha 控制）
    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    let mem_dc = unsafe { CreateCompatibleDC(screen_dc) };

    let bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: w,
        bi_height: -h, // 自上而下
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
            0,
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
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    let old_bmp = unsafe { SelectObject(mem_dc, dib as HGDIOBJ) };

    // 清空为全透明
    let total_pixels = (w as usize) * (h as usize);
    let pixel_slice = unsafe { std::slice::from_raw_parts_mut(bits, total_pixels) };
    for p in pixel_slice.iter_mut() {
        *p = Pixel32 {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        };
    }

    // ===== 绘制半透明背景 rgba(1,1,1,0.15) =====
    let bg_alpha: u8 = 38; // 0.15 * 255 ≈ 38
    fill_rect_alpha(bits, w, h, 0, 0, w, h, 1, 1, 1, bg_alpha);

    // ===== 绘制圆角边框（完全不透明） =====
    draw_rounded_border_alpha(bits, w, h, 0, 0, w, h, 16, 2, 80, 100, 150);

    // ===== 绘制标题栏 =====
    let title_h = data.title_h;
    // 标题栏背景与主体相同 rgba(1,1,1,0.15)
    fill_rect_alpha(bits, w, h, 1, 1, w - 2, title_h, 1, 1, 1, bg_alpha);

    // 标题文字（完全不透明，白色）
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
    let font_size = scale_for_dpi(18, dpi);
    let title_font = unsafe {
        CreateFontW(
            font_size,
            0,
            0,
            0,
            700,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            font_name.as_ptr(),
        )
    };
    let old_font = unsafe { SelectObject(mem_dc, title_font as HGDIOBJ) };
    unsafe {
        SetBkMode(mem_dc, TRANSPARENT_MODE);
        SetTextColor(mem_dc, rgb(255, 255, 255));
    }

    let title_text: Vec<u16> = format!("📁 {} ({})\0", data.title, data.items.len())
        .encode_utf16()
        .collect();
    let mut title_draw = RECT {
        left: 10,
        top: 0,
        right: w - 10,
        bottom: title_h,
    };
    unsafe {
        DrawTextW(
            mem_dc,
            title_text.as_ptr(),
            -1,
            &mut title_draw,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }

    unsafe {
        SelectObject(mem_dc, old_font);
        DeleteObject(title_font as HGDIOBJ);
    }

    // ===== 绘制图标网格 =====
    let icon_size = data.icon_size;
    let cell_w = data.cell_w;
    let cell_h = data.cell_h;
    let cols = data.cols;
    let grid_left = (w - cols * cell_w) / 2; // 居中排列
    let grid_top = title_h + scale_for_dpi(8, dpi);
    let icon_label_h = scale_for_dpi(20, dpi);

    // 图标标签字体
    let label_font_size = scale_for_dpi(12, dpi);
    let label_font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
    let label_font = unsafe {
        CreateFontW(
            label_font_size,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            label_font_name.as_ptr(),
        )
    };
    let old_label_font = unsafe { SelectObject(mem_dc, label_font as HGDIOBJ) };
    unsafe {
        SetBkMode(mem_dc, TRANSPARENT_MODE);
        SetTextColor(mem_dc, rgb(230, 230, 230));
    }

    for (i, item) in data.items.iter().enumerate() {
        let col = (i as i32) % cols;
        let row = (i as i32) / cols;

        let cell_x = grid_left + col * cell_w;
        let cell_y = grid_top + row * cell_h;

        // 选中项高亮背景
        if i as i32 == data.selected_item {
            fill_rect_alpha(bits, w, h, cell_x, cell_y, cell_w, cell_h, 60, 80, 120, 120);
        }

        // 绘制图标（完全不透明）
        let icon_x = cell_x + (cell_w - icon_size) / 2;
        let icon_y = cell_y + 4;
        if !item.icon_handle.is_null() {
            unsafe {
                DrawIconEx(
                    mem_dc,
                    icon_x,
                    icon_y,
                    item.icon_handle,
                    icon_size,
                    icon_size,
                    0,
                    std::ptr::null_mut(),
                    DI_NORMAL,
                );
            }
        }

        // 绘制图标标签文字（完全不透明）
        let label: Vec<u16> = item
            .display_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut label_rect = RECT {
            left: cell_x + 2,
            top: icon_y + icon_size + 2,
            right: cell_x + cell_w - 2,
            bottom: icon_y + icon_size + 2 + icon_label_h,
        };
        unsafe {
            DrawTextW(
                mem_dc,
                label.as_ptr(),
                -1,
                &mut label_rect,
                DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }
    }

    unsafe {
        SelectObject(mem_dc, old_label_font);
        DeleteObject(label_font as HGDIOBJ);
    }

    // ===== 调用 UpdateLayeredWindow 提交渲染结果 =====
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
    }

    // 清理 GDI 资源
    unsafe {
        SelectObject(mem_dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }

    RENDERING.store(false, Ordering::SeqCst);
}

/// 填充矩形区域（逐像素 alpha）
#[allow(clippy::too_many_arguments)]
fn fill_rect_alpha(
    pixels: *mut Pixel32,
    bmp_w: i32,
    bmp_h: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) {
    let color = Pixel32 { r, g, b, a };
    let x_start = x.max(0);
    let y_start = y.max(0);
    let x_end = (x + w).min(bmp_w);
    let y_end = (y + h).min(bmp_h);

    for py in y_start..y_end {
        let row_offset = (py as usize) * (bmp_w as usize);
        for px in x_start..x_end {
            let idx = row_offset + (px as usize);
            unsafe {
                *pixels.add(idx) = color;
            }
        }
    }
}

/// 绘制圆角边框（逐像素，完全不透明）
#[allow(clippy::too_many_arguments)]
fn draw_rounded_border_alpha(
    pixels: *mut Pixel32,
    bmp_w: i32,
    bmp_h: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    border_width: i32,
    r: u8,
    g: u8,
    b: u8,
) {
    let color = Pixel32 { r, g, b, a: 255 };
    let x_end = (x + w).min(bmp_w);
    let y_end = (y + h).min(bmp_h);

    for py in y.max(0)..y_end {
        for px in x.max(0)..x_end {
            let lx = px - x;
            let ly = py - y;

            // 检查是否在圆角矩形边框上
            let on_border = is_on_rounded_rect_border(lx, ly, w, h, radius, border_width);
            if on_border {
                let idx = (py as usize) * (bmp_w as usize) + (px as usize);
                unsafe {
                    *pixels.add(idx) = color;
                }
            }
        }
    }
}

/// 判断点是否在圆角矩形的边框上
fn is_on_rounded_rect_border(
    lx: i32,
    ly: i32,
    w: i32,
    h: i32,
    radius: i32,
    border_width: i32,
) -> bool {
    // 简化实现：检查点是否在外圆角矩形和内圆角矩形之间
    let in_outer = is_in_rounded_rect(lx, ly, w, h, radius);
    let in_inner = is_in_rounded_rect(
        lx - border_width,
        ly - border_width,
        w - 2 * border_width,
        h - 2 * border_width,
        (radius - border_width).max(0),
    );
    in_outer && !in_inner
}

/// 判断点是否在圆角矩形内部
fn is_in_rounded_rect(lx: i32, ly: i32, w: i32, h: i32, radius: i32) -> bool {
    if lx < 0 || ly < 0 || lx >= w || ly >= h {
        return false;
    }
    let r = radius.min(w / 2).min(h / 2);

    // 检查四个角
    if lx < r && ly < r {
        // 左上角
        let dx = r - lx;
        let dy = r - ly;
        return dx * dx + dy * dy <= r * r;
    }
    if lx >= w - r && ly < r {
        // 右上角
        let dx = lx - (w - r - 1);
        let dy = r - ly;
        return dx * dx + dy * dy <= r * r;
    }
    if lx < r && ly >= h - r {
        // 左下角
        let dx = r - lx;
        let dy = ly - (h - r - 1);
        return dx * dx + dy * dy <= r * r;
    }
    if lx >= w - r && ly >= h - r {
        // 右下角
        let dx = lx - (w - r - 1);
        let dy = ly - (h - r - 1);
        return dx * dx + dy * dy <= r * r;
    }

    // 非角落区域，只要在矩形内即可
    true
}

/// 命中测试：根据鼠标位置获取网格项索引
fn grid_hit_test(data: &GridWindowData, w: i32, click_x: i32, click_y: i32) -> i32 {
    let cols = data.cols;
    let grid_left = (w - cols * data.cell_w) / 2;
    let grid_top = data.title_h + 8;

    if click_y < grid_top || click_x < grid_left {
        return -1;
    }

    let col = (click_x - grid_left) / data.cell_w;
    let row = (click_y - grid_top) / data.cell_h;

    if col < 0 || col >= cols {
        return -1;
    }

    let idx = row * cols + col;
    if idx >= 0 && (idx as usize) < data.items.len() {
        idx
    } else {
        -1
    }
}

/// 销毁所有格子窗口
pub fn destroy_all_grid_windows() {
    if let Ok(mut grids) = GRID_WINDOWS.lock() {
        for grid in grids.drain(..) {
            unsafe {
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

/// 从 HWND 获取 GridWindowData 引用
unsafe fn get_data<'a>(hwnd: HWND) -> Option<&'a mut GridWindowData> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

unsafe extern "system" fn grid_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // ===== WM_PAINT（分层窗口使用 UpdateLayeredWindow，此处仅验证绘制） =====
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let _hdc = BeginPaint(hwnd, &mut ps);
            // 分层窗口内容由 render_layered_window + UpdateLayeredWindow 提供
            // WM_PAINT 仅用于验证无效区域，防止消息循环
            EndPaint(hwnd, &ps);
            0
        }

        // ===== Hit Test：标题栏拖拽 + 边缘缩放（让 Windows 自动处理） =====
        WM_NCHITTEST => {
            let screen_x = get_x_lparam(lparam);
            let screen_y = get_y_lparam(lparam);

            let mut wr = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe { GetWindowRect(hwnd, &mut wr) };
            let dx = screen_x - wr.left;
            let dy = screen_y - wr.top;
            let win_w = wr.right - wr.left;
            let win_h = wr.bottom - wr.top;

            let title_h = get_data(hwnd).map(|d| d.title_h).unwrap_or(40);
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };
            let edge = scale_for_dpi(8, dpi);

            let on_left = dx >= 0 && dx < edge;
            let on_right = dx > win_w - edge && dx <= win_w;
            let on_bottom = dy > win_h - edge && dy <= win_h;

            // 主体区域边缘检测（不含顶部：顶部 resize 由 header 接管）
            if dy >= title_h {
                if on_left && on_bottom {
                    return HTBOTTOMLEFT as LRESULT;
                }
                if on_right && on_bottom {
                    return HTBOTTOMRIGHT as LRESULT;
                }
                if on_left {
                    return HTLEFT as LRESULT;
                }
                if on_right {
                    return HTRIGHT as LRESULT;
                }
                if on_bottom {
                    return HTBOTTOM as LRESULT;
                }
            }

            // 标题栏区域：边缘可缩放（阈值更小），其余区域拖拽移动
            if dy >= 0 && dy < title_h {
                let h_edge = scale_for_dpi(8, dpi);
                let on_h_left = dx >= 0 && dx < h_edge;
                let on_h_right = dx > win_w - h_edge && dx <= win_w;
                let on_h_top = dy >= 0 && dy < h_edge;

                if on_h_top && on_h_left {
                    return HTTOPLEFT as LRESULT;
                }
                if on_h_top && on_h_right {
                    return HTTOPRIGHT as LRESULT;
                }
                if on_h_top {
                    return HTTOP as LRESULT;
                }
                if on_h_left {
                    return HTLEFT as LRESULT;
                }
                if on_h_right {
                    return HTRIGHT as LRESULT;
                }
                return HTCAPTION as LRESULT;
            }

            HTCLIENT as LRESULT
        }

        // ===== 光标设置 =====
        WM_SETCURSOR => {
            let hit_test = (lparam & 0xFFFF) as i16 as i32;
            let cursor_id = match hit_test {
                HTLEFT | HTRIGHT => IDC_SIZEWE_ID,
                HTTOP | HTBOTTOM => IDC_SIZENS_ID,
                HTTOPLEFT | HTBOTTOMRIGHT => IDC_SIZENWSE_ID,
                HTTOPRIGHT | HTBOTTOMLEFT => IDC_SIZENESW_ID,
                _ => {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
            };
            let cursor = LoadCursorW(std::ptr::null_mut(), cursor_id as *const u16);
            SetCursor(cursor);
            1 // 已处理
        }

        // ===== 窗口缩放 → 重新渲染分层窗口 =====
        WM_SIZE => {
            render_layered_window(hwnd);
            0
        }

        // ===== 最小/最大窗口尺寸（DPI 感知）=====
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };
            let title_h = get_data(hwnd)
                .map(|d| d.title_h)
                .unwrap_or(scale_for_dpi(40, dpi));
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.pt_min_track_size = POINT {
                x: scale_for_dpi(200, dpi),
                y: title_h + scale_for_dpi(100, dpi), // 标题栏 + 至少 100px 内容区
            };
            mmi.pt_max_track_size = POINT {
                x: scale_for_dpi(1200, dpi),
                y: scale_for_dpi(1600, dpi),
            };
            0
        }

        // ===== 鼠标左键：选中项 =====
        WM_LBUTTONDOWN => {
            let click_x = get_x_lparam(lparam);
            let click_y = get_y_lparam(lparam);
            let mut need_rerender = false;

            if let Some(data) = get_data(hwnd) {
                let mut client = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                GetClientRect(hwnd, &mut client);
                let w = client.right;
                let hit = grid_hit_test(data, w, click_x, click_y);
                if hit != data.selected_item {
                    data.selected_item = hit;
                    need_rerender = true;
                }
            }

            if need_rerender {
                render_layered_window(hwnd);
            }
            0
        }

        // ===== 鼠标左键双击：打开文件 =====
        WM_LBUTTONDBLCLK => {
            let click_x = get_x_lparam(lparam);
            let click_y = get_y_lparam(lparam);

            if let Some(data) = get_data(hwnd) {
                let mut client = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                GetClientRect(hwnd, &mut client);
                let w = client.right;
                let idx = grid_hit_test(data, w, click_x, click_y);

                if idx >= 0 && (idx as usize) < data.items.len() {
                    let item = &data.items[idx as usize];
                    let op: Vec<u16> = "open\0".encode_utf16().collect();
                    let path: Vec<u16> = item
                        .file_path
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect();
                    ShellExecuteW(
                        hwnd,
                        op.as_ptr(),
                        path.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        SW_SHOW,
                    );
                    log::info!("双击打开: {}", item.display_name);
                }
            }
            0
        }

        // ===== 鼠标右键：弹出上下文菜单 =====
        WM_RBUTTONUP => {
            let click_x = get_x_lparam(lparam);
            let click_y = get_y_lparam(lparam);

            // 先选中右键点击的项
            let mut selected_idx = -1i32;
            if let Some(data) = get_data(hwnd) {
                let mut client = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                GetClientRect(hwnd, &mut client);
                let w = client.right;
                let hit = grid_hit_test(data, w, click_x, click_y);
                if hit >= 0 {
                    data.selected_item = hit;
                    selected_idx = hit;
                    render_layered_window(hwnd);
                }
            }

            if selected_idx >= 0 {
                // 获取数据中选中项的信息（需要重新获取因为 get_data 借用规则）
                let file_path;
                let _display_name;
                if let Some(data) = get_data(hwnd) {
                    if (selected_idx as usize) < data.items.len() {
                        let item = &data.items[selected_idx as usize];
                        file_path = item.file_path.clone();
                        _display_name = item.display_name.clone();
                    } else {
                        return 0;
                    }
                } else {
                    return 0;
                }

                // 获取光标位置（屏幕坐标）
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);

                // 创建弹出菜单
                let menu = CreatePopupMenu();
                let open_text: Vec<u16> = "打开(O)\0".encode_utf16().collect();
                let open_dir_text: Vec<u16> = "打开文件所在位置(I)\0".encode_utf16().collect();
                let props_text: Vec<u16> = "属性(R)\0".encode_utf16().collect();

                AppendMenuW(menu, MF_STRING, 1, open_text.as_ptr());
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                AppendMenuW(menu, MF_STRING, 2, open_dir_text.as_ptr());
                AppendMenuW(menu, MF_STRING, 3, props_text.as_ptr());

                let cmd =
                    TrackPopupMenu(menu, TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, std::ptr::null());

                let path: Vec<u16> = file_path.encode_utf16().chain(std::iter::once(0)).collect();

                match cmd {
                    1 => {
                        let op: Vec<u16> = "open\0".encode_utf16().collect();
                        ShellExecuteW(
                            hwnd,
                            op.as_ptr(),
                            path.as_ptr(),
                            std::ptr::null(),
                            std::ptr::null(),
                            SW_SHOW,
                        );
                    }
                    2 => {
                        let op: Vec<u16> = "explore\0".encode_utf16().collect();
                        ShellExecuteW(
                            hwnd,
                            op.as_ptr(),
                            path.as_ptr(),
                            std::ptr::null(),
                            std::ptr::null(),
                            SW_SHOW,
                        );
                    }
                    3 => {
                        let op: Vec<u16> = "properties\0".encode_utf16().collect();
                        ShellExecuteW(
                            hwnd,
                            op.as_ptr(),
                            path.as_ptr(),
                            std::ptr::null(),
                            std::ptr::null(),
                            SW_SHOW,
                        );
                    }
                    _ => {}
                }

                DestroyMenu(menu);
            }
            0
        }

        // ===== 窗口销毁 =====
        WM_DESTROY => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let data = Box::from_raw(data_ptr);

                // 释放图标句柄
                for item in &data.items {
                    if !item.icon_handle.is_null() {
                        crate::desktop::file_icon::destroy_icon(item.icon_handle);
                    }
                }

                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            0
        }

        // ===== 默认处理 =====
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
