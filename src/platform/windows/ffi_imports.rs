//! Win32 FFI 函数声明和 Shell COM 类型
//!
//! 本模块包含所有非 D2D 的 Win32 API FFI 声明，以及 Shell 上下文菜单相关的 COM 类型。
//! 被 grid_window、shell_menu、d2d_render 等模块使用。

#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(clippy::upper_case_acronyms)]

use super::ffi_types::*;
use std::ffi::c_void;

// ===== FFI Declarations (non-D2D Win32 APIs) =====
#[allow(dead_code)]
extern "system" {
    // Window management
    pub fn GetModuleHandleW(lpModuleName: *const u16) -> HINSTANCE;
    pub fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
    pub fn CreateWindowExW(
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
    pub fn SetLayeredWindowAttributes(hwnd: HWND, crKey: COLORREF, bAlpha: u8, dwFlags: u32)
        -> i32;
    pub fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> i32;
    pub fn SetWindowPos(
        hWnd: HWND,
        hWndInsertAfter: HWND,
        X: i32,
        Y: i32,
        cx: i32,
        cy: i32,
        uFlags: u32,
    ) -> i32;
    pub fn DestroyWindow(hWnd: HWND) -> i32;
    pub fn DefWindowProcW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    pub fn SetWindowLongPtrW(hWnd: HWND, nIndex: i32, dwNewLong: isize) -> isize;
    pub fn GetWindowLongPtrW(hWnd: HWND, nIndex: i32) -> isize;
    pub fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
    pub fn SetCursor(hCursor: HCURSOR) -> HCURSOR;
    pub fn InvalidateRect(hWnd: HWND, lpRect: *const RECT, bErase: i32) -> i32;
    pub fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> i32;
    pub fn GetWindowRect(hWnd: HWND, lpRect: *mut RECT) -> i32;
    #[allow(dead_code)]
    pub fn ScreenToClient(hWnd: HWND, lpPoint: *mut POINT) -> i32;
    pub fn GetCursorPos(lpPoint: *mut POINT) -> i32;
    #[allow(dead_code)]
    pub fn SetCapture(hWnd: HWND) -> HWND;
    #[allow(dead_code)]
    pub fn ReleaseCapture() -> i32;
    pub fn TrackMouseEvent(lpEventTrack: *mut TRACKMOUSEEVENT) -> i32;
    pub fn MoveWindow(hWnd: HWND, X: i32, Y: i32, nWidth: i32, nHeight: i32, bRepaint: i32) -> i32;
    pub fn GetDpiForWindow(hwnd: HWND) -> u32;
    pub fn SendMessageW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    pub fn GetDC(hWnd: HWND) -> HDC;
    pub fn ReleaseDC(hWnd: HWND, hDC: HDC) -> i32;
    pub fn UpdateLayeredWindow(
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
    pub fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    pub fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> i32;
    pub fn CreateCompatibleDC(hdc: HDC) -> HDC;
    pub fn CreateDIBSection(
        hdc: HDC,
        lpbmi: *const c_void,
        usage: u32,
        ppvBits: *mut *mut c_void,
        hSection: *mut c_void,
        offset: u32,
    ) -> HBITMAP;
    pub fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    pub fn DeleteObject(ho: HGDIOBJ) -> i32;
    pub fn DeleteDC(hdc: HDC) -> i32;

    // GDI (icon extraction for D2D conversion)
    pub fn GetIconInfo(hIcon: HICON, pIconInfo: *mut ICONINFO) -> i32;
    pub fn GetDIBits(
        hdc: HDC,
        hbmp: HBITMAP,
        uStartScan: u32,
        cScanLines: u32,
        lpvBits: *mut c_void,
        lpbmi: *mut c_void,
        usage: u32,
    ) -> i32;

    // System
    pub fn GetSystemMetrics(nIndex: i32) -> i32;

    // Shell
    pub fn ShellExecuteW(
        hwnd: HWND,
        lpOperation: *const u16,
        lpFile: *const u16,
        lpParameters: *const u16,
        lpDirectory: *const u16,
        nShowCmd: i32,
    ) -> HINSTANCE;

    // Menu
    pub fn CreatePopupMenu() -> HMENU;
    pub fn AppendMenuW(hMenu: HMENU, uFlags: u32, uIDNewItem: usize, lpNewItem: *const u16) -> i32;
    /// InsertMenuW — 在指定位置插入菜单项（MF_BYPOSITION 时 uPosition 为索引）
    pub fn InsertMenuW(
        hMenu: HMENU,
        uPosition: usize,
        uFlags: u32,
        uIDNewItem: usize,
        lpNewItem: *const u16,
    ) -> i32;
    pub fn TrackPopupMenu(
        hMenu: HMENU,
        uFlags: u32,
        x: i32,
        y: i32,
        nReserved: i32,
        hwnd: HWND,
        prcRect: *const RECT,
    ) -> i32;
    pub fn DestroyMenu(hMenu: HMENU) -> i32;
    pub fn GetMenuItemCount(hMenu: HMENU) -> i32;

    // Window management (additional)
    pub fn SetForegroundWindow(hWnd: HWND) -> i32;
    pub fn EnableWindow(hWnd: HWND, bEnable: i32) -> i32;
    pub fn GetParent(hWnd: HWND) -> HWND;
    pub fn GetClassNameW(hWnd: HWND, lpClassName: *mut u16, nMaxCount: i32) -> i32;
    pub fn WindowFromPoint(pt: POINT) -> HWND;
    pub fn PostMessageW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> i32;
    pub fn PostThreadMessageW(idThread: u32, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> i32;
    pub fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> HWND;

    // Message pump
    pub fn GetMessageW(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: u32, wMsgFilterMax: u32) -> i32;
    pub fn TranslateMessage(lpMsg: *const MSG) -> i32;
    pub fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;

    // Hooks
    pub fn SetWindowsHookExW(
        idHook: i32,
        lpfn: HOOKPROC,
        hmod: HINSTANCE,
        dwThreadId: u32,
    ) -> HHOOK;
    pub fn UnhookWindowsHookEx(hhk: HHOOK) -> i32;
    pub fn CallNextHookEx(hhk: HHOOK, nCode: i32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;

    // Common Controls (InitCommonControlsEx already declared above)

    // Registry
    pub fn RegCreateKeyExW(
        hKey: isize,
        lpSubKey: *const u16,
        Reserved: u32,
        lpClass: *const u16,
        dwOptions: u32,
        samDesired: u32,
        lpSecurityAttributes: *mut c_void,
        phkResult: *mut isize,
        lpdwDisposition: *mut u32,
    ) -> i32;
    pub fn RegSetValueExW(
        hKey: isize,
        lpValueName: *const u16,
        Reserved: u32,
        dwType: u32,
        lpData: *const u8,
        cbData: u32,
    ) -> i32;
    pub fn RegCloseKey(hKey: isize) -> i32;
    pub fn RegDeleteKeyW(hKey: isize, lpSubKey: *const u16) -> i32;

    // GDI - Font
    pub fn CreateFontW(
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
    ) -> HGDIOBJ;

    // Window
    pub fn GetDesktopWindow() -> HWND;

    // Multi-monitor
    pub fn MonitorFromPoint(pt: POINT, dwFlags: u32) -> HMONITOR;
    pub fn GetMonitorInfoW(hMonitor: HMONITOR, lpmi: *mut MONITORINFO) -> i32;
    pub fn GetDpiForMonitor(
        hMonitor: HMONITOR,
        dpiType: u32,
        dpiX: *mut u32,
        dpiY: *mut u32,
    ) -> i32;

    // Tooltip / Common Controls
    pub fn InitCommonControlsEx(lpInitCtrls: *const INITCOMMONCONTROLSEX) -> i32;

    // Timer
    pub fn SetTimer(hWnd: HWND, nIDEvent: usize, uElapse: u32, lpTimerFunc: *mut c_void) -> usize;
    pub fn KillTimer(hWnd: HWND, uIDEvent: usize) -> i32;
}

// ===== Shell Context Menu COM Types =====

pub type HRESULT = i32;
pub const S_OK: HRESULT = 0;
#[allow(dead_code)]
pub const COINIT_APARTMENTTHREADED: u32 = 0x2;

// IID_IShellFolder = {000214E6-0000-0000-C000-000000000046}
pub const IID_ISHELLFOLDER: [u8; 16] = [
    0xE6, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
// IID_IContextMenu = {000214E4-0000-0000-C000-000000000046}
pub const IID_ICONTEXTMENU: [u8; 16] = [
    0xE4, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
// IID_IContextMenu2 = {000214F4-0000-0000-C000-000000000046}
pub const IID_ICONTEXTMENU2: [u8; 16] = [
    0xF4, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
// IID_IContextMenu3 = {BCFCE0A0-EC17-11D0-8D10-00A0C90F2719}
pub const IID_ICONTEXTMENU3: [u8; 16] = [
    0xA0, 0xE0, 0xCF, 0xBC, 0x17, 0xEC, 0xD0, 0x11, 0x8D, 0x10, 0x00, 0xA0, 0xC9, 0x0F, 0x27, 0x19,
];

#[repr(C)]
pub struct ITEMIDLIST {
    _opaque: u8,
}

/// IShellFolder vtable
#[repr(C)]
#[allow(non_snake_case)]
pub struct IShellFolderVtbl {
    pub QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub ParseDisplayName: unsafe extern "system" fn(
        *mut c_void,
        HWND,
        *mut c_void,
        *const u16,
        *mut u32,
        *mut *mut ITEMIDLIST,
        *mut u32,
    ) -> HRESULT,
    pub EnumObjects: unsafe extern "system" fn(*mut c_void, HWND, u32, *mut *mut c_void) -> HRESULT,
    pub BindToObject: unsafe extern "system" fn(
        *mut c_void,
        *const ITEMIDLIST,
        *mut c_void,
        *const u8,
        *mut *mut c_void,
    ) -> HRESULT,
    pub BindToStorage: unsafe extern "system" fn(
        *mut c_void,
        *const ITEMIDLIST,
        *mut c_void,
        *const u8,
        *mut *mut c_void,
    ) -> HRESULT,
    pub CompareIDs: unsafe extern "system" fn(
        *mut c_void,
        LPARAM,
        *const ITEMIDLIST,
        *const ITEMIDLIST,
    ) -> HRESULT,
    pub CreateViewObject:
        unsafe extern "system" fn(*mut c_void, HWND, *const u8, *mut *mut c_void) -> HRESULT,
    pub GetAttributesOf:
        unsafe extern "system" fn(*mut c_void, u32, *const *const ITEMIDLIST, *mut u32) -> HRESULT,
    pub GetUIObjectOf: unsafe extern "system" fn(
        *mut c_void,
        HWND,
        u32,
        *const *const ITEMIDLIST,
        *const u8,
        *mut u32,
        *mut *mut c_void,
    ) -> HRESULT,
    pub GetDisplayNameOf:
        unsafe extern "system" fn(*mut c_void, *const ITEMIDLIST, u32, *mut c_void) -> HRESULT,
    pub SetNameOf: unsafe extern "system" fn(
        *mut c_void,
        HWND,
        *const ITEMIDLIST,
        *const u16,
        u32,
        *mut *mut ITEMIDLIST,
    ) -> HRESULT,
}

#[repr(C)]
pub struct IShellFolder {
    pub lpVtbl: *const IShellFolderVtbl,
}

/// IContextMenu vtable
#[repr(C)]
#[allow(non_snake_case)]
pub struct IContextMenuVtbl {
    pub QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub QueryContextMenu: unsafe extern "system" fn(HMENU, u32, u32, u32, u32) -> HRESULT,
    pub InvokeCommand:
        unsafe extern "system" fn(*mut c_void, *const CMINVOKECOMMANDINFO) -> HRESULT,
    pub GetCommandString:
        unsafe extern "system" fn(*mut c_void, usize, u32, *mut u32, *mut u16, u32) -> HRESULT,
}

#[repr(C)]
pub struct IContextMenu {
    pub lpVtbl: *const IContextMenuVtbl,
}

/// IContextMenu2 vtable
#[repr(C)]
#[allow(non_snake_case)]
pub struct IContextMenu2Vtbl {
    pub QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub QueryContextMenu: unsafe extern "system" fn(HMENU, u32, u32, u32, u32) -> HRESULT,
    pub InvokeCommand:
        unsafe extern "system" fn(*mut c_void, *const CMINVOKECOMMANDINFO) -> HRESULT,
    pub GetCommandString:
        unsafe extern "system" fn(*mut c_void, usize, u32, *mut u32, *mut u16, u32) -> HRESULT,
    pub HandleMenuMsg: unsafe extern "system" fn(*mut c_void, u32, WPARAM, LPARAM) -> HRESULT,
}

#[repr(C)]
pub struct IContextMenu2 {
    pub lpVtbl: *const IContextMenu2Vtbl,
}

/// IContextMenu3 vtable
#[repr(C)]
#[allow(non_snake_case)]
pub struct IContextMenu3Vtbl {
    pub QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub QueryContextMenu: unsafe extern "system" fn(HMENU, u32, u32, u32, u32) -> HRESULT,
    pub InvokeCommand:
        unsafe extern "system" fn(*mut c_void, *const CMINVOKECOMMANDINFO) -> HRESULT,
    pub GetCommandString:
        unsafe extern "system" fn(*mut c_void, usize, u32, *mut u32, *mut u16, u32) -> HRESULT,
    pub HandleMenuMsg: unsafe extern "system" fn(*mut c_void, u32, WPARAM, LPARAM) -> HRESULT,
    pub HandleMenuMsg2:
        unsafe extern "system" fn(*mut c_void, u32, WPARAM, LPARAM, *mut LRESULT) -> HRESULT,
}

#[repr(C)]
pub struct IContextMenu3 {
    pub lpVtbl: *const IContextMenu3Vtbl,
}

/// CMINVOKECOMMANDINFO（匹配 Windows 64-bit C 布局，40 字节）
#[repr(C)]
pub struct CMINVOKECOMMANDINFO {
    pub cb_size: u32,
    pub f_mask: u32,
    pub hwnd: HWND,
    pub lp_verb: *const u8,
    pub lp_parameters: *const u8,
    pub lp_directory: *const u8,
    pub n_show: i32,
    pub dw_hot_key: u32,
    pub h_icon: HWND,
}

// Shell Context Menu FFI
extern "system" {
    pub fn SHGetDesktopFolder(ppshf: *mut *mut c_void) -> HRESULT;
    pub fn SHParseDisplayName(
        pszName: *const u16,
        pbc: *mut c_void,
        ppidl: *mut *mut ITEMIDLIST,
        sfgaoIn: u32,
        psfgaoOut: *mut u32,
    ) -> HRESULT;
    pub fn SHBindToParent(
        pidl: *const ITEMIDLIST,
        riid: *const u8,
        ppv: *mut *mut c_void,
        ppidlLast: *mut *const ITEMIDLIST,
    ) -> HRESULT;
    pub fn ILFree(pidl: *mut ITEMIDLIST);
    pub fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> HRESULT;
    pub fn CoUninitialize();
}

/// Shell 菜单命令 ID 起始值（预留 1–99 给自定义菜单项，0 为 TrackPopupMenu "无选择" 返回值）
pub const FCIDM_SHVIEWFIRST: u32 = 0x0064; // 100
pub const MENU_CMD_ID_LAST: u32 = FCIDM_SHVIEWFIRST + 0x7FFF;
pub const CMF_NORMAL: u32 = 0x00000000;
pub const CMF_EXPLORE: u32 = 0x00000004;
pub const CMF_CANRENAME: u32 = 0x00000010;
pub const CMF_EXTENDED_VERBS: u32 = 0x00000100;
pub const CMF_ITEMMENU: u32 = 0x00000080;
pub const CMF_NOVERBS: u32 = 0x00000008;

// BHID_SFUIObject - 通过 IShellItem::BindToHandler 获取 IContextMenu
// {b3a4b685-b6d5-4882-bcf6-f4107100a1e4}
pub const BHID_SFUIOBJECT: [u8; 16] = [
    0x85, 0xB6, 0xA4, 0xB3, 0xD5, 0xB6, 0x82, 0x48, 0xBC, 0xF6, 0xF4, 0x10, 0x71, 0x00, 0xA1, 0xE4,
];

// IID_IShellItem = {43826d1e-e718-42ee-bc55-a1e261c37bfe}
pub const IID_ISHELLITEM: [u8; 16] = [
    0x1E, 0x6D, 0x82, 0x43, 0x18, 0xE7, 0xEE, 0x42, 0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE,
];

/// IShellItem vtable（继承 IUnknown）
#[repr(C)]
#[allow(non_snake_case)]
pub struct IShellItemVtbl {
    pub QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub BindToHandler: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const u8,
        *const u8,
        *mut *mut c_void,
    ) -> HRESULT,
    pub GetParent: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub GetDisplayName: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> HRESULT,
    pub GetAttributes: unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> HRESULT,
    pub Compare: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut i32) -> HRESULT,
}

#[repr(C)]
pub struct IShellItem {
    pub lpVtbl: *const IShellItemVtbl,
}

extern "system" {
    pub fn SHCreateItemFromParsingName(
        pszPath: *const u16,
        pbc: *mut c_void,
        riid: *const u8,
        ppv: *mut *mut c_void,
    ) -> HRESULT;
}

// Tooltip 窗口创建（避免与主 CreateWindowExW 签名冲突）
#[link(name = "comctl32")]
extern "system" {
    #[link_name = "CreateWindowExW"]
    pub fn CreateWindowExW_tooltip(
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
