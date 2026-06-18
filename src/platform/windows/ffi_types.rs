//! Win32 FFI 类型定义、C 结构体和常量
//!
//! 本模块包含所有 Win32 FFI 所需的类型别名、结构体定义和常量。
//! 被 grid_window、win32_ffi、shell_menu、d2d_render 等模块共享使用。

#![allow(clippy::upper_case_acronyms)]

use std::ffi::c_void;

// ===== Windows crate D2D/DW/WIC imports =====
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteTextFormat, IDWriteTextLayout, DWRITE_PARAGRAPH_ALIGNMENT, DWRITE_TEXT_ALIGNMENT,
};

// ===== Raw Win32 Types =====
pub type HWND = *mut c_void;
pub type HINSTANCE = *mut c_void;
pub type HDC = *mut c_void;
pub type HBRUSH = *mut c_void;
#[allow(dead_code)]
pub type HPEN = *mut c_void;
pub type HGDIOBJ = *mut c_void;
pub type HCURSOR = *mut c_void;
pub type HBITMAP = *mut c_void;
pub type HICON = *mut c_void;
pub type HMENU = *mut c_void;
pub type HHOOK = *mut c_void;
pub type HMONITOR = *mut c_void;
pub type HOOKPROC =
    unsafe extern "system" fn(nCode: i32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
pub type LPARAM = isize;
pub type WPARAM = usize;
pub type LRESULT = isize;
pub type COLORREF = u32;

#[repr(C)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct PAINTSTRUCT {
    pub hdc: HDC,
    pub f_erase: i32,
    pub rc_paint: RECT,
    pub f_restore: i32,
    pub f_inc_update: i32,
    pub rgb_reserved: [u8; 32],
}

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cb_size: u32,
    pub style: u32,
    pub lpfn_wnd_proc: Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>,
    pub cb_cls_extra: i32,
    pub cb_wnd_extra: i32,
    pub h_instance: HINSTANCE,
    pub h_icon: *mut c_void,
    pub h_cursor: HCURSOR,
    pub hbr_background: HBRUSH,
    pub lpsz_menu_name: *const u16,
    pub lpsz_class_name: *const u16,
    pub h_icon_sm: *mut c_void,
}

#[repr(C)]
pub struct MINMAXINFO {
    pub pt_reserved: POINT,
    pub pt_max_size: POINT,
    pub pt_max_position: POINT,
    pub pt_min_track_size: POINT,
    pub pt_max_track_size: POINT,
}

/// GetIconInfo 用的 ICONINFO 结构体
#[repr(C)]
pub struct ICONINFO {
    pub f_icon: i32,
    pub x_hotspot: u32,
    pub y_hotspot: u32,
    pub hbm_mask: HBITMAP,
    pub hbm_color: HBITMAP,
}

