//! 格子窗口 - 核心组件（Direct2D/DirectWrite 渲染）
//!
//! 每个收纳格子是一个独立的 WS_POPUP 分层窗口。
//! 使用 Direct2D + DirectWrite 渲染到 DIB，通过 UpdateLayeredWindow 提交。
//!
//! 核心优势（相比旧 GDI 方案）：
//! - DirectWrite 原生 per-pixel alpha，文字边缘平滑无锯齿
//! - Direct2D DrawBitmap 线性插值缩放，图标质量高
//! - FillRoundedRectangle 硬件加速圆角矩形
//! - GPU 加速渲染，resize 更流畅
//!
//! 拖动标题栏 = WM_NCHITTEST 返回 HTCAPTION，Windows 原生移动。
//! 边缘缩放 = WM_NCHITTEST 返回 HTLEFT/HTRIGHT/...，Windows 原生缩放。

use anyhow::Result;
use std::ffi::c_void;
use std::sync::Mutex;

// ===== Windows crate D2D/DW/WIC imports =====
use windows::Win32::Graphics::Direct2D::{
    Common::{D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U},
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_WORD, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

// ===== Raw Win32 Types =====
type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HDC = *mut c_void;
type HBRUSH = *mut c_void;
#[allow(dead_code)]
type HPEN = *mut c_void;
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

/// GetIconInfo 用的 ICONINFO 结构体
#[repr(C)]
struct ICONINFO {
    f_icon: i32,
    x_hotspot: u32,
    y_hotspot: u32,
    hbm_mask: HBITMAP,
    hbm_color: HBITMAP,
}

/// 32-bit ARGB pixel（用于 DIB Section）
#[repr(C)]
#[derive(Clone, Copy)]
struct Pixel32 {
    b: u8,
    g: u8,
    r: u8,
    a: u8,
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
const WM_DPICHANGED: u32 = 0x02E0;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
#[allow(dead_code)]
const WM_LBUTTONUP: u32 = 0x0202;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_RBUTTONUP: u32 = 0x0205;

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

// ===== Other Constants =====
#[allow(dead_code)]
const LWA_ALPHA: u32 = 0x00000002;
#[allow(dead_code)]
const LWA_COLORKEY: u32 = 0x00000001;
const SW_SHOW: i32 = 5;
const SWP_NOZORDER: u32 = 0x0004;
const GWLP_USERDATA: i32 = -21;
const SM_CXSCREEN: i32 = 0;
#[allow(dead_code)]
const SM_CYSCREEN: i32 = 1;

// ===== Alpha Blend Constants =====
const AC_SRC_OVER: u8 = 0x00;
const AC_SRC_ALPHA: u8 = 0x01;

// ===== Mouse Messages =====
const MF_STRING: u32 = 0x00000000;
const MF_SEPARATOR: u32 = 0x00000800;
const TPM_RETURNCMD: u32 = 0x0100;

// ===== Tooltip Constants =====
const TTS_ALWAYSTIP: u32 = 0x01;
const TTS_NOPREFIX: u32 = 0x02;
const TTF_SUBCLASS: u32 = 0x0010;
const TTF_IDISHWND: u32 = 0x0001;
const TTM_ADDTOOLW: u32 = 0x0432;
const TTM_UPDATETIPTEXTW: u32 = 0x0439;
const TTM_TRACKPOSITION: u32 = 0x0442;
const TTM_TRACKACTIVATE: u32 = 0x0441;
const TTM_SETMAXTIPWIDTH: u32 = 0x0418;
const ICC_TAB_CLASSES: u32 = 0x00000008;
static TOOLTIPS_CLASSW: &[u16] = &[
    116, 111, 111, 108, 116, 105, 112, 115, 95, 99, 108, 97, 115, 115, 51, 50, 0,
]; // "tooltips_class32"

/// Tooltip 工具信息结构体
#[repr(C)]
#[allow(non_snake_case)]
struct TOOLINFOW {
    cbSize: u32,
    uFlags: u32,
    hwnd: HWND,
    uId: usize,
    rect: RECT,
    hinst: HINSTANCE,
    lpszText: *const u16,
    lParam: LPARAM,
    lpReserved: *mut c_void,
}

/// Common Controls 初始化结构体
#[repr(C)]
#[allow(non_snake_case)]
struct INITCOMMONCONTROLSEX {
    dwSize: u32,
    dwICC: u32,
}

// ===== GDI DIB Constants =====
const DIB_RGB_COLORS: u32 = 0;

#[repr(C)]
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

// ===== Helpers =====
fn get_x_lparam(lp: LPARAM) -> i32 {
    (lp & 0xFFFF) as i16 as i32
}
fn get_y_lparam(lp: LPARAM) -> i32 {
    ((lp >> 16) & 0xFFFF) as i16 as i32
}
fn scale_for_dpi(px: i32, dpi: u32) -> i32 {
    (px * dpi as i32) / 96
}

/// 将 u8 RGBA 颜色转换为 D2D1_COLOR_F（浮点 premultiplied alpha）
fn d2d_color(r: u8, g: u8, b: u8, a: u8) -> D2D1_COLOR_F {
    let af = a as f32 / 255.0;
    D2D1_COLOR_F {
        r: (r as f32 / 255.0) * af,
        g: (g as f32 / 255.0) * af,
        b: (b as f32 / 255.0) * af,
        a: af,
    }
}

/// 创建 D2D 矩形
fn d2d_rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

// ===== FFI Declarations (non-D2D Win32 APIs) =====
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

    // GDI (DIB management only - rendering now via D2D)
    fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> i32;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
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

    // GDI (icon extraction for D2D conversion)
    fn GetIconInfo(hIcon: HICON, pIconInfo: *mut ICONINFO) -> i32;
    fn GetDIBits(
        hdc: HDC,
        hbmp: HBITMAP,
        uStartScan: u32,
        cScanLines: u32,
        lpvBits: *mut c_void,
        lpbmi: *mut c_void,
        usage: u32,
    ) -> i32;

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

    // Window
    fn GetDesktopWindow() -> HWND;

    // Tooltip / Common Controls
    fn InitCommonControlsEx(lpInitCtrls: *const INITCOMMONCONTROLSEX) -> i32;
}

// ===== Shell Context Menu COM Types =====

type HRESULT = i32;
const S_OK: HRESULT = 0;
#[allow(dead_code)]
const COINIT_APARTMENTTHREADED: u32 = 0x2;

// IID_IShellFolder = {000214E6-0000-0000-C000-000000000046}
const IID_ISHELLFOLDER: [u8; 16] = [
    0xE6, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
// IID_IContextMenu = {000214E4-0000-0000-C000-000000000046}
const IID_ICONTEXTMENU: [u8; 16] = [
    0xE4, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
// IID_IContextMenu2 = {000214F4-0000-0000-C000-000000000046}
const IID_ICONTEXTMENU2: [u8; 16] = [
    0xF4, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
// IID_IContextMenu3 = {BCFCE0A0-EC17-11D0-8D10-00A0C90F2719}
const IID_ICONTEXTMENU3: [u8; 16] = [
    0xA0, 0xE0, 0xCF, 0xBC, 0x17, 0xEC, 0xD0, 0x11, 0x8D, 0x10, 0x00, 0xA0, 0xC9, 0x0F, 0x27, 0x19,
];

#[repr(C)]
struct ITEMIDLIST {
    _opaque: u8,
}

/// IShellFolder vtable
#[repr(C)]
#[allow(non_snake_case)]
struct IShellFolderVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    ParseDisplayName: unsafe extern "system" fn(
        *mut c_void,
        HWND,
        *mut c_void,
        *const u16,
        *mut u32,
        *mut *mut ITEMIDLIST,
        *mut u32,
    ) -> HRESULT,
    EnumObjects: unsafe extern "system" fn(*mut c_void, HWND, u32, *mut *mut c_void) -> HRESULT,
    BindToObject: unsafe extern "system" fn(
        *mut c_void,
        *const ITEMIDLIST,
        *mut c_void,
        *const u8,
        *mut *mut c_void,
    ) -> HRESULT,
    BindToStorage: unsafe extern "system" fn(
        *mut c_void,
        *const ITEMIDLIST,
        *mut c_void,
        *const u8,
        *mut *mut c_void,
    ) -> HRESULT,
    CompareIDs: unsafe extern "system" fn(
        *mut c_void,
        LPARAM,
        *const ITEMIDLIST,
        *const ITEMIDLIST,
    ) -> HRESULT,
    CreateViewObject:
        unsafe extern "system" fn(*mut c_void, HWND, *const u8, *mut *mut c_void) -> HRESULT,
    GetAttributesOf:
        unsafe extern "system" fn(*mut c_void, u32, *const *const ITEMIDLIST, *mut u32) -> HRESULT,
    GetUIObjectOf: unsafe extern "system" fn(
        *mut c_void,
        HWND,
        u32,
        *const *const ITEMIDLIST,
        *const u8,
        *mut u32,
        *mut *mut c_void,
    ) -> HRESULT,
    GetDisplayNameOf:
        unsafe extern "system" fn(*mut c_void, *const ITEMIDLIST, u32, *mut c_void) -> HRESULT,
    SetNameOf: unsafe extern "system" fn(
        *mut c_void,
        HWND,
        *const ITEMIDLIST,
        *const u16,
        u32,
        *mut *mut ITEMIDLIST,
    ) -> HRESULT,
}

#[repr(C)]
struct IShellFolder {
    lpVtbl: *const IShellFolderVtbl,
}

/// IContextMenu vtable
#[repr(C)]
#[allow(non_snake_case)]
struct IContextMenuVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    QueryContextMenu: unsafe extern "system" fn(HMENU, u32, u32, u32, u32) -> HRESULT,
    InvokeCommand: unsafe extern "system" fn(*mut c_void, *const CMINVOKECOMMANDINFO) -> HRESULT,
    GetCommandString:
        unsafe extern "system" fn(*mut c_void, usize, u32, *mut u32, *mut u16, u32) -> HRESULT,
}

#[repr(C)]
struct IContextMenu {
    lpVtbl: *const IContextMenuVtbl,
}

/// IContextMenu2 vtable
#[repr(C)]
#[allow(non_snake_case)]
struct IContextMenu2Vtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    QueryContextMenu: unsafe extern "system" fn(HMENU, u32, u32, u32, u32) -> HRESULT,
    InvokeCommand: unsafe extern "system" fn(*mut c_void, *const CMINVOKECOMMANDINFO) -> HRESULT,
    GetCommandString:
        unsafe extern "system" fn(*mut c_void, usize, u32, *mut u32, *mut u16, u32) -> HRESULT,
    HandleMenuMsg: unsafe extern "system" fn(*mut c_void, u32, WPARAM, LPARAM) -> HRESULT,
}

#[repr(C)]
struct IContextMenu2 {
    lpVtbl: *const IContextMenu2Vtbl,
}

/// IContextMenu3 vtable
#[repr(C)]
#[allow(non_snake_case)]
struct IContextMenu3Vtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    QueryContextMenu: unsafe extern "system" fn(HMENU, u32, u32, u32, u32) -> HRESULT,
    InvokeCommand: unsafe extern "system" fn(*mut c_void, *const CMINVOKECOMMANDINFO) -> HRESULT,
    GetCommandString:
        unsafe extern "system" fn(*mut c_void, usize, u32, *mut u32, *mut u16, u32) -> HRESULT,
    HandleMenuMsg: unsafe extern "system" fn(*mut c_void, u32, WPARAM, LPARAM) -> HRESULT,
    HandleMenuMsg2:
        unsafe extern "system" fn(*mut c_void, u32, WPARAM, LPARAM, *mut LRESULT) -> HRESULT,
}

