//! 隐藏/恢复原生桌面图标
//!
//! 将 SysListView32 中的所有图标移到屏幕外（X=-3000），
//! 使原生图标不可见。退出时恢复到原始位置。

use anyhow::Result;
use std::ffi::c_void;
use std::sync::Mutex;

// ===== Raw FFI =====
type HWND = *mut c_void;
type HANDLE = *mut c_void;
type LPARAM = isize;
type WPARAM = usize;
type LRESULT = isize;

const LVM_FIRST: u32 = 0x1000;
const LVM_GETITEMCOUNT: u32 = LVM_FIRST + 4;
const LVM_GETITEMPOSITION: u32 = LVM_FIRST + 16;
const LVM_SETITEMPOSITION: u32 = LVM_FIRST + 15;

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;
const PROCESS_VM_OPERATION: u32 = 0x0008;
const PROCESS_VM_READ: u32 = 0x0010;
const PROCESS_VM_WRITE: u32 = 0x0020;

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

extern "system" {
    fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> HWND;
    fn FindWindowExW(
        hWndParent: HWND,
        hWndChildAfter: HWND,
        lpClassName: *const u16,
        lpWindowName: *const u16,
    ) -> HWND;
    fn SendMessageW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut u32) -> u32;
    fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> HANDLE;
    fn CloseHandle(hObject: HANDLE) -> i32;
    fn VirtualAllocEx(
        hProcess: HANDLE,
        lpAddress: *mut c_void,
        dwSize: usize,
        flAllocType: u32,
        flProtect: u32,
    ) -> *mut c_void;
    fn VirtualFreeEx(
        hProcess: HANDLE,
        lpAddress: *mut c_void,
        dwSize: usize,
        dwFreeType: u32,
    ) -> i32;
    fn ReadProcessMemory(
        hProcess: HANDLE,
        lpBaseAddress: *const c_void,
        lpBuffer: *mut c_void,
        nSize: usize,
        lpNumberOfBytesRead: *mut usize,
    ) -> i32;
}

fn makelparam(lo: i32, hi: i32) -> LPARAM {
    ((hi as u32 & 0xFFFF) as isize) << 16 | (lo as u32 & 0xFFFF) as isize
}

static PROG_CLASS: &[u16] = &[80, 114, 111, 103, 109, 97, 110, 0]; // "Progman"
static WORKER_CLASS: &[u16] = &[87, 111, 114, 107, 101, 114, 87, 0]; // "WorkerW"
static DEFVIEW_CLASS: &[u16] = &[
    83, 72, 69, 76, 76, 68, 76, 76, 95, 68, 101, 102, 86, 105, 101, 119, 0,
]; // "SHELLDLL_DefView"
static LISTVIEW_CLASS: &[u16] = &[
    83, 121, 115, 76, 105, 115, 116, 86, 105, 101, 119, 51, 50, 0,
]; // "SysListView32"

/// 查找桌面 SysListView32 句柄
fn find_desktop_listview() -> Result<HWND> {
    unsafe {
        // 尝试 Progman
        let progman = FindWindowW(PROG_CLASS.as_ptr(), std::ptr::null());
        if !progman.is_null() {
            let defview = FindWindowExW(
                progman,
                std::ptr::null_mut(),
                DEFVIEW_CLASS.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    LISTVIEW_CLASS.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Ok(lv);
                }
            }
        }

        // 尝试 WorkerW
        let mut worker = FindWindowW(WORKER_CLASS.as_ptr(), std::ptr::null());
        while !worker.is_null() {
            let defview = FindWindowExW(
                worker,
                std::ptr::null_mut(),
                DEFVIEW_CLASS.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    LISTVIEW_CLASS.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Ok(lv);
                }
            }
            worker = FindWindowExW(
                std::ptr::null_mut(),
                worker,
                WORKER_CLASS.as_ptr(),
                std::ptr::null(),
            );
        }

        anyhow::bail!("无法找到 SysListView32")
    }
}

