//! Win32 FFI 类型定义、C 结构体和常量
//!
//! 本模块包含所有 Win32 FFI 所需的类型别名、结构体定义和常量。
//! 被 grid_window、win32_ffi、shell_menu、d2d_render 等模块共享使用。

#![allow(clippy::upper_case_acronyms)]
#![allow(non_snake_case)] // Win32 FFI 结构体字段必须保持 PascalCase 以匹配 Windows API 命名
#![allow(dead_code)] // Win32 FFI 类型/常量库，部分预留供后续 Win32 交互使用

use std::ffi::c_void;

// ===== Windows crate D2D/DW/WIC imports =====
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteTextFormat, IDWriteTextLayout, DWRITE_PARAGRAPH_ALIGNMENT, DWRITE_TEXT_ALIGNMENT,
};

// ===== Raw Win32 Types =====
pub type HWND = *mut c_void;

/// HWND_TOPMOST: Places the window above all non-topmost windows.
/// Value is (HWND)(-1) in Windows API.
pub const HWND_TOPMOST: HWND = -1isize as *mut c_void;

/// SWP_NOOWNERZORDER: Does not change the owner window's position in the Z order.
pub const SWP_NOOWNERZORDER: u32 = 0x0200;
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
pub type HANDLE = *mut c_void;
pub type HIMC = *mut c_void;

/// COM HRESULT 返回值类型
pub type HRESULT = i32;
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
#[derive(Clone, Copy)]
pub struct SIZE {
    pub cx: i32,
    pub cy: i32,
}

pub const TRANSPARENT: i32 = 1;

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
/// 窗口不带 DWM 重定向表面：所有像素都由 DirectComposition 视觉树提供。
///
/// 与 `WS_EX_LAYERED` **互斥**。分层窗口的画面来自 `UpdateLayeredWindow`
/// 提交的 GDI DIB，DComp 的画面来自合成交换链；两者同时存在时
/// DWM 用哪一个没有保证，表现为窗口全黑或闪烁。
pub const WS_EX_NOREDIRECTIONBITMAP: u32 = 0x00200000;
pub const WS_EX_TOOLWINDOW: u32 = 0x00000080;
#[allow(dead_code)]
pub const GWL_EXSTYLE: i32 = -20;
pub const WS_EX_TOPMOST: u32 = 0x00000008;
pub const WS_EX_TRANSPARENT: u32 = 0x00000020;

// ===== Class Styles =====
pub const CS_HREDRAW: u32 = 0x0002;
pub const CS_VREDRAW: u32 = 0x0001;
pub const CS_DBLCLKS: u32 = 0x0008;

// ===== Window Messages =====
pub const WM_NULL: u32 = 0x0000;
pub const WM_SETREDRAW: u32 = 0x000B;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_SIZE: u32 = 0x0005;
pub const WM_NCHITTEST: u32 = 0x0084;
pub const WM_NCLBUTTONDOWN: u32 = 0x00A1;
pub const WM_NCLBUTTONUP: u32 = 0x00A2;
pub const WM_NCLBUTTONDBLCLK: u32 = 0x00A3;
pub const WM_SETCURSOR: u32 = 0x0020;
pub const WM_GETMINMAXINFO: u32 = 0x0024;
pub const WM_DPICHANGED: u32 = 0x02E0;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
#[allow(dead_code)]
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MOUSELEAVE: u32 = 0x02A3;
pub const WM_CAPTURECHANGED: u32 = 0x0215;
pub const VK_LBUTTON: i32 = 0x01;
/// Timer ID for overlay drag polling
pub const TIMER_ID_OVERLAY_DRAG: usize = 0x4F56_0001;
/// Timer ID for grid drag polling
pub const TIMER_ID_GRID_DRAG: usize = 0x4F56_0002;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_TIMER: u32 = 0x0113;
pub const WM_HOTKEY: u32 = 0x0312;
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
pub const MOD_NOREPEAT: u32 = 0x4000;
pub const WM_ACTIVATE: u32 = 0x0006;
pub const WM_WINDOWPOSCHANGING: u32 = 0x0046;
/// 窗口位置已变化（完成后可安全读取新几何并同步玻璃块）
pub const WM_WINDOWPOSCHANGED: u32 = 0x0047;