/// 32-bit ARGB pixel（用于 DIB Section）
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Pixel32 {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

// ===== Window Styles =====
pub const WS_POPUP: u32 = 0x80000000;
pub const WS_VISIBLE: u32 = 0x10000000;
pub const WS_CHILD: u32 = 0x40000000;
pub const WS_THICKFRAME: u32 = 0x00040000;
pub const WS_EX_LAYERED: u32 = 0x00080000;
pub const WS_EX_TOOLWINDOW: u32 = 0x00000080;
#[allow(dead_code)]
pub const WS_EX_TOPMOST: u32 = 0x00000008;

// ===== Class Styles =====
pub const CS_HREDRAW: u32 = 0x0002;
pub const CS_VREDRAW: u32 = 0x0001;
pub const CS_DBLCLKS: u32 = 0x0008;

// ===== Window Messages =====
pub const WM_PAINT: u32 = 0x000F;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_SIZE: u32 = 0x0005;
pub const WM_NCHITTEST: u32 = 0x0084;
pub const WM_SETCURSOR: u32 = 0x0020;
pub const WM_GETMINMAXINFO: u32 = 0x0024;
pub const WM_DPICHANGED: u32 = 0x02E0;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
#[allow(dead_code)]
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MOUSELEAVE: u32 = 0x02A3;
pub const WM_TIMER: u32 = 0x0113;

// ===== Hit Test Constants =====
pub const HTCLIENT: i32 = 1;
pub const HTCAPTION: i32 = 2;
pub const HTLEFT: i32 = 10;
pub const HTRIGHT: i32 = 11;
pub const HTTOP: i32 = 12;
pub const HTTOPLEFT: i32 = 13;
pub const HTTOPRIGHT: i32 = 14;
pub const HTBOTTOM: i32 = 15;
pub const HTBOTTOMLEFT: i32 = 16;
pub const HTBOTTOMRIGHT: i32 = 17;

// ===== Cursor IDs =====
pub const IDC_ARROW_ID: u16 = 32512;
pub const IDC_SIZENWSE_ID: u16 = 32642;
pub const IDC_SIZENESW_ID: u16 = 32643;
pub const IDC_SIZEWE_ID: u16 = 32644;
pub const IDC_SIZENS_ID: u16 = 32645;
pub const IDC_HAND_ID: u16 = 32649;

// ===== Other Constants =====
#[allow(dead_code)]
pub const LWA_ALPHA: u32 = 0x00000002;
#[allow(dead_code)]
pub const LWA_COLORKEY: u32 = 0x00000001;
pub const SW_SHOW: i32 = 5;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const GWLP_USERDATA: i32 = -21;
pub const SM_CXSCREEN: i32 = 0;
#[allow(dead_code)]
pub const SM_CYSCREEN: i32 = 1;

// ===== Alpha Blend Constants =====
pub const AC_SRC_OVER: u8 = 0x00;
pub const AC_SRC_ALPHA: u8 = 0x01;

// ===== Menu / Popup Constants =====
pub const MF_STRING: u32 = 0x00000000;
pub const MF_SEPARATOR: u32 = 0x00000800;
pub const MF_BYPOSITION: u32 = 0x00000400;
pub const TPM_RETURNCMD: u32 = 0x0100;

// ===== Hook / Message Constants =====
pub const WH_MOUSE_LL: i32 = 14;
pub const HC_ACTION: i32 = 0;
pub const WM_APP: u32 = 0x8000;
pub const WM_USER_SHOW_DESKTOP_MENU: u32 = WM_APP + 1;
pub const WM_QUIT: u32 = 0x0012;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_CREATE: u32 = 0x0001;
pub const WM_CLOSE: u32 = 0x0010;
pub const HWND_MESSAGE: HWND = -3isize as *mut c_void;
pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF0000;

// ===== Registry Constants =====
pub const HKEY_CURRENT_USER: isize = 0x80000001u32 as isize;
pub const KEY_WRITE: u32 = 0x20006;
pub const REG_SZ: u32 = 1;
pub const REG_OPTION_NON_VOLATILE: u32 = 0;

// ===== Common Controls =====
pub const ICC_STANDARD_CLASSES: u32 = 0x4000;
pub const ICC_WIN95_CLASSES: u32 = 0x00FF;

/// Win32 MSG 结构体（消息泵使用）
#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: u32,
    pub pt: POINT,
}

/// Win32 低级鼠标钩子结构体
#[repr(C)]
pub struct MSLLHOOKSTRUCT {
    pub pt: POINT,
    pub mouseData: u32,
    pub flags: u32,
    pub time: u32,
    pub dwExtraInfo: usize,
}

// ===== Tooltip Constants =====
pub const TTS_ALWAYSTIP: u32 = 0x01;
pub const TTS_NOPREFIX: u32 = 0x02;
pub const TTF_SUBCLASS: u32 = 0x0010;
pub const TTF_IDISHWND: u32 = 0x0001;
pub const TTM_ADDTOOLW: u32 = 0x0432;
pub const TTM_UPDATETIPTEXTW: u32 = 0x0439;
pub const TTM_TRACKPOSITION: u32 = 0x0442;
pub const TTM_TRACKACTIVATE: u32 = 0x0441;
pub const TTM_SETMAXTIPWIDTH: u32 = 0x0418;
pub const ICC_TAB_CLASSES: u32 = 0x00000008;
pub static TOOLTIPS_CLASSW: &[u16] = &[
    116, 111, 111, 108, 116, 105, 112, 115, 95, 99, 108, 97, 115, 115, 51, 50, 0,
]; // "tooltips_class32"

