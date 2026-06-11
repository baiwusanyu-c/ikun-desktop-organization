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
type HPEN = *mut c_void;
type HFONT = *mut c_void;
type HGDIOBJ = *mut c_void;
type HCURSOR = *mut c_void;
type HBITMAP = *mut c_void;
type HICON = *mut c_void;
type HIMAGELIST = *mut c_void;
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
struct INITCOMMONCONTROLSEX {
    dw_size: u32,
    dw_icc: u32,
}

#[repr(C)]
struct LVITEMW {
    mask: u32,
    i_item: i32,
    i_sub_item: i32,
    state: u32,
    state_mask: u32,
    psz_text: *const u16,
    cch_text_max: i32,
    i_image: i32,
    l_param: isize,
    i_indent: i32,
    i_group_id: i32,
    c_columns: u32,
    pu_columns: *mut u32,
}

#[repr(C)]
struct NMHDR {
    hwnd_from: HWND,
    id_from: usize,
    code: u32,
}

#[repr(C)]
struct NMITEMACTIVATE {
    hdr: NMHDR,
    i_item: i32,
    i_sub_item: i32,
    u_new_state: u32,
    u_old_state: u32,
    u_changed: u32,
    pt_action: POINT,
    l_param: isize,
    i_key_flags: i32,
    u_key_flags: u32,
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
const WS_CHILD: u32 = 0x40000000;
const WS_EX_LAYERED: u32 = 0x00080000;
const WS_EX_TOOLWINDOW: u32 = 0x00000080;
const WS_EX_TOPMOST: u32 = 0x00000008;

// ===== Class Styles =====
const CS_HREDRAW: u32 = 0x0002;
const CS_VREDRAW: u32 = 0x0001;
const CS_DBLCLKS: u32 = 0x0008;

// ===== ListView Styles =====
const LVS_ICON: u32 = 0x0000;
const LVS_SHAREIMAGELISTS: u32 = 0x0040;
const LVS_SHOWSELALWAYS: u32 = 0x0800;

// ===== Common Controls =====
const ICC_LISTVIEW_CLASSES: u32 = 0x00000001;
const ILC_COLOR32: u32 = 0x0020;
const ILC_MASK: u32 = 0x0001;

// ===== ListView Messages =====
const LVM_FIRST: u32 = 0x1000;
const LVM_SETIMAGELIST: u32 = LVM_FIRST + 3;
const LVM_INSERTITEMW: u32 = LVM_FIRST + 77;
const LVM_GETNEXTITEM: u32 = LVM_FIRST + 12;
const LVM_SETBKCOLOR: u32 = LVM_FIRST + 1;
const LVM_SETTEXTBKCOLOR: u32 = LVM_FIRST + 38;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = LVM_FIRST + 54;
const LVSIL_NORMAL: usize = 0;

// ===== ListView Extended Styles =====
const LVS_EX_DOUBLEBUFFER: usize = 0x00010000;

// ===== ListView Item Flags =====
const LVIF_TEXT: u32 = 0x0001;
const LVIF_IMAGE: u32 = 0x0002;
const LVNI_SELECTED: usize = 0x0002;

// ===== Window Messages =====
const WM_PAINT: u32 = 0x000F;
const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_NOTIFY: u32 = 0x004E;
const WM_NCHITTEST: u32 = 0x0084;
const WM_SETCURSOR: u32 = 0x0020;
const WM_GETMINMAXINFO: u32 = 0x0024;
#[allow(dead_code)]
const WM_LBUTTONDOWN: u32 = 0x0201;
#[allow(dead_code)]
const WM_LBUTTONUP: u32 = 0x0202;
#[allow(dead_code)]
const WM_MOUSEMOVE: u32 = 0x0200;
#[allow(dead_code)]
const WM_CAPTURECHANGED: u32 = 0x0215;

// ===== Notification Codes (as u32 from -i32) =====
const NM_DBLCLK: u32 = (-3i32) as u32;
const NM_RCLICK: u32 = (-5i32) as u32;

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
const SRCCOPY: u32 = 0x00CC0020;
const TRANSPARENT_MODE: i32 = 1;
const DT_LEFT: u32 = 0x00000000;
const DT_VCENTER: u32 = 0x00000004;
const DT_SINGLELINE: u32 = 0x00000020;
const PS_SOLID: i32 = 0;
const NULL_BRUSH: i32 = 5;
#[allow(dead_code)]
const LWA_ALPHA: u32 = 0x00000002;
#[allow(dead_code)]
const LWA_COLORKEY: u32 = 0x00000001;
const SW_SHOW: i32 = 5;
const SWP_NOZORDER: u32 = 0x0004;
const GWLP_USERDATA: i32 = -21;

// ===== Menu Constants =====
const MF_STRING: u32 = 0x00000000;
const MF_SEPARATOR: u32 = 0x00000800;
const TPM_RETURNCMD: u32 = 0x0100;

// ===== System Metrics =====
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;

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

// Common Controls (comctl32.dll) — 需要显式链接
#[link(name = "comctl32")]
extern "system" {
    fn InitCommonControlsEx(lpInitCtrls: *const INITCOMMONCONTROLSEX) -> i32;
    fn ImageList_Create(cx: i32, cy: i32, flags: u32, initial: i32, grow: i32) -> HIMAGELIST;
    fn ImageList_AddIcon(himl: HIMAGELIST, hicon: HICON) -> i32;
    fn ImageList_Destroy(himl: HIMAGELIST) -> i32;
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
    listview_hwnd: HWND,
    image_list: HIMAGELIST,
    grid_id: usize,
    title_h: i32,
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

static LISTVIEW_CLASS_NAME: &[u16] = &[
    83, 121, 115, 76, 105, 115, 116, 86, 105, 101, 119, 51, 50, 0,
]; // "SysListView32"

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

/// 初始化 Common Controls（确保 SysListView32 可用）
fn ensure_common_controls() {
    let icc = INITCOMMONCONTROLSEX {
        dw_size: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dw_icc: ICC_LISTVIEW_CLASSES,
    };
    unsafe {
        InitCommonControlsEx(&icc);
    }
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
    ensure_common_controls();

    // 先用初始 DPI 创建窗口，创建后查询实际 DPI 并重新计算尺寸
    let initial_dpi: u32 = 96;
    let initial_w = scale_for_dpi(params.width, initial_dpi);
    let initial_h = scale_for_dpi(params.height, initial_dpi);

    // 1. 创建主窗口
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
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
            "DPI 自适应: {} -> 窗口尺寸 {}x{}, title_h={}, icon_size={}",
            dpi,
            scaled_w,
            scaled_h,
            title_h,
            icon_size
        );
    }