// ===== Alignment guide constants =====
pub const WM_ENTERSIZEMOVE: u32 = 0x0231;
pub const WM_EXITSIZEMOVE: u32 = 0x0232;
pub const WM_MOVING: u32 = 0x0216;
pub const WM_SIZING: u32 = 0x0214;
pub const TIMER_ID_ALIGNMENT: usize = 0x4F56_0003;
/// Timer ID for overlay hover polling (clears hover state when cursor leaves icon area via alpha=0 pixels)
pub const TIMER_ID_OVERLAY_HOVER: usize = 0x4F56_0004;
/// 分类框液态玻璃刷新定时器（约 30 FPS）
pub const TIMER_ID_LIQUID_GLASS: usize = 0x4F56_0005;
pub const TIMER_ID_OVERLAY_MARQUEE: usize = 0x4F56_0006;

// ===== Hit Test Constants =====
pub const HTCLIENT: i32 = 1;
pub const HTTRANSPARENT: i32 = -1;
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
pub const IDC_IBEAM_ID: u16 = 32513;

// ===== Other Constants =====
#[allow(dead_code)]
pub const LWA_ALPHA: u32 = 0x00000002;
#[allow(dead_code)]
pub const LWA_COLORKEY: u32 = 0x00000001;
pub const SW_SHOW: i32 = 5;
pub const SW_SHOWNOACTIVATE: i32 = 4;
pub const SW_SHOWNA: i32 = 8;
pub const SW_HIDE: i32 = 0;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const SWP_NOACTIVATE: u32 = 0x0010;
/// 扩展样式改动后必须带上它，否则 DWM 不会重建窗口表面
pub const SWP_FRAMECHANGED: u32 = 0x0020;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOSIZE: u32 = 0x0001;
pub const HWND_BOTTOM: HWND = std::ptr::dangling_mut::<c_void>();
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
pub const MF_POPUP: u32 = 0x00000010;
pub const MF_CHECKED: u32 = 0x00000008;
pub const MF_UNCHECKED: u32 = 0x00000000;
pub const MF_RADIOCHECK: u32 = 0x00000200;

// ===== TrackPopupMenu Flags =====
pub const TPM_RETURNCMD: u32 = 0x0100;
pub const TPM_LEFTALIGN: u32 = 0x0000;
pub const TPM_RIGHTALIGN: u32 = 0x0008;
pub const TPM_TOPALIGN: u32 = 0x0000;
pub const TPM_LEFTBUTTON: u32 = 0x0000;

// ===== Hook / Message Constants =====
pub const WH_MOUSE_LL: i32 = 14;
pub const HC_ACTION: i32 = 0;
pub const WM_APP: u32 = 0x8000;
pub const WM_USER_SHOW_DESKTOP_MENU: u32 = WM_APP + 1;
pub const WM_USER_OPEN_MENU: u32 = WM_APP + 2; // 延迟打开分类框菜单
pub const WM_USER_DELETE_BOX: u32 = WM_APP + 3; // 延迟删除分类框（从菜单 PostMessage 触发）
pub const WM_USER_REFRESH_OVERLAY: u32 = WM_APP + 4; // 桌面文件变化后刷新覆盖窗口
pub const WM_QUIT: u32 = 0x0012;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_CREATE: u32 = 0x0001;
pub const WM_CLOSE: u32 = 0x0010;
pub const HWND_MESSAGE: HWND = -3isize as *mut c_void;
pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF0000;