#[repr(C)]
struct IContextMenu3 {
    lpVtbl: *const IContextMenu3Vtbl,
}

/// CMINVOKECOMMANDINFO（匹配 Windows 64-bit C 布局，40 字节）
#[repr(C)]
struct CMINVOKECOMMANDINFO {
    cb_size: u32,
    f_mask: u32,
    hwnd: HWND,
    lp_verb: *const u8,
    lp_parameters: *const u8,
    lp_directory: *const u8,
    n_show: i32,
    dw_hot_key: u32,
    h_icon: HWND,
}

// Shell Context Menu FFI
extern "system" {
    fn SHGetDesktopFolder(ppshf: *mut *mut c_void) -> HRESULT;
    fn SHParseDisplayName(
        pszName: *const u16,
        pbc: *mut c_void,
        ppidl: *mut *mut ITEMIDLIST,
        sfgaoIn: u32,
        psfgaoOut: *mut u32,
    ) -> HRESULT;
    fn SHBindToParent(
        pidl: *const ITEMIDLIST,
        riid: *const u8,
        ppv: *mut *mut c_void,
        ppidlLast: *mut *const ITEMIDLIST,
    ) -> HRESULT;
    fn ILFree(pidl: *mut ITEMIDLIST);
    fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> HRESULT;
    fn CoUninitialize();
}

const FCIDM_SHVIEWFIRST: u32 = 0x0000;
const MENU_CMD_ID_LAST: u32 = FCIDM_SHVIEWFIRST + 0x7FFF;
const CMF_NORMAL: u32 = 0x00000000;
const CMF_EXPLORE: u32 = 0x00000004;
const CMF_CANRENAME: u32 = 0x00000010;
const CMF_EXTENDED_VERBS: u32 = 0x00000100;
const CMF_ITEMMENU: u32 = 0x00000080;
const CMF_NOVERBS: u32 = 0x00000008;

// BHID_SFUIObject - 通过 IShellItem::BindToHandler 获取 IContextMenu
// {b3a4b685-b6d5-4882-bcf6-f4107100a1e4}
const BHID_SFUIOBJECT: [u8; 16] = [
    0x85, 0xB6, 0xA4, 0xB3, 0xD5, 0xB6, 0x82, 0x48, 0xBC, 0xF6, 0xF4, 0x10, 0x71, 0x00, 0xA1, 0xE4,
];