    // 窗口整体半透明：alpha=26 约 10% 不透明（用户要求 0.1 透明度）
    unsafe {
        SetLayeredWindowAttributes(hwnd, 0, 26, LWA_ALPHA);
    }

    // 2. 创建 SysListView32 子窗口
    let lv_hwnd = unsafe {
        CreateWindowExW(
            0,
            LISTVIEW_CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | LVS_ICON | LVS_SHAREIMAGELISTS | LVS_SHOWSELALWAYS,
            0,
            title_h,
            scaled_w,
            scaled_h - title_h,
            hwnd,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    };

    if lv_hwnd.is_null() {
        unsafe {
            DestroyWindow(hwnd);
        }
        anyhow::bail!("CreateWindowExW 创建 SysListView32 失败");
    }

    // 3. 创建 ImageList 并填充图标
    let initial_count = params.items.len().max(1) as i32;
    let himl = unsafe {
        ImageList_Create(
            icon_size,
            icon_size,
            ILC_COLOR32 | ILC_MASK,
            initial_count,
            4,
        )
    };

    if himl.is_null() {
        unsafe {
            DestroyWindow(hwnd);
        }
        anyhow::bail!("ImageList_Create 失败");
    }

    for (i, item) in params.items.iter().enumerate() {
        // 添加图标到 ImageList（ImageList 会复制图标数据）
        if !item.icon_handle.is_null() {
            unsafe {
                ImageList_AddIcon(himl, item.icon_handle);
            }
        }

        // 插入 ListView 项
        let text: Vec<u16> = item
            .display_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let lvi = LVITEMW {
            mask: LVIF_TEXT | LVIF_IMAGE,
            i_item: i as i32,
            i_sub_item: 0,
            state: 0,
            state_mask: 0,
            psz_text: text.as_ptr(),
            cch_text_max: 0,
            i_image: i as i32,
            l_param: 0,
            i_indent: 0,
            i_group_id: -1,
            c_columns: 0,
            pu_columns: std::ptr::null_mut(),
        };
        unsafe {
            SendMessageW(
                lv_hwnd,
                LVM_INSERTITEMW,
                0,
                &lvi as *const LVITEMW as LPARAM,
            );
        }
    }

    // 关联 ImageList + 设置 ListView 样式
    unsafe {
        SendMessageW(lv_hwnd, LVM_SETIMAGELIST, LVSIL_NORMAL, himl as LPARAM);

        // 设置背景色与窗口主题统一 rgb(20,25,40)
        let bg = rgb(20, 25, 40);
        SendMessageW(lv_hwnd, LVM_SETBKCOLOR, 0, bg as LPARAM);
        SendMessageW(lv_hwnd, LVM_SETTEXTBKCOLOR, 0, bg as LPARAM);

        // 启用双缓冲（减少闪烁，改善图标 alpha 渲染）
        SendMessageW(
            lv_hwnd,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            LVS_EX_DOUBLEBUFFER,
            LVS_EX_DOUBLEBUFFER as LPARAM,
        );
    }

    // 4. 存储窗口数据
    let data = Box::new(GridWindowData {
        title: params.title,
        items: params.items,
        listview_hwnd: lv_hwnd,
        image_list: himl,
        grid_id: params.id,
        title_h,
    });
    let data_ptr = Box::into_raw(data);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
    }