/// 打开 explorer.exe 进程句柄
fn open_explorer_process(listview_hwnd: HWND) -> Result<HANDLE> {
    unsafe {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(listview_hwnd, &mut pid);
        let handle = OpenProcess(
            PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE,
            0,
            pid,
        );
        if handle.is_null() {
            anyhow::bail!("无法打开 explorer 进程 (PID={})", pid);
        }
        Ok(handle)
    }
}

/// 读取图标位置
fn get_icon_position(process: HANDLE, lv_hwnd: HWND, index: i32) -> Result<(i32, i32)> {
    unsafe {
        let remote = VirtualAllocEx(
            process,
            std::ptr::null_mut(),
            std::mem::size_of::<POINT>(),
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );
        if remote.is_null() {
            anyhow::bail!("VirtualAllocEx 失败");
        }

        SendMessageW(
            lv_hwnd,
            LVM_GETITEMPOSITION,
            index as usize,
            remote as LPARAM,
        );

        let mut pt = POINT { x: 0, y: 0 };
        ReadProcessMemory(
            process,
            remote,
            &mut pt as *mut _ as *mut _,
            std::mem::size_of::<POINT>(),
            std::ptr::null_mut(),
        );
        VirtualFreeEx(process, remote, 0, MEM_RELEASE);

        Ok((pt.x, pt.y))
    }
}

/// 保存的图标位置（用于恢复）
struct SavedIconPosition {
    index: i32,
    x: i32,
    y: i32,
}

/// 图标隐藏器全局状态
pub struct IconHider {
    listview_hwnd: HWND,
    process_handle: HANDLE,
    saved_positions: Vec<SavedIconPosition>,
    hidden: bool,
}

// Safety: HWND 和 HANDLE 是 Send 的（只在同一线程使用）
unsafe impl Send for IconHider {}

static ICON_HIDER: Mutex<Option<IconHider>> = Mutex::new(None);

/// 隐藏所有原生桌面图标（移到屏幕外）
pub fn hide_native_icons() -> Result<()> {
    let lv_hwnd = find_desktop_listview()?;
    let process = open_explorer_process(lv_hwnd)?;

    let count = unsafe { SendMessageW(lv_hwnd, LVM_GETITEMCOUNT, 0, 0) } as i32;
    log::info!("SysListView32 中有 {} 个图标", count);

    // 保存当前位置
    let mut saved = Vec::with_capacity(count as usize);
    for i in 0..count {
        if let Ok((x, y)) = get_icon_position(process, lv_hwnd, i) {
            saved.push(SavedIconPosition { index: i, x, y });
        }
    }

    // 移到屏幕外
    for i in 0..count {
        unsafe {
            SendMessageW(
                lv_hwnd,
                LVM_SETITEMPOSITION,
                i as usize,
                makelparam(-3000, -3000),
            );
        }
    }

    log::info!("已隐藏 {} 个原生桌面图标", count);

    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;
    *guard = Some(IconHider {
        listview_hwnd: lv_hwnd,
        process_handle: process,
        saved_positions: saved,
        hidden: true,
    });

    Ok(())
}

/// 恢复原生桌面图标到原始位置
pub fn restore_native_icons() -> Result<()> {
    let mut guard = ICON_HIDER
        .lock()
        .map_err(|e| anyhow::anyhow!("锁获取失败: {}", e))?;

    if let Some(hider) = guard.take() {
        if !hider.hidden {
            return Ok(());
        }

        for pos in &hider.saved_positions {
            unsafe {
                SendMessageW(
                    hider.listview_hwnd,
                    LVM_SETITEMPOSITION,
                    pos.index as usize,
                    makelparam(pos.x, pos.y),
                );
            }
        }

        unsafe {
            CloseHandle(hider.process_handle);
        }

        log::info!("已恢复 {} 个原生桌面图标", hider.saved_positions.len());
    }

    Ok(())
}