// IID_IShellItem = {43826d1e-e718-42ee-bc55-a1e261c37bfe}
const IID_ISHELLITEM: [u8; 16] = [
    0x1E, 0x6D, 0x82, 0x43, 0x18, 0xE7, 0xEE, 0x42, 0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE,
];

/// IShellItem vtable（继承 IUnknown）
#[repr(C)]
#[allow(non_snake_case)]
struct IShellItemVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    BindToHandler: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const u8,
        *const u8,
        *mut *mut c_void,
    ) -> HRESULT,
    GetParent: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    GetDisplayName: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> HRESULT,
    GetAttributes: unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> HRESULT,
    Compare: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut i32) -> HRESULT,
}

#[repr(C)]
struct IShellItem {
    lpVtbl: *const IShellItemVtbl,
}

extern "system" {
    fn SHCreateItemFromParsingName(
        pszPath: *const u16,
        pbc: *mut c_void,
        riid: *const u8,
        ppv: *mut *mut c_void,
    ) -> HRESULT;
}

/// 显示文件的 Shell 右键菜单（尝试 windows crate 高级绑定 → ShellExecuteW 回退）
fn show_shell_context_menu(hwnd: HWND, file_path: &str) {
    // 尝试使用 windows crate 获取完整 Shell 上下文菜单
    if try_windows_crate_context_menu(hwnd, file_path) {
        return;
    }
    // 回退：ShellExecuteW 增强菜单
    show_shell_fallback_menu(hwnd, file_path);
}