    unsafe {
        ShowWindow(hwnd, SW_SHOW);
    }

    // 5. 注册到全局列表
    if let Ok(mut grids) = GRID_WINDOWS.lock() {
        grids.push(GridWindow {
            hwnd,
            id: params.id,
        });
    }

    log::info!(
        "格子窗口已创建: id={} title='{}' items={} dpi={}",
        params.id,
        if let Ok(grids) = GRID_WINDOWS.lock() {
            grids.last().map(|_| "ok").unwrap_or("?")
        } else {
            "?"
        },
        unsafe { &*data_ptr }.items.len(),
        dpi
    );
    Ok(hwnd)
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
        // ===== 标题栏绘制 =====
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

            // 背景（深色，配合 LWA_ALPHA 形成半透明效果）
            let bg_color = rgb(20, 25, 40);
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

            // 标题栏
            if let Some(data) = get_data(hwnd) {
                let title_h = data.title_h;
                // 标题栏与主体统一颜色
                let title_brush = CreateSolidBrush(rgb(20, 25, 40));
                let title_rect = RECT {
                    left: 1,
                    top: 1,
                    right: w - 1,
                    bottom: title_h,
                };
                FillRect(mem_dc, &title_rect, title_brush);
                DeleteObject(title_brush as HGDIOBJ);

                // 标题文字（DPI 感知：使用实际 DPI 缩放字体）
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
                let font_size = scale_for_dpi(24, dpi);
                let title_font = CreateFontW(
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
                );
                let old_font = SelectObject(mem_dc, title_font as HGDIOBJ);
                SetBkMode(mem_dc, TRANSPARENT_MODE);
                SetTextColor(mem_dc, rgb(255, 255, 255));

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

                SelectObject(mem_dc, old_font);
                DeleteObject(title_font as HGDIOBJ);
            }

            // 复制到屏幕
            BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
            SelectObject(mem_dc, old_bmp);
            DeleteObject(mem_bmp as HGDIOBJ);
            DeleteDC(mem_dc);
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
            let edge = scale_for_dpi(30, dpi);