// ===== Registry Constants =====
pub const HKEY_CURRENT_USER: isize = 0x80000001u32 as isize;
pub const KEY_WRITE: u32 = 0x20006;
pub const KEY_READ: u32 = 0x20019;
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

/// GetDpiForMonitor DPI type: effective DPI (accounts for OS scaling)
pub const MDT_EFFECTIVE_DPI: u32 = 0;

/// MonitorFromPoint/MonitorFromWindow: 返回主显示器
///
/// 值为 0x00000001，用于获取主显示器句柄。
pub const MONITOR_DEFAULTTOPRIMARY: u32 = 0x00000001;

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

// =============================================================================
// 自定义分层窗口菜单相关类型
// =============================================================================

/// 菜单项定义（用于自定义分层窗口菜单）
pub struct MenuItem {
    pub label: String,
    pub id: u32,
    pub is_separator: bool,
    pub is_checked: bool,
    pub is_radio: bool,
    pub is_disabled: bool,
    pub children: Vec<MenuItem>,
}

impl MenuItem {
    /// 创建一个普通菜单项
    pub fn new(label: &str, id: u32) -> Self {
        Self {
            label: label.to_string(),
            id,
            is_separator: false,
            is_checked: false,
            is_radio: false,
            is_disabled: false,
            children: Vec::new(),
        }
    }

    /// 创建一个分隔线
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            id: 0,
            is_separator: true,
            is_checked: false,
            is_radio: false,
            is_disabled: false,
            children: Vec::new(),
        }
    }

    /// 创建一个带子菜单的菜单项
    pub fn with_children(label: &str, children: Vec<MenuItem>) -> Self {
        Self {
            label: label.to_string(),
            id: 0,
            is_separator: false,
            is_checked: false,
            is_radio: false,
            is_disabled: false,
            children,
        }
    }

    /// 设置勾选状态
    pub fn checked(mut self, checked: bool) -> Self {
        self.is_checked = checked;
        self
    }

    /// 设置为单选按钮样式
    pub fn radio(mut self, radio: bool) -> Self {
        self.is_radio = radio;
        self
    }

    /// 设置为禁用状态
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.is_disabled = disabled;
        self
    }

    /// 是否有子菜单
    pub fn has_submenu(&self) -> bool {
        !self.children.is_empty()
    }
}

// ===== 键盘消息常量 =====
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_KILLFOCUS: u32 = 0x0008;
pub const WM_IME_COMPOSITION: u32 = 0x010F;
pub const GCS_RESULTSTR: u32 = 0x0800;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SHIFT: i32 = 0x10;
pub const VK_CONTROL: i32 = 0x11;
pub const VK_LSHIFT: i32 = 0xA0;
pub const VK_RSHIFT: i32 = 0xA1;
pub const VK_LCONTROL: i32 = 0xA2;
pub const VK_RCONTROL: i32 = 0xA3;
pub const VK_UP: u32 = 0x26;
pub const VK_DOWN: u32 = 0x28;
pub const VK_LEFT: u32 = 0x25;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_RETURN: u32 = 0x0D;
pub const VK_BACK: u32 = 0x08;
pub const VK_DELETE: u32 = 0x2E;
pub const VK_HOME: u32 = 0x24;
pub const VK_END: u32 = 0x23;

// ===== WM_ACTIVATE 常量 =====
pub const WA_INACTIVE: u32 = 0;
pub const WM_MOUSEACTIVATE: u32 = 0x0021;
pub const MA_NOACTIVATE: i32 = 3;

// ===== SetWindowPos 标志 =====
pub const SWP_SHOWWINDOW: u32 = 0x0040;