/// 使用 windows crate 高级 COM 绑定获取完整 Shell 菜单
///
/// 关键修复：使用 SHBindToParent 获取父文件夹和相对 PIDL，
/// 而非将绝对 PIDL 直接传给桌面文件夹的 GetUIObjectOf。
/// 使用 windows crate 自带的 QueryContextMenu/InvokeCommand 方法，
/// 避免手动 vtable 调用导致的 ABI 兼容性问题。
fn try_windows_crate_context_menu(hwnd: HWND, file_path: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{
        IContextMenu, IShellFolder, ILFree, SHBindToParent, SHParseDisplayName,
    };

    unsafe {
        let path_wide: Vec<u16> = file_path.encode_utf16().chain(std::iter::once(0)).collect();

        // 1. 解析路径为绝对 PIDL
        let mut full_pidl = std::ptr::null_mut();
        if SHParseDisplayName(PCWSTR(path_wide.as_ptr()), None, &mut full_pidl, 0, None).is_err()
            || full_pidl.is_null()
        {
            log::warn!("windows crate: SHParseDisplayName 失败");
            return false;
        }

        // 2. 获取父文件夹和相对 PIDL（关键修复）
        //    旧代码直接将绝对 PIDL 传给桌面文件夹的 GetUIObjectOf，
        //    但 GetUIObjectOf 期望的是相对 PIDL（父文件夹的子项）。
        //    对于深层嵌套路径，这会导致 GetUIObjectOf 返回损坏的 IContextMenu，
        //    后续 QueryContextMenu 调用时崩溃 (STATUS_ACCESS_VIOLATION)。
        let mut rel_pidl: *mut windows::Win32::UI::Shell::Common::ITEMIDLIST =
            std::ptr::null_mut();
        let parent: Result<IShellFolder, _> =
            SHBindToParent(full_pidl as *const _, Some(&mut rel_pidl));
        let parent = match parent {
            Ok(f) => f,
            Err(e) => {
                log::warn!("windows crate: SHBindToParent 失败: {:?}", e);
                let _ = ILFree(Some(full_pidl));
                return false;
            }
        };

        if rel_pidl.is_null() {
            log::warn!("windows crate: SHBindToParent 返回空相对 PIDL");
            let _ = ILFree(Some(full_pidl));
            return false;
        }

        // 3. 使用父文件夹 + 相对 PIDL 获取 IContextMenu
        log::debug!("windows crate: 调用 GetUIObjectOf (父文件夹 + 相对 PIDL)");
        let pidl_array: [*const windows::Win32::UI::Shell::Common::ITEMIDLIST; 1] =
            [rel_pidl as *const _];
        let result: Result<IContextMenu, _> = parent.GetUIObjectOf(
            windows::Win32::Foundation::HWND(std::ptr::null_mut()),
            &pidl_array,
            None,
        );

        match result {
            Ok(cm) => {
                log::debug!("windows crate: GetUIObjectOf 成功!");

                // 4. 使用 windows crate 的 QueryContextMenu（安全方法调用，非手动 vtable）
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                let menu = CreatePopupMenu();
                let flags =
                    CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME | CMF_EXTENDED_VERBS | CMF_ITEMMENU;

                let hmenu =
                    windows::Win32::UI::WindowsAndMessaging::HMENU(menu);
                match cm.QueryContextMenu(hmenu, 0, FCIDM_SHVIEWFIRST, MENU_CMD_ID_LAST, flags) {
                    Ok(()) => {
                        log::debug!("windows crate: QueryContextMenu 成功");
                        let cmd = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        if cmd > 0 {
                            // 5. 使用 windows crate 的 InvokeCommand
                            let invoke_info = windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
                                cbSize: std::mem::size_of::<
                                    windows::Win32::UI::Shell::CMINVOKECOMMANDINFO,
                                >()
                                    as u32,
                                fMask: 0,
                                hwnd: windows::Win32::Foundation::HWND(hwnd),
                                lpVerb: windows::core::PCSTR(
                                    ((cmd as u32 - FCIDM_SHVIEWFIRST) as usize) as *const u8,
                                ),
                                lpParameters: windows::core::PCSTR(std::ptr::null()),
                                lpDirectory: windows::core::PCSTR(std::ptr::null()),
                                nShow: SW_SHOW,
                                dwHotKey: 0,
                                hIcon: windows::Win32::Foundation::HANDLE(
                                    std::ptr::null_mut(),
                                ),
                            };
                            if let Err(e) = cm.InvokeCommand(&invoke_info) {
                                log::warn!("windows crate: InvokeCommand 失败: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("windows crate: QueryContextMenu 失败: {:?}", e);
                    }
                }
                DestroyMenu(menu);
                drop(cm); // Release COM object
                let _ = ILFree(Some(full_pidl));
                true
            }
            Err(e) => {
                log::warn!("windows crate: GetUIObjectOf 失败: {:?}", e);
                let _ = ILFree(Some(full_pidl));
                false
            }
        }
    }
}

/// ShellExecuteW 回退菜单
fn show_shell_fallback_menu(hwnd: HWND, file_path: &str) {
    unsafe {
        let path_wide: Vec<u16> = file_path.encode_utf16().chain(std::iter::once(0)).collect();

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let menu = CreatePopupMenu();

        let open_text: Vec<u16> = "打开(&O)\0".encode_utf16().collect();
        let open_dir: Vec<u16> = "打开文件所在位置(&I)\0".encode_utf16().collect();
        let runas_text: Vec<u16> = "以管理员身份运行(&A)\0".encode_utf16().collect();
        let delete_text: Vec<u16> = "删除(&D)\0".encode_utf16().collect();
        let props_text: Vec<u16> = "属性(&R)\0".encode_utf16().collect();

        AppendMenuW(menu, MF_STRING, 1, open_text.as_ptr());
        AppendMenuW(menu, MF_STRING, 2, open_dir.as_ptr());
        let lower = file_path.to_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".lnk") {
            AppendMenuW(menu, MF_STRING, 3, runas_text.as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, 4, delete_text.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, 5, props_text.as_ptr());

        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, std::ptr::null());
        match cmd {
            1 => {
                let op: Vec<u16> = "open\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
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
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            3 => {
                let op: Vec<u16> = "runas\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            4 => {
                let op: Vec<u16> = "delete\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            5 => {
                let op: Vec<u16> = "properties\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            _ => {}
        }
        DestroyMenu(menu);
    }
}

/// 显示桌面背景右键菜单（使用 IShellView::GetItemObject(SVGIO_BACKGROUND) 获取真实桌面菜单）
///
/// 桌面背景右键菜单由 shell 视图 (defview) 提供，而非 IShellFolder 直接提供。
/// 正确路径：IShellFolder → CreateViewObject::<IShellView> → GetItemObject::<IContextMenu>(SVGIO_BACKGROUND)
/// 这会返回与桌面右键完全一致的菜单：查看、排序方式、刷新、新建、显示设置、个性化等。
fn show_desktop_context_menu(hwnd: HWND) {
    use windows::Win32::UI::Shell::{
        IContextMenu, IShellFolder, IShellView, SHGetDesktopFolder, SVGIO_BACKGROUND,
    };

    unsafe {
        // 1. 获取桌面 IShellFolder
        let desktop: Result<IShellFolder, _> = SHGetDesktopFolder();
        let desktop = match desktop {
            Ok(f) => f,
            Err(e) => {
                log::warn!("桌面菜单: SHGetDesktopFolder 失败: {:?}", e);
                show_desktop_fallback_menu(hwnd);
                return;
            }
        };

        // 2. 创建 shell 视图 (IShellView)
        //    CreateViewObject 在桌面文件夹上创建的是 Explorer 视图对象，
        //    它包含了桌面背景菜单所需的所有 shell 扩展。
        let view: Result<IShellView, _> =
            desktop.CreateViewObject(windows::Win32::Foundation::HWND(hwnd));
        let view = match view {
            Ok(v) => v,
            Err(e) => {
                log::warn!("桌面菜单: CreateViewObject(IShellView) 失败: {:?}", e);
                show_desktop_fallback_menu(hwnd);
                return;
            }
        };

        // 3. 从 shell 视图获取背景区域的 IContextMenu
        //    SVGIO_BACKGROUND 表示请求的是文件夹空白区域的上下文菜单，
        //    这正是用户在桌面空白处右键看到的菜单。
        let result: Result<IContextMenu, _> = view.GetItemObject(SVGIO_BACKGROUND);
        match result {
            Ok(cm) => {
                log::debug!("桌面菜单: GetItemObject(SVGIO_BACKGROUND) 成功");
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                let menu = CreatePopupMenu();
                let flags = CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME;
                let hmenu = windows::Win32::UI::WindowsAndMessaging::HMENU(menu);

                match cm.QueryContextMenu(hmenu, 0, FCIDM_SHVIEWFIRST, MENU_CMD_ID_LAST, flags) {
                    Ok(()) => {
                        log::debug!("桌面菜单: QueryContextMenu 成功");
                        let cmd = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        if cmd > 0 {
                            let invoke_info = windows::Win32::UI::Shell::CMINVOKECOMMANDINFO {
                                cbSize: std::mem::size_of::<
                                    windows::Win32::UI::Shell::CMINVOKECOMMANDINFO,
                                >()
                                    as u32,
                                fMask: 0,
                                hwnd: windows::Win32::Foundation::HWND(hwnd),
                                lpVerb: windows::core::PCSTR(
                                    ((cmd as u32 - FCIDM_SHVIEWFIRST) as usize) as *const u8,
                                ),
                                lpParameters: windows::core::PCSTR(std::ptr::null()),
                                lpDirectory: windows::core::PCSTR(std::ptr::null()),
                                nShow: SW_SHOW,
                                dwHotKey: 0,
                                hIcon: windows::Win32::Foundation::HANDLE(std::ptr::null_mut()),
                            };
                            if let Err(e) = cm.InvokeCommand(&invoke_info) {
                                log::warn!("桌面菜单: InvokeCommand 失败: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("桌面菜单: QueryContextMenu 失败: {:?}", e);
                    }
                }
                DestroyMenu(menu);
                drop(cm);
            }
            Err(e) => {
                log::warn!(
                    "桌面菜单: GetItemObject(SVGIO_BACKGROUND) 失败: {:?}",
                    e
                );
                show_desktop_fallback_menu(hwnd);
            }
        }
    }
}

/// 桌面背景右键菜单回退方案（ShellExecuteW 自定义菜单）
fn show_desktop_fallback_menu(hwnd: HWND) {
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let menu = CreatePopupMenu();

        let open_desktop: Vec<u16> = "打开桌面文件夹(&O)\0".encode_utf16().collect();
        let display: Vec<u16> = "显示设置(&D)\0".encode_utf16().collect();
        let personalize: Vec<u16> = "个性化(&P)\0".encode_utf16().collect();

        AppendMenuW(menu, MF_STRING, 1, open_desktop.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, 2, display.as_ptr());
        AppendMenuW(menu, MF_STRING, 3, personalize.as_ptr());

        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, std::ptr::null());
        match cmd {
            1 => {
                // 打开桌面文件夹
                let desktop_path = dirs::desktop_dir().unwrap_or_default();
                let path_wide: Vec<u16> = desktop_path
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let op: Vec<u16> = "explore\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            2 => {
                // 显示设置
                let uri: Vec<u16> = "ms-settings:display\0".encode_utf16().collect();
                let op: Vec<u16> = "open\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    uri.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            3 => {
                // 个性化
                let uri: Vec<u16> = "ms-settings:personalization\0".encode_utf16().collect();
                let op: Vec<u16> = "open\0".encode_utf16().collect();
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    uri.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOW,
                );
            }
            _ => {}
        }
        DestroyMenu(menu);
    }
}

// Tooltip 窗口创建（避免与主 CreateWindowExW 签名冲突）
#[link(name = "comctl32")]
extern "system" {
    #[link_name = "CreateWindowExW"]
    fn CreateWindowExW_tooltip(
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
}

fn create_tooltip_window(parent: HWND) -> HWND {
    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_TAB_CLASSES,
    };
    unsafe { InitCommonControlsEx(&icc) };

    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let tooltip_style = TTS_ALWAYSTIP | TTS_NOPREFIX;
    unsafe {
        CreateWindowExW_tooltip(
            0,
            TOOLTIPS_CLASSW.as_ptr(),
            std::ptr::null(),
            tooltip_style,
            -0x8000, // CW_USEDEFAULT
            -0x8000,
            -0x8000,
            -0x8000,
            parent,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    }
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
    /// 当前 hover 项索引（-1 = 无 hover）
    hovered_item: i32,
    /// Tooltip 窗口句柄
    tooltip_hwnd: HWND,

    // ===== D2D/DirectWrite 设备缓存（窗口级） =====
    d2d_factory: Option<ID2D1Factory>,
    dw_factory: Option<IDWriteFactory>,
    wic_factory: Option<IWICImagingFactory>,
    d2d_target: Option<ID2D1DCRenderTarget>,
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

/// 初始化 D2D/DirectWrite/WIC 设备（首次渲染时调用）
fn init_d2d_devices(data: &mut GridWindowData) {
    // D2D Factory
    if data.d2d_factory.is_none() {
        match unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
        {
            Ok(f) => data.d2d_factory = Some(f),
            Err(e) => {
                log::error!("创建 D2D Factory 失败: {:?}", e);
                return;
            }
        }
    }

    // DirectWrite Factory
    if data.dw_factory.is_none() {
        match unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) } {
            Ok(f) => data.dw_factory = Some(f),
            Err(e) => {
                log::error!("创建 DirectWrite Factory 失败: {:?}", e);
                return;
            }
        }
    }

    // WIC Imaging Factory
    if data.wic_factory.is_none() {
        match unsafe {
            CoCreateInstance::<_, IWICImagingFactory>(
                &CLSID_WICImagingFactory,
                None,
                CLSCTX_INPROC_SERVER,
            )
        } {
            Ok(f) => data.wic_factory = Some(f),
            Err(e) => {
                log::error!("创建 WIC Factory 失败: {:?}", e);
                return;
            }
        }
    }

    // DC Render Target（创建一次，每次渲染时 BindDC 到新 DC）
    if data.d2d_target.is_none() {
        if let Some(ref factory) = data.d2d_factory {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode:
                        windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            match unsafe { factory.CreateDCRenderTarget(&props) } {
                Ok(rt) => {
                    data.d2d_target = Some(rt);
                    log::info!("D2D DC RenderTarget 已创建");
                }
                Err(e) => {
                    log::error!("创建 D2D DC RenderTarget 失败: {:?}", e);
                }
            }
        }
    }
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
    let icon_label_h = scale_for_dpi(36, dpi); // 2 行文字高度（支持换行）
    let cell_spacing = scale_for_dpi(16, dpi);
    let cell_w = icon_size + scale_for_dpi(32, dpi); // 文字区域需要更多宽度（类似原生桌面 ~80px）
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

    // 2. 存储窗口数据（含 D2D/DW 设备缓存）
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
        hovered_item: -1,
        tooltip_hwnd: std::ptr::null_mut(),
        d2d_factory: None,
        dw_factory: None,
        wic_factory: None,
        d2d_target: None,
    });
    let data_ptr = Box::into_raw(data);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
    }

    unsafe {
        ShowWindow(hwnd, SW_SHOW);
    }

    // 创建 tooltip 窗口（仅在有选中项时显示）
    let tooltip_hwnd = create_tooltip_window(hwnd);
    if !tooltip_hwnd.is_null() {
        // 设置最大宽度（防止 tooltip 过宽）
        unsafe {
            SendMessageW(tooltip_hwnd, TTM_SETMAXTIPWIDTH, 0, 400);
        }
        // 注册 tooltip 工具（tracking 模式）
        let empty_text: Vec<u16> = "\0".encode_utf16().collect();
        let mut ti = TOOLINFOW {
            cbSize: std::mem::size_of::<TOOLINFOW>() as u32,
            uFlags: TTF_SUBCLASS | TTF_IDISHWND,
            hwnd,
            uId: 0,
            rect: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            hinst: std::ptr::null_mut(),
            lpszText: empty_text.as_ptr(),
            lParam: 0,
            lpReserved: std::ptr::null_mut(),
        };
        unsafe {
            SendMessageW(
                tooltip_hwnd,
                TTM_ADDTOOLW,
                0,
                &mut ti as *mut TOOLINFOW as LPARAM,
            );
        }
    }

    // 更新 tooltip_hwnd 到 data
    unsafe {
        let data_ref = &mut *data_ptr;
        data_ref.tooltip_hwnd = tooltip_hwnd;
    }

    // 3. 首次渲染（使用 Direct2D + UpdateLayeredWindow）
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

/// 渲染分层窗口内容（Direct2D/DirectWrite → DIB → UpdateLayeredWindow）
///
/// 背景 rgba(1,1,1,0.15) = alpha 38/255 ≈ 15% 不透明
/// 标题文字和图标以完全不透明（alpha=255）绘制
///
/// 重要：使用 GetWindowRect（而非 GetClientRect）获取尺寸。
/// 对 WS_POPUP|WS_THICKFRAME 分层窗口，UpdateLayeredWindow 的 psize 参数
/// 会直接设置窗口尺寸。
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

    // 确保 D2D/DW/WIC 设备已初始化
    init_d2d_devices(data);

    let (Some(ref _d2d_factory), Some(ref dw_factory), Some(ref wic_factory), Some(ref target)) = (
        &data.d2d_factory,
        &data.dw_factory,
        &data.wic_factory,
        &data.d2d_target,
    ) else {
        log::warn!("D2D 设备未初始化，跳过渲染");
        RENDERING.store(false, Ordering::SeqCst);
        return;
    };

    // 使用 GetWindowRect 获取完整窗口尺寸
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

    let ww = w as f32;
    let hh = h as f32;

    // 创建 32 位 ARGB DIB Section（D2D 渲染后端）
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
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    let old_bmp = unsafe { SelectObject(mem_dc, dib as HGDIOBJ) };

    // 绑定 D2D render target 到 memory DC
    let bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    if unsafe { target.BindDC(GdiHDC(mem_dc), &bind_rect) }.is_err() {
        log::error!("BindDC 失败");
        unsafe {
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    // ===== D2D 渲染开始 =====
    unsafe { target.BeginDraw() };

    // 清空为全透明（premultiplied alpha: 所有通道为 0）
    let transparent = D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    unsafe { target.Clear(Some(&transparent)) };

    // ===== 绘制圆角背景（整体，四角圆角）=====
    let bg_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(1, 1, 1, 38), None) };
    if let Ok(ref brush) = bg_brush {
        let corner_radius = 16.0_f32;
        let rounded_rect = D2D1_ROUNDED_RECT {
            rect: d2d_rect(0.0, 0.0, ww, hh),
            radiusX: corner_radius,
            radiusY: corner_radius,
        };
        unsafe { target.FillRoundedRectangle(&rounded_rect, brush) };
    }

    // ===== 绘制标题栏与主体的分隔线 =====
    let title_h = data.title_h;
    let sep_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(100, 100, 100, 100), None) };
    if let Ok(ref brush) = sep_brush {
        let sep_rect = d2d_rect(0.0, (title_h - 1) as f32, ww, title_h as f32);
        unsafe { target.FillRectangle(&sep_rect, brush) };
    }

    // ===== 获取 DPI =====
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };

    // ===== 创建文字画刷（复用于标题和标签）=====
    let white_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(255, 255, 255, 255), None) };
    let shadow_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(0, 0, 0, 200), None) };
    let label_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(250, 250, 255, 255), None) };

    // ===== 标题文字（DirectWrite 渲染）=====
    let title_font_size = scale_for_dpi(15, dpi) as f32;
    let title_format = unsafe {
        let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
        let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
        dw_factory.CreateTextFormat::<_, _, _>(
            windows::core::PCWSTR(font_name.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            title_font_size,
            windows::core::PCWSTR(locale.as_ptr()),
        )
    };

    if let Ok(ref tf) = title_format {
        let _ = unsafe { tf.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP) };
        let title_text: Vec<u16> = format!("📁 {} ({})", data.title, data.items.len())
            .encode_utf16()
            .collect();

        // 创建 TextLayout
        let layout = unsafe {
            target_inner_create_text_layout(
                dw_factory,
                &title_text,
                tf,
                (w - 20) as f32,
                title_h as f32,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
            )
        };

        if let Some(ref layout) = layout {
            // 阴影描边
            let shadow_offset = scale_for_dpi(1, dpi) as f32;
            if let Ok(ref sb) = shadow_brush {
                for &(dx, dy) in &[
                    (shadow_offset, 0.0),
                    (-shadow_offset, 0.0),
                    (0.0, shadow_offset),
                    (0.0, -shadow_offset),
                ] {
                    let origin = D2D_POINT_2F {
                        x: 10.0 + dx,
                        y: dy,
                    };
                    unsafe {
                        target.DrawTextLayout(origin, layout, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
            // 主文字（白色）
            if let Ok(ref wb) = white_brush {
                let origin = D2D_POINT_2F { x: 10.0, y: 0.0 };
                unsafe {
                    target.DrawTextLayout(origin, layout, wb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                }
            }
        }
    }

    // ===== 绘制图标网格 =====
    let icon_size = data.icon_size;
    let cell_w = data.cell_w;
    let cell_h = data.cell_h;
    let cols = data.cols;
    let grid_left = (w - cols * cell_w) / 2;
    let grid_top = title_h + scale_for_dpi(8, dpi);
    let icon_label_h = scale_for_dpi(36, dpi); // 2 行文字高度（支持换行）

    // 图标标签字体
    let label_font_size = scale_for_dpi(13, dpi) as f32;
    let label_format = unsafe {
        let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
        let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
        dw_factory.CreateTextFormat::<_, _, _>(
            windows::core::PCWSTR(font_name.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            label_font_size,
            windows::core::PCWSTR(locale.as_ptr()),
        )
    };

    // 设置标签文字对齐方式
    if let Ok(ref lf) = label_format {
        let _ = unsafe { lf.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) };
        let _ = unsafe { lf.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };
        let _ = unsafe { lf.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP) };
    }

    // 创建省略号裁剪标志（line-clamp 效果：最多 2 行 + "..."）
    let ellipsis_sign = if let Ok(ref lf) = label_format {
        unsafe { dw_factory.CreateEllipsisTrimmingSign(lf).ok() }
    } else {
        None
    };

    // 选中项高亮画刷
    let highlight_brush =
        unsafe { target.CreateSolidColorBrush(&d2d_color(60, 80, 120, 120), None) };
    // Hover 高亮画刷（比选中色浅）
    let hover_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(70, 90, 130, 70), None) };

    for (i, item) in data.items.iter().enumerate() {
        let col = (i as i32) % cols;
        let row = (i as i32) / cols;

        let cell_x = grid_left + col * cell_w;
        let cell_y = grid_top + row * cell_h;
        let is_selected = i as i32 == data.selected_item;
        let is_hovered = i as i32 == data.hovered_item;

        // Hover 背景（比选中浅，仅非选中项显示）
        if is_hovered && !is_selected {
            if let Ok(ref hb) = hover_brush {
                let hl_rect = d2d_rect(
                    cell_x as f32,
                    cell_y as f32,
                    (cell_x + cell_w) as f32,
                    (cell_y + cell_h) as f32,
                );
                unsafe { target.FillRectangle(&hl_rect, hb) };
            }
        }

        // 选中项高亮背景
        if is_selected {
            if let Ok(ref hb) = highlight_brush {
                let hl_rect = d2d_rect(
                    cell_x as f32,
                    cell_y as f32,
                    (cell_x + cell_w) as f32,
                    (cell_y + cell_h) as f32,
                );
                unsafe { target.FillRectangle(&hl_rect, hb) };
            }
        }

        // 绘制图标（HICON → GetDIBits → D2D Bitmap → DrawBitmap）
        let icon_x = cell_x + (cell_w - icon_size) / 2;
        let icon_y = cell_y + 4;
        if !item.icon_handle.is_null() {
            draw_icon_d2d(
                target,
                wic_factory,
                item.icon_handle,
                icon_x,
                icon_y,
                icon_size,
            );
        }

        // 绘制图标标签文字（DirectWrite）
        if let Ok(ref lf) = label_format {
            let label: Vec<u16> = item.display_name.encode_utf16().collect();
            let label_x = cell_x + 2;
            let label_y = icon_y + icon_size + scale_for_dpi(6, dpi);
            let label_w = (cell_w - 4) as f32;

            // 选中项：足够高度显示完整名称；未选中：2 行裁剪
            // 两种状态都使用 NEAR（顶部对齐），确保单行文本与图标间距一致
            let (label_h, para_align) = if is_selected {
                let max_h = scale_for_dpi(36 * 3, dpi);
                (max_h as f32, DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
            } else {
                (icon_label_h as f32, DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
            };

            let layout = unsafe {
                target_inner_create_text_layout(
                    dw_factory,
                    &label,
                    lf,
                    label_w,
                    label_h,
                    DWRITE_TEXT_ALIGNMENT_CENTER,
                    para_align,
                )
            };

            if let Some(ref layout) = layout {
                // 未选中：应用 line-clamp（2 行 + 省略号）
                if !is_selected {
                    if let Some(ref sign) = ellipsis_sign {
                        let trimming = DWRITE_TRIMMING {
                            granularity: DWRITE_TRIMMING_GRANULARITY_WORD,
                            delimiter: 0,
                            delimiterCount: 0,
                        };
                        let _ = unsafe { layout.SetTrimming(&trimming, sign) };
                    }
                }

                // 阴影描边
                let lsh = scale_for_dpi(1, dpi) as f32;
                if let Ok(ref sb) = shadow_brush {
                    for &(dx, dy) in &[(lsh, 0.0), (-lsh, 0.0), (0.0, lsh), (0.0, -lsh)] {
                        let origin = D2D_POINT_2F {
                            x: label_x as f32 + dx,
                            y: label_y as f32 + dy,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, layout, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                        }
                    }
                }
                // 主文字（浅灰白色）
                if let Ok(ref lb) = label_brush {
                    let origin = D2D_POINT_2F {
                        x: label_x as f32,
                        y: label_y as f32,
                    };
                    unsafe {
                        target.DrawTextLayout(origin, layout, lb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
        }
    }

    // ===== D2D 渲染结束 =====
    let mut tag1: u64 = 0;
    let mut tag2: u64 = 0;
    if let Err(e) = unsafe { target.EndDraw(Some(&mut tag1), Some(&mut tag2)) } {
        log::error!("D2D EndDraw 失败: {:?}", e);
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

    // 清理 GDI 资源（D2D target 保留复用）
    unsafe {
        SelectObject(mem_dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }

    RENDERING.store(false, Ordering::SeqCst);
}

/// 辅助：创建 DirectWrite TextLayout
///
/// 封装 TextLayout 创建 + 对齐设置，返回 Option 便于错误处理。
/// 文字换行模式从 TextFormat 继承（标题 = NO_WRAP，标签 = WRAP）。
unsafe fn target_inner_create_text_layout(
    dw_factory: &IDWriteFactory,
    text: &[u16],
    format: &IDWriteTextFormat,
    max_width: f32,
    max_height: f32,
    text_align: windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT,
    para_align: windows::Win32::Graphics::DirectWrite::DWRITE_PARAGRAPH_ALIGNMENT,
) -> Option<IDWriteTextLayout> {
    let layout = dw_factory
        .CreateTextLayout(text, format, max_width, max_height)
        .ok()?;
    let _ = layout.SetTextAlignment(text_align);
    let _ = layout.SetParagraphAlignment(para_align);
    Some(layout)
}

/// 使用 Direct2D 绘制图标（HICON → GetDIBits → premultiplied D2D Bitmap → DrawBitmap）
///
/// 绕过 WIC，直接从 HICON 的颜色位图读取原始 BGRA 像素，
/// 手动转换为 premultiplied alpha 后创建 D2D bitmap，确保边缘透明过渡正确。
fn draw_icon_d2d(
    target: &ID2D1DCRenderTarget,
    _wic_factory: &IWICImagingFactory,
    hicon: HICON,
    x: i32,
    y: i32,
    size: i32,
) {
    // 1. 从 HICON 提取颜色位图
    let mut icon_info = ICONINFO {
        f_icon: 0,
        x_hotspot: 0,
        y_hotspot: 0,
        hbm_mask: std::ptr::null_mut(),
        hbm_color: std::ptr::null_mut(),
    };
    if unsafe { GetIconInfo(hicon, &mut icon_info) } == 0 {
        return;
    }

    // 必须有颜色位图（32-bit ARGB）；monochrome mask 无法正确渲染
    if icon_info.hbm_color.is_null() {
        if !icon_info.hbm_mask.is_null() {
            unsafe { DeleteObject(icon_info.hbm_mask as HGDIOBJ) };
        }
        return;
    }

    // 2. 用 GetDIBits 读取原始 BGRA 像素
    //    第一次查询：bi_bit_count 必须为 0，让系统返回实际格式
    let mut bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: 0,
        bi_height: 0,
        bi_planes: 0,
        bi_bit_count: 0, // 关键：查询时必须为 0
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };

    // 第一次调用获取位图尺寸和格式
    if unsafe {
        GetDIBits(
            screen_dc,
            icon_info.hbm_color,
            0,
            0,
            std::ptr::null_mut(),
            &mut bi as *mut BITMAPINFOHEADER as *mut c_void,
            DIB_RGB_COLORS,
        )
    } == 0
    {
        log::warn!("GetDIBits 查询失败");
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    let bmp_w = bi.bi_width;
    let bmp_h = bi.bi_height.abs(); // 可能是负值（top-down DIB）

    if bmp_w <= 0 || bmp_h <= 0 || bi.bi_bit_count != 32 {
        log::debug!(
            "图标位图格式不支持: {}x{} {}bpp",
            bmp_w,
            bmp_h,
            bi.bi_bit_count
        );
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    // 分配像素缓冲区
    let pixel_count = (bmp_w as usize) * (bmp_h as usize);
    let mut pixels = vec![
        Pixel32 {
            b: 0,
            g: 0,
            r: 0,
            a: 0
        };
        pixel_count
    ];

    // 设置 bi_height 为负值以请求 top-down 输出，确保 bi_bit_count = 32
    bi.bi_height = -bmp_h;
    bi.bi_bit_count = 32;
    bi.bi_compression = 0;

    // 第二次调用读取像素数据
    if unsafe {
        GetDIBits(
            screen_dc,
            icon_info.hbm_color,
            0,
            bmp_h as u32,
            pixels.as_mut_ptr() as *mut c_void,
            &mut bi as *mut BITMAPINFOHEADER as *mut c_void,
            DIB_RGB_COLORS,
        )
    } == 0
    {
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    unsafe { ReleaseDC(std::ptr::null_mut(), screen_dc) };

    // 3. 将 straight alpha (BGRA) 转换为 premultiplied alpha
    //    HBITMAP 像素是 straight alpha: RGB 是原始颜色，A 是透明度
    //    D2D 需要 premultiplied: RGB' = RGB * A/255
    for p in pixels.iter_mut() {
        let a = p.a as u16;
        p.b = ((p.b as u16 * a) / 255) as u8;
        p.g = ((p.g as u16 * a) / 255) as u8;
        p.r = ((p.r as u16 * a) / 255) as u8;
    }

    // 4. 创建 D2D bitmap（显式 premultiplied alpha 格式）
    let bitmap_props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };

    let d2d_bitmap = unsafe {
        target.CreateBitmap(
            D2D_SIZE_U {
                width: bmp_w as u32,
                height: bmp_h as u32,
            },
            Some(pixels.as_ptr() as *const c_void),
            (bmp_w as u32) * 4, // pitch = width * bytes_per_pixel
            &bitmap_props,
        )
    };

    if let Ok(ref bmp) = d2d_bitmap {
        let dest_rect = d2d_rect(x as f32, y as f32, (x + size) as f32, (y + size) as f32);
        unsafe {
            target.DrawBitmap(
                bmp,
                Some(&dest_rect),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }
    }

    // 5. 清理 GDI 位图资源
    if !icon_info.hbm_color.is_null() {
        unsafe { DeleteObject(icon_info.hbm_color as HGDIOBJ) };
    }
    if !icon_info.hbm_mask.is_null() {
        unsafe { DeleteObject(icon_info.hbm_mask as HGDIOBJ) };
    }
}

/// 根据当前窗口尺寸和 DPI 重新计算网格布局参数
fn recalculate_layout(data: &mut GridWindowData, width: i32, height: i32, dpi: u32) {
    data.title_h = scale_for_dpi(40, dpi);
    data.icon_size = scale_for_dpi(48, dpi);
    let icon_label_h = scale_for_dpi(36, dpi); // 2 行文字高度（支持换行）
    let cell_spacing = scale_for_dpi(16, dpi);
    data.cell_w = data.icon_size + scale_for_dpi(32, dpi); // 文字区域需要更多宽度
    data.cell_h = data.icon_size + icon_label_h + cell_spacing;
    let body_w = width - scale_for_dpi(16, dpi);
    data.cols = (body_w / data.cell_w).max(1);
    let _ = height;
}

/// 命中测试：根据鼠标位置获取网格项索引
fn grid_hit_test(data: &GridWindowData, w: i32, click_x: i32, click_y: i32) -> i32 {
    let cols = data.cols;
    let grid_left = (w - cols * data.cell_w) / 2;
    let grid_top = data.title_h + scale_for_dpi(8, 96);

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
        // ===== WM_PAINT（分层窗口使用 UpdateLayeredWindow，此处仅验证绘制）=====
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let _hdc = BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }

        // ===== Hit Test：标题栏拖拽 + 边缘缩放 =====
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

            // 主体区域边缘检测
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

            // 标题栏区域
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
            1
        }

        // ===== 窗口缩放 → 重新计算布局并渲染 =====
        WM_SIZE => {
            if wparam != 0 {
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let mut wr = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetWindowRect(hwnd, &mut wr) };
                let win_w = wr.right - wr.left;
                let win_h = wr.bottom - wr.top;

                if let Some(data) = get_data(hwnd) {
                    recalculate_layout(data, win_w, win_h, dpi);
                }
            }
            render_layered_window(hwnd);
            0
        }

        // ===== DPI 变化（跨显示器移动）=====
        WM_DPICHANGED => {
            let new_dpi = (wparam & 0xFFFF) as u32;
            log::info!("DPI 变化: {} → 重新计算布局", new_dpi);

            let suggested_rect = lparam as *const RECT;
            let new_w = unsafe { (*suggested_rect).right - (*suggested_rect).left };
            let new_h = unsafe { (*suggested_rect).bottom - (*suggested_rect).top };

            if let Some(data) = get_data(hwnd) {
                recalculate_layout(data, new_w, new_h, new_dpi);
            }

            unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    (*suggested_rect).left,
                    (*suggested_rect).top,
                    new_w,
                    new_h,
                    SWP_NOZORDER,
                );
            }

            render_layered_window(hwnd);
            0
        }

        // ===== 最小/最大窗口尺寸 =====
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };
            let title_h = get_data(hwnd)
                .map(|d| d.title_h)
                .unwrap_or(scale_for_dpi(40, dpi));
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.pt_min_track_size = POINT {
                x: scale_for_dpi(200, dpi),
                y: title_h + scale_for_dpi(100, dpi),
            };
            mmi.pt_max_track_size = POINT {
                x: scale_for_dpi(3000, dpi),
                y: scale_for_dpi(3000, dpi),
            };
            0
        }

        // ===== 鼠标移动：hover 效果 + tooltip =====
        WM_MOUSEMOVE => {
            let mx = get_x_lparam(lparam);
            let my = get_y_lparam(lparam);

            let mut need_rerender = false;
            let mut show_tooltip = false;
            let mut tooltip_text = String::new();
            let mut tooltip_x: i32 = 0;
            let mut tooltip_y: i32 = 0;

            if let Some(data) = get_data(hwnd) {
                let mut client = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetClientRect(hwnd, &mut client) };
                let w = client.right;
                let hit = grid_hit_test(data, w, mx, my);

                // 更新 hover 状态
                if hit != data.hovered_item {
                    data.hovered_item = hit;
                    need_rerender = true;
                }

                // 仅在分类框有选中项时显示 tooltip
                if hit >= 0
                    && data.selected_item >= 0
                    && (hit as usize) < data.items.len()
                    && !data.tooltip_hwnd.is_null()
                {
                    tooltip_text = data.items[hit as usize].display_name.clone();
                    show_tooltip = true;
                    // tooltip 位置：鼠标下方偏移（屏幕坐标）
                    let mut pt = POINT { x: 0, y: 0 };
                    unsafe { GetCursorPos(&mut pt) };
                    tooltip_x = pt.x + 16;
                    tooltip_y = pt.y + 16;
                }
            }

            if need_rerender {
                render_layered_window(hwnd);
            }

            // 更新 tooltip（在 render 之后，避免 data 借用冲突）
            if let Some(data) = get_data(hwnd) {
                let tt = data.tooltip_hwnd;
                if !tt.is_null() {
                    if show_tooltip {
                        let text_wide: Vec<u16> = tooltip_text.encode_utf16().collect();
                        let mut ti = TOOLINFOW {
                            cbSize: std::mem::size_of::<TOOLINFOW>() as u32,
                            uFlags: TTF_SUBCLASS | TTF_IDISHWND,
                            hwnd,
                            uId: 0,
                            rect: RECT {
                                left: 0,
                                top: 0,
                                right: 0,
                                bottom: 0,
                            },
                            hinst: std::ptr::null_mut(),
                            lpszText: text_wide.as_ptr(),
                            lParam: 0,
                            lpReserved: std::ptr::null_mut(),
                        };
                        unsafe {
                            SendMessageW(
                                tt,
                                TTM_UPDATETIPTEXTW,
                                0,
                                &mut ti as *mut TOOLINFOW as LPARAM,
                            );
                            SendMessageW(
                                tt,
                                TTM_TRACKPOSITION,
                                0,
                                ((tooltip_x as u32 as LPARAM) & 0xFFFF)
                                    | (((tooltip_y as u32 as LPARAM) & 0xFFFF) << 16),
                            );
                            SendMessageW(
                                tt,
                                TTM_TRACKACTIVATE,
                                1,
                                &mut ti as *mut TOOLINFOW as LPARAM,
                            );
                        }
                    } else {
                        // 隐藏 tooltip
                        let mut ti = TOOLINFOW {
                            cbSize: std::mem::size_of::<TOOLINFOW>() as u32,
                            uFlags: TTF_SUBCLASS | TTF_IDISHWND,
                            hwnd,
                            uId: 0,
                            rect: RECT {
                                left: 0,
                                top: 0,
                                right: 0,
                                bottom: 0,
                            },
                            hinst: std::ptr::null_mut(),
                            lpszText: std::ptr::null(),
                            lParam: 0,
                            lpReserved: std::ptr::null_mut(),
                        };
                        unsafe {
                            SendMessageW(
                                tt,
                                TTM_TRACKACTIVATE,
                                0,
                                &mut ti as *mut TOOLINFOW as LPARAM,
                            );
                        }
                    }
                }
            }
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

            let mut file_path: Option<String> = None;
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
                    if (hit as usize) < data.items.len() {
                        file_path = Some(data.items[hit as usize].file_path.clone());
                    }
                    render_layered_window(hwnd);
                }
            }

            if let Some(ref path) = file_path {
                show_shell_context_menu(hwnd, path);
            } else {
                show_desktop_context_menu(hwnd);
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

                // D2D/DW/WIC 资源通过 Drop 自动释放（windows crate COM 包装器）
                drop(data.d2d_target);
                drop(data.d2d_factory);
                drop(data.dw_factory);
                drop(data.wic_factory);

                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                log::info!("格子窗口 D2D/DW 资源已释放");
            }
            0
        }

        // ===== 默认处理 =====
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