            // 主体区域边缘检测（优先于标题栏检测，确保角落可缩放）
            if dy >= title_h {
                let on_left = dx >= 0 && dx < edge;
                let on_right = dx > win_w - edge && dx <= win_w;
                let on_bottom = dy > win_h - edge && dy <= win_h;
                let on_top_body = dy >= title_h && dy < title_h + edge;

                if on_left && on_bottom {
                    return HTBOTTOMLEFT as LRESULT;
                }
                if on_right && on_bottom {
                    return HTBOTTOMRIGHT as LRESULT;
                }
                if on_left && on_top_body {
                    return HTTOPLEFT as LRESULT;
                }
                if on_right && on_top_body {
                    return HTTOPRIGHT as LRESULT;
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

            // 标题栏区域 → 拖拽移动
            if dy >= 0 && dy < title_h {
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

        // ===== 窗口缩放 → 调整 ListView 子窗口 =====
        WM_SIZE => {
            if let Some(data) = get_data(hwnd) {
                let new_w = (lparam & 0xFFFF) as i32;
                let new_h = ((lparam >> 16) & 0xFFFF) as i32;
                let title_h = data.title_h;
                let lv_h = (new_h - title_h).max(0);
                MoveWindow(data.listview_hwnd, 0, title_h, new_w, lv_h, 1);
            }
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

        // ===== ListView 事件通知 =====
        WM_NOTIFY => {
            let nmhdr = &*(lparam as *const NMHDR);

            match nmhdr.code {
                // 双击打开
                NM_DBLCLK => {
                    if let Some(data) = get_data(hwnd) {
                        let nma = &*(lparam as *const NMITEMACTIVATE);
                        let idx = nma.i_item;
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

                // 右键菜单
                NM_RCLICK => {
                    if let Some(data) = get_data(hwnd) {
                        // 获取当前选中项
                        let sel = SendMessageW(
                            data.listview_hwnd,
                            LVM_GETNEXTITEM,
                            (-1isize) as WPARAM,
                            LVNI_SELECTED as LPARAM,
                        ) as i32;

                        if sel >= 0 && (sel as usize) < data.items.len() {
                            let item = &data.items[sel as usize];

                            // 获取光标位置（屏幕坐标）
                            let mut pt = POINT { x: 0, y: 0 };
                            GetCursorPos(&mut pt);

                            // 创建弹出菜单
                            let menu = CreatePopupMenu();
                            let open_text: Vec<u16> = "打开(O)\0".encode_utf16().collect();
                            let open_dir_text: Vec<u16> =
                                "打开文件所在位置(I)\0".encode_utf16().collect();
                            let props_text: Vec<u16> = "属性(R)\0".encode_utf16().collect();

                            AppendMenuW(menu, MF_STRING, 1, open_text.as_ptr());
                            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                            AppendMenuW(menu, MF_STRING, 2, open_dir_text.as_ptr());
                            AppendMenuW(menu, MF_STRING, 3, props_text.as_ptr());

                            let cmd = TrackPopupMenu(
                                menu,
                                TPM_RETURNCMD,
                                pt.x,
                                pt.y,
                                0,
                                hwnd,
                                std::ptr::null(),
                            );

                            match cmd {
                                1 => {
                                    // 打开
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
                                }
                                2 => {
                                    // 打开文件所在位置
                                    let op: Vec<u16> = "explore\0".encode_utf16().collect();
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
                                }
                                3 => {
                                    // 属性
                                    let op: Vec<u16> = "properties\0".encode_utf16().collect();
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
                                }
                                _ => {}
                            }

                            DestroyMenu(menu);
                        }
                    }
                    0
                }

                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }

        // ===== 窗口销毁 =====
        WM_DESTROY => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let data = Box::from_raw(data_ptr);

                // 销毁 ImageList
                if !data.image_list.is_null() {
                    ImageList_Destroy(data.image_list);
                }

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