/// Tooltip 工具信息结构体
#[repr(C)]
#[allow(non_snake_case)]
pub struct TOOLINFOW {
    pub cbSize: u32,
    pub uFlags: u32,
    pub hwnd: HWND,
    pub uId: usize,
    pub rect: RECT,
    pub hinst: HINSTANCE,
    pub lpszText: *const u16,
    pub lParam: LPARAM,
    pub lpReserved: *mut c_void,
}

/// Common Controls 初始化结构体
#[repr(C)]
#[allow(non_snake_case)]
pub struct INITCOMMONCONTROLSEX {
    pub dwSize: u32,
    pub dwICC: u32,
}

// ===== GDI DIB Constants =====
pub const DIB_RGB_COLORS: u32 = 0;

// ===== Monitor Constants =====
pub const MONITOR_DEFAULTTONEAREST: u32 = 0x00000002;

/// Win32 MONITORINFO 结构体
#[repr(C)]
pub struct MONITORINFO {
    pub cbSize: u32,
    pub rcMonitor: RECT,
    pub rcWork: RECT,
    pub dwFlags: u32,
}

/// Win32 TRACKMOUSEEVENT 结构体（用于 WM_MOUSELEAVE 跟踪）
#[repr(C)]
pub struct TRACKMOUSEEVENT {
    pub cbSize: u32,
    pub dwFlags: u32,
    pub hwndTrack: HWND,
    pub dwHoverTime: u32,
}

/// TrackMouseEvent 标志：请求 WM_MOUSELEAVE 消息
pub const TME_LEAVE: u32 = 0x00000002;

#[repr(C)]
pub struct BLENDFUNCTION {
    pub blend_op: u8,
    pub blend_flags: u8,
    pub source_constant_alpha: u8,
    pub alpha_format: u8,
}

#[repr(C)]
pub struct BITMAPINFOHEADER {
    pub bi_size: u32,
    pub bi_width: i32,
    pub bi_height: i32,
    pub bi_planes: u16,
    pub bi_bit_count: u16,
    pub bi_compression: u32,
    pub bi_size_image: u32,
    pub bi_x_pels_per_meter: i32,
    pub bi_y_pels_per_meter: i32,
    pub bi_clr_used: u32,
    pub bi_clr_important: u32,
}

// ===== Helpers =====
pub fn get_x_lparam(lp: LPARAM) -> i32 {
    (lp & 0xFFFF) as i16 as i32
}
pub fn get_y_lparam(lp: LPARAM) -> i32 {
    ((lp >> 16) & 0xFFFF) as i16 as i32
}
pub fn scale_for_dpi(px: i32, dpi: u32) -> i32 {
    (px * dpi as i32) / 96
}

/// 将 u8 RGBA 颜色转换为 D2D1_COLOR_F（浮点 premultiplied alpha）
pub fn d2d_color(r: u8, g: u8, b: u8, a: u8) -> D2D1_COLOR_F {
    let af = a as f32 / 255.0;
    D2D1_COLOR_F {
        r: (r as f32 / 255.0) * af,
        g: (g as f32 / 255.0) * af,
        b: (b as f32 / 255.0) * af,
        a: af,
    }
}

/// 创建 D2D 矩形
pub fn d2d_rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

/// 辅助：创建 DirectWrite TextLayout
///
/// 封装 TextLayout 创建 + 对齐设置，返回 Option 便于错误处理。
/// 文字换行模式从 TextFormat 继承（标题 = NO_WRAP，标签 = WRAP）。
pub unsafe fn target_inner_create_text_layout(
    dw_factory: &windows::Win32::Graphics::DirectWrite::IDWriteFactory,
    text: &[u16],
    format: &IDWriteTextFormat,
    max_width: f32,
    max_height: f32,
    text_align: DWRITE_TEXT_ALIGNMENT,
    para_align: DWRITE_PARAGRAPH_ALIGNMENT,
) -> Option<IDWriteTextLayout> {
    let layout = dw_factory
        .CreateTextLayout(text, format, max_width, max_height)
        .ok()?;
    let _ = layout.SetTextAlignment(text_align);
    let _ = layout.SetParagraphAlignment(para_align);
    Some(layout)
}