/// COLORREF 辅助：从 RGB 创建 COLORREF（BGR 顺序）
pub const fn colorref(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

// ===== ListView 消息常量（用于跨进程操作 SysListView32）=====
pub const LVM_FIRST: u32 = 0x1000;
pub const LVM_GETITEMCOUNT: u32 = LVM_FIRST + 4;
pub const LVM_GETITEMW: u32 = LVM_FIRST + 75;
pub const LVM_GETITEMTEXTW: u32 = LVM_FIRST + 115;
pub const LVM_GETITEMPOSITION: u32 = LVM_FIRST + 16;
pub const LVM_SETITEMPOSITION: u32 = LVM_FIRST + 15;
pub const LVM_SETITEMPOSITION32: u32 = LVM_FIRST + 163;
pub const LVM_GETIMAGELIST: u32 = LVM_FIRST + 2;
pub const LVSIL_NORMAL: usize = 0;
pub const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = LVM_FIRST + 55;
pub const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = LVM_FIRST + 54;

/// ListView 项目掩码
pub const LVIF_TEXT: u32 = 0x0001;
pub const LVIF_IMAGE: u32 = 0x0002;

/// 跨进程内存操作常量
pub const PROCESS_VM_OPERATION: u32 = 0x0008;
pub const PROCESS_VM_READ: u32 = 0x0010;
pub const PROCESS_VM_WRITE: u32 = 0x0020;

pub const MEM_COMMIT: u32 = 0x1000;
pub const MEM_RESERVE: u32 = 0x2000;
pub const MEM_RELEASE: u32 = 0x8000;

pub const PAGE_READWRITE: u32 = 0x04;

/// 隐藏图标时使用的屏幕外 X 坐标
pub const HIDDEN_ICON_X: i32 = -32000;
/// 隐藏图标时使用的屏幕外 Y 坐标
pub const HIDDEN_ICON_Y: i32 = -32000;

/// LVITEMW 结构体（64-bit Windows 布局，60 字节）
///
/// 用于跨进程 SendMessage 操作 SysListView32。
/// 字段布局严格匹配 Windows 64-bit 的 C 结构体对齐规则。
/// 注意：不包含 iGroupId/cColumns/puColumns（后续版本追加字段）。
#[repr(C)]
pub struct LVITEMW {
    pub mask: u32,
    pub i_item: i32,
    pub i_sub_item: i32,
    pub state: u32,
    pub state_mask: u32,
    pub psz_text: *mut u16,
    pub cch_text_max: i32,
    pub i_image: i32,
    pub l_param: isize,
    pub i_indent: i32,
}

// ===== 拖拽相关自定义消息 =====
/// 通知源框：某项已被拖拽到其他框或桌面（从 items 中移除）
pub const WM_USER_DRAG_ITEM_REMOVED: u32 = WM_APP + 10;
/// 通知目标框：外部文件被拖入（添加新项）
pub const WM_USER_DRAG_ITEM_ADDED: u32 = WM_APP + 11;
/// 通知框：拖拽悬停进入（设置高亮）
pub const WM_USER_DRAG_ENTER: u32 = WM_APP + 12;
pub const WM_USER_DESTROY_GUIDE: u32 = WM_APP + 30; // 延迟销毁全屏参考线 overlay
/// 通知框：拖拽悬停离开（清除高亮）
pub const WM_USER_DRAG_LEAVE: u32 = WM_APP + 13;
/// 通知框：拖拽放置完成（处理 pending drop）
pub const WM_USER_DRAG_DROP: u32 = WM_APP + 14;
pub const WM_USER_DEFERRED_POSITION_ICON: u32 = WM_APP + 16; // 异步定位图标到桌面指定位置

/// 异步定位图标到桌面指定位置的信息
pub struct DeferredPositionInfo {
    pub file_path: String,
    pub screen_x: i32,
    pub screen_y: i32,
    pub hwnd: HWND,
}
/// 窗口位置变化信息（WM_WINDOWPOSCHANGING/WM_WINDOWPOSCHANGED 的 lParam）
#[repr(C)]
pub struct WINDOWPOS {
    pub hwnd: HWND,
    pub hwnd_insert_after: HWND,
    pub x: i32,
    pub y: i32,
    pub cx: i32,
    pub cy: i32,
    pub flags: u32,
}

/// 通知分类框：将所有其他分类框推到指定窗口之下（Z-order 管理）
pub const WM_USER_PUSH_GRIDS_BELOW: u32 = WM_APP + 15;
/// 通知目标框：更新拖拽覆盖可视化状态（lparam = 光标屏幕坐标）
pub const WM_USER_DRAG_OVER_UPDATE: u32 = WM_APP + 22;

/// 分类框异步重绘一帧（配置里只改了颜色/圆角等样式时使用）。
///
/// 液态玻璃关闭后 200ms 周期重绘定时器不再装配，静态画面的更新
/// 必须有显式入口，否则控制面板改完样式会看不到变化。
pub const WM_USER_GRID_REDRAW: u32 = WM_APP + 23;

/// 按当前液态玻璃开关重设分类框留白、定时器并重绘。
/// 引擎缩放会话开始/结束转发（wParam=1 开始，0 结束）
pub const WM_USER_GLASS_RESIZE: u32 = WM_APP + 25;
/// 库 FocusHint 转发：宿主代理窗口在自己线程上 SetFocus 自己（同线程调用必成）
pub const WM_USER_GLASS_FOCUS: u32 = WM_APP + 26;
/// 鼠标按键状态标志（合成消息 wparam 用，与 Win32 MK_* 同值）
pub const MK_LBUTTON: usize = 0x0001;
pub const MK_RBUTTON: usize = 0x0002;

pub const WM_USER_CUT_FILE_MOVED: u32 = WM_APP + 17;
/// 通知覆盖窗口：外部文件拖拽放置完成（由 overlay IDropTarget PostMessage 触发）
pub const WM_USER_OVERLAY_DROP: u32 = WM_APP + 18;
pub const WM_USER_OVERLAY_LBUTTONDOWN: u32 = WM_APP + 19; // 鼠标钩子转发的覆盖区域左键按下
pub const WM_USER_OVERLAY_LBUTTONUP: u32 = WM_APP + 20;
pub const WM_USER_DEFERRED_INGEST: u32 = WM_APP + 21; // 异步文件收入仓库（避免阻塞 OLE 回调）
                                                      // 鼠标钩子转发的覆盖区域左键释放

/// 拖拽项信息（用于框间拖拽数据传递）
#[derive(Debug, Clone)]
pub struct DragItemInfo {
    /// 在源框 items 中的索引
    pub index: usize,
    /// 文件路径（.lnk）
    pub file_path: String,
    /// 显示名称
    pub display_name: String,
    /// 图标句柄（目标框需要重新提取）
    pub icon_handle: HICON,
}

// ===== 共享 SysListView32 HWND =====
//
// icon_hider 在隐藏前找到 SysListView32 并保存 HWND 到此全局变量。
// DesktopIconManager 初始化时优先使用此缓存 HWND，
// 避免隐藏后 FindWindowExW 无法找到 SysListView32 的问题。

use std::sync::atomic::AtomicIsize;

/// 缓存的 SysListView32 HWND（由 icon_hider 保存，icon_manager 读取）
pub static SHARED_LISTVIEW_HWND: AtomicIsize = AtomicIsize::new(0);

/// 拖拽启动阈值（像素），鼠标移动超过此距离后启动 OLE 拖拽
pub const DRAG_THRESHOLD: i32 = 5;

// ===== OLE / 拖拽类型 =====
pub type HGLOBAL = *mut c_void;

/// CF_HDROP 标准剪贴板格式 ID
pub const CF_HDROP: u32 = 15;

/// OLE 拖拽效果
pub const DROPEFFECT_NONE: u32 = 0;
pub const DROPEFFECT_COPY: u32 = 1;
pub const DROPEFFECT_MOVE: u32 = 2;
#[allow(dead_code)]
pub const DROPEFFECT_LINK: u32 = 4;

/// FORMATETC 存储介质类型
pub const TYMED_HGLOBAL: i32 = 1;

/// FORMATETC 绘制方向
pub const DVASPECT_CONTENT: u32 = 1;

/// GlobalAlloc 标志
pub const GMEM_MOVEABLE: u32 = 0x0002;
pub const GMEM_ZEROINIT: u32 = 0x0040;
pub const GHND: u32 = GMEM_MOVEABLE | GMEM_ZEROINIT;

/// OLE HRESULT 常量
pub const S_FALSE: HRESULT = 1;
pub const E_NOTIMPL: HRESULT = -2147467263; // 0x80004001
pub const E_NOINTERFACE: HRESULT = -2147467262; // 0x80004002
pub const E_POINTER: HRESULT = -2147467261; // 0x80004003
pub const E_INVALIDARG: HRESULT = -2147467260; // 0x80070057
pub const E_UNEXPECTED: HRESULT = -2147418113; // 0x8000FFFF
pub const E_OUTOFMEMORY: HRESULT = -2147024882; // 0x8007000E
pub const OLE_E_ADVISENOTSUPPORTED: HRESULT = -2147221501; // 0x80040003
pub const DRAGDROP_S_DROP: HRESULT = 0x00040100;
pub const DRAGDROP_S_CANCEL: HRESULT = 0x00040101;
pub const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = 0x00040102;

/// IID_IUnknown = {00000000-0000-0000-C000-000000000046}
pub const IID_IUNKNOWN: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

/// IID_IDataObject = {0000010e-0000-0000-C000-000000000046}
pub const IID_IDATAOBJECT: [u8; 16] = [
    0x0E, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

/// IID_IDropSource = {00000121-0000-0000-C000-000000000046}
pub const IID_IDROPSOURCE: [u8; 16] = [
    0x21, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

/// IID_IDropTarget = {00000122-0000-0000-C000-000000000046}
pub const IID_IDROPTARGET: [u8; 16] = [
    0x22, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

/// IID_IDropTargetHelper = {4657278B-411B-11D2-839A-00C04FD918D0}
pub const IID_IDROPTARGETHELPER: [u8; 16] = [
    0x8B, 0x27, 0x57, 0x46, 0x1B, 0x41, 0xD2, 0x11, 0x83, 0x9A, 0x00, 0xC0, 0x4F, 0xD9, 0x18, 0xD0,
];

/// IID_IDragSourceHelper = {DE5BF786-477A-11D2-839D-00C04FD918D0}
pub const IID_IDRAGSOURCEHELPER: [u8; 16] = [
    0x86, 0xF7, 0x5B, 0xDE, 0x7A, 0x47, 0xD2, 0x11, 0x83, 0x9D, 0x00, 0xC0, 0x4F, 0xD9, 0x18, 0xD0,
];

/// CLSID_DragDropHelper = {4657278A-411B-11D2-839A-00C04FD918D0}
pub const CLSID_DRAGDROPHELPER: [u8; 16] = [
    0x8A, 0x27, 0x57, 0x46, 0x1B, 0x41, 0xD2, 0x11, 0x83, 0x9A, 0x00, 0xC0, 0x4F, 0xD9, 0x18, 0xD0,
];

/// COM 类上下文
pub const CLSCTX_INPROC_SERVER: u32 = 0x1;

/// SHDRAGIMAGE 结构体（IDragSourceHelper 拖拽图像描述）
#[repr(C)]
pub struct SHDRAGIMAGE {
    pub size_drag_image: SIZE,
    pub pt_offset: POINT,
    pub hbmp_drag_image: HBITMAP,
    pub cr_color_key: u32,
}

/// FORMATETC 结构体（OLE 数据格式描述）
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FORMATETC {
    pub cf_format: u32,
    pub ptd: *mut c_void,
    pub dw_aspect: u32,
    pub lindex: i32,
    pub tymed: i32,
}

/// STGMEDIUM 结构体（OLE 数据存储）
#[repr(C)]
#[derive(Clone, Copy)]
pub struct STGMEDIUM {
    pub tymed: u32,
    pub h_global: HGLOBAL,
    pub unk_for_release: *mut c_void,
}

/// DROPFILES 结构体（CF_HDROP 格式头部）
#[repr(C)]
pub struct DROPFILES {
    pub p_files: u32,
    pub pt_x: i32,
    pub pt_y: i32,
    pub f_nc: i32,
    pub f_wide: i32,
}

/// 框间拖拽自定义数据（序列化为 JSON 存储在全局内存中）
///
/// 用于在 IDataObject 中传递源框信息，使目标框可以执行内部移动
/// 而不是文件系统操作。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IkunBoxDragData {
    /// 源分类框窗口句柄（usize 以便序列化）
    pub source_hwnd: usize,
    /// 源分类框 config_id
    pub source_config_id: String,
    /// 被拖拽的项列表（MVP 为单个，预留多选）
    pub items: Vec<IkunDragItemData>,
}

/// 框间拖拽的单项数据
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IkunDragItemData {
    /// 在源框 items 中的索引
    pub index: usize,
    /// 文件路径
    pub file_path: String,
    /// 显示名称
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_colorref_black() {
        assert_eq!(colorref(0, 0, 0), 0x000000);
    }

    #[test]
    fn test_colorref_white() {
        assert_eq!(colorref(255, 255, 255), 0x00FFFFFF);
    }

    #[test]
    fn test_colorref_red() {
        assert_eq!(colorref(255, 0, 0), 0x000000FF); // BGR: R is lowest byte
    }

    #[test]
    fn test_colorref_green() {
        assert_eq!(colorref(0, 255, 0), 0x0000FF00);
    }

    #[test]
    fn test_colorref_blue() {
        assert_eq!(colorref(0, 0, 255), 0x00FF0000); // BGR: B is highest byte
    }

    #[test]
    fn test_colorref_mixed() {
        // rgb(100, 210, 255) → BGR: 255<<16 | 210<<8 | 100
        assert_eq!(colorref(100, 210, 255), 0x00FFD264);
    }

    #[test]
    fn test_colorref_is_const() {
        // 验证可以在 const 上下文中使用
        const C: u32 = colorref(10, 20, 30);
        assert_eq!(C, 0x001E140A);
    }

    #[test]
    fn test_menu_constants() {
        assert_eq!(WM_KEYDOWN, 0x0100);
        assert_eq!(VK_ESCAPE, 0x1B);
        assert_eq!(WA_INACTIVE, 0);
        assert_eq!(TRANSPARENT, 1);
    }

    #[test]
    fn test_edit_mode_constants() {
        // 名称编辑模式使用的 Win32 常量
        assert_eq!(WM_CHAR, 0x0102, "WM_CHAR 字符输入消息");
        assert_eq!(WM_KILLFOCUS, 0x0008, "WM_KILLFOCUS 失去焦点消息");
        assert_eq!(
            WM_NCLBUTTONDBLCLK, 0x00A3,
            "WM_NCLBUTTONDBLCLK 非客户区双击"
        );
        assert_eq!(VK_BACK, 0x08, "退格键");
        assert_eq!(VK_DELETE, 0x2E, "删除键");
        assert_eq!(VK_HOME, 0x24, "Home 键");
        assert_eq!(VK_END, 0x23, "End 键");
    }

    #[test]
    fn test_idc_ibeam_cursor_id() {
        // I 型光标（文本输入模式）
        assert_eq!(IDC_IBEAM_ID, 32513);
        // 确保与其他光标 ID 不冲突
        assert_ne!(IDC_IBEAM_ID, IDC_ARROW_ID);
        assert_ne!(IDC_IBEAM_ID, IDC_HAND_ID);
    }

    #[test]
    fn test_menu_item_builder() {
        let item = MenuItem::new("测试", 42);
        assert_eq!(item.label, "测试");
        assert_eq!(item.id, 42);
        assert!(!item.is_separator);
        assert!(!item.has_submenu());

        let sep = MenuItem::separator();
        assert!(sep.is_separator);

        let with_sub = MenuItem::with_children("排序", vec![MenuItem::new("名称", 1)]);
        assert!(with_sub.has_submenu());
        assert_eq!(with_sub.children.len(), 1);

        let checked = MenuItem::new("递增", 2).checked(true).radio(true);
        assert!(checked.is_checked);
        assert!(checked.is_radio);

        let disabled = MenuItem::new("大图标", 3).disabled(true);
        assert!(disabled.is_disabled);
    }

    #[test]
    fn test_windowpos_struct() {
        // WINDOWPOS repr(C) struct layout verification
        let wp = WINDOWPOS {
            hwnd: std::ptr::null_mut(),
            hwnd_insert_after: std::ptr::null_mut(),
            x: 100,
            y: 200,
            cx: 300,
            cy: 400,
            flags: SWP_NOZORDER | SWP_NOACTIVATE,
        };
        assert_eq!(wp.x, 100);
        assert_eq!(wp.y, 200);
        assert_eq!(wp.cx, 300);
        assert_eq!(wp.cy, 400);
        assert_eq!(wp.flags, SWP_NOZORDER | SWP_NOACTIVATE);
    }

    #[test]
    fn test_windowpos_no_zorder_flag() {
        // SWP_NOZORDER flag should be non-zero and combinable
        assert_ne!(SWP_NOZORDER, 0);
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER;
        assert_ne!(flags & SWP_NOZORDER, 0, "SWP_NOZORDER should be set");
        assert_ne!(flags & SWP_NOMOVE, 0, "SWP_NOMOVE should be set");
        assert_ne!(flags & SWP_NOSIZE, 0, "SWP_NOSIZE should be set");
        assert_eq!(
            flags & SWP_NOACTIVATE,
            0,
            "SWP_NOACTIVATE should NOT be set"
        );
    }

    #[test]
    fn test_zorder_constants() {
        // WM_WINDOWPOSCHANGING should be the standard Win32 value
        assert_eq!(WM_WINDOWPOSCHANGING, 0x0046);
        // Custom message should be in WM_APP range
        assert_eq!(WM_USER_PUSH_GRIDS_BELOW, WM_APP + 15);
        // WM_APP base
        assert_eq!(WM_APP, 0x8000);
    }

    #[test]
    fn test_hwnd_constants() {
        // HWND_BOTTOM should be 1 (below all non-topmost)
        assert_eq!(HWND_BOTTOM as isize, 1);
        // HWND_TOPMOST should be -1 (above all non-topmost)
        assert_eq!(HWND_TOPMOST as isize, -1);
    }
}
pub const DI_NORMAL: u32 = 0x0003;

// ===== SHFileOperationW FFI 类型/常量 =====

/// SHFileOperationW 文件操作结构体
#[repr(C)]
#[allow(non_snake_case)]
pub struct SHFILEOPSTRUCTW {
    pub hwnd: HWND,
    pub w_func: u32,
    pub p_from: *const u16,
    pub p_to: *const u16,
    pub f_flags: u16,
    pub f_any_operations_aborted: i32,
    pub h_name_mappings: *mut c_void,
    pub lpsz_progress_title: *const u16,
}

// SHFileOperationW 操作类型
pub const FO_MOVE: u32 = 0x0001;
pub const FO_COPY: u32 = 0x0002;
pub const FO_DELETE: u32 = 0x0003;

// SHFileOperationW 标志
pub const FOF_NOCONFIRMATION: u16 = 0x0010;
pub const FOF_NOCONFIRMMKDIR: u16 = 0x0200;
