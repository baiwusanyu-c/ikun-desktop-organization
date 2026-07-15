//! 文件图标提取

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! 使用 SHGetFileInfoW / SHDefExtractIconW 获取文件的系统图标句柄 (HICON)
//! 支持解析 .lnk 快捷方式的目标路径以提取高清图标

use std::ffi::c_void;

type HICON = *mut c_void;
type HRESULT = i32;

const SHGFI_ICON: u32 = 0x000000100;
const SHGFI_DISPLAYNAME: u32 = 0x000000200;
const SHGFI_TYPENAME: u32 = 0x000000400;
const SHGFI_LARGEICON: u32 = 0x000000000;
const SHGFI_SMALLICON: u32 = 0x000000001;
const SHGFI_PIDL: u32 = 0x00000008;

// COM 常量
const S_OK: HRESULT = 0;
const COINIT_APARTMENTTHREADED: u32 = 0x2;

// CLSID_ShellLink = {00021401-0000-0000-C000-000000000046}
const CLSID_SHELLLINK: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

// IID_IShellLinkW = {000214F9-0000-0000-C000-000000000046}
const IID_ISHELLLINKW: [u8; 16] = [
    0xF9, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

// IID_IPersistFile = {0000010B-0000-0000-C000-000000000046}
const IID_IPERSISTFILE: [u8; 16] = [
    0x0B, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

/// IUnknown vtable 基础结构
#[repr(C)]
#[allow(dead_code)]
struct IUnknownVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// IPersistFile vtable（继承 IPersist → IUnknown）
#[repr(C)]
struct IPersistFileVtbl {
    // IUnknown
    query_interface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IPersist
    get_class_id: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    // IPersistFile
    is_dirty: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    load: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> HRESULT,
}

/// IShellLinkW vtable（继承 IUnknown）
#[repr(C)]
struct IShellLinkWVtbl {
    // IUnknown
    query_interface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IShellLinkW - GetPath 是第一个方法
    get_path: unsafe extern "system" fn(
        *mut c_void,
        *mut u16,
        i32,
        *mut c_void, // WIN32_FIND_DATAW*
        u32,
    ) -> HRESULT,
}

#[repr(C)]
#[allow(non_snake_case)]
struct IShellLinkW {
    lpVtbl: *const IShellLinkWVtbl,
}

#[repr(C)]
#[allow(non_snake_case)]
struct IPersistFile {
    lpVtbl: *const IPersistFileVtbl,
}

/// ITEMIDLIST 占位类型（仅用于指针传递，不需要解引用）
#[repr(C)]
struct ITEMIDLIST {
    _mkid: [u8; 0],
}

/// SHFILEINFOW 结构体
#[repr(C)]
pub struct SHFILEINFOW {
    pub h_icon: HICON,
    pub i_icon: i32,
    pub dw_attributes: u32,
    pub sz_display_name: [u16; 260],
    pub sz_type_name: [u16; 80],
}

impl Default for SHFILEINFOW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

extern "system" {
    fn SHGetFileInfoW(
        pszPath: *const u16,
        dwFileAttributes: u32,
        psfi: *mut SHFILEINFOW,
        cbFileInfo: u32,
        uFlags: u32,
    ) -> usize;
    fn DestroyIcon(hIcon: HICON) -> i32;
    fn SHDefExtractIconW(
        pszIconFile: *const u16,
        iIndex: i32,
        uFlags: u32,
        phiconLarge: *mut HICON,
        phiconSmall: *mut HICON,
        nIconSize: u32,
    ) -> i32;
    // COM
    fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> HRESULT;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const u8,
        pUnkOuter: *mut c_void,
        dwClsContext: u32,
        riid: *const u8,
        ppv: *mut *mut c_void,
    ) -> HRESULT;
    // Shell 名称解析与 PIDL 管理（用于虚拟 Shell 对象图标提取）
    fn SHParseDisplayName(
        pszName: *const u16,
        pbc: *mut c_void,
        ppidl: *mut *mut ITEMIDLIST,
        sfgaoIn: u32,
        psfgaoOut: *mut u32,
    ) -> HRESULT;
    fn ILFree(pidl: *mut ITEMIDLIST);
}

/// 文件图标信息
pub struct FileIconInfo {
    /// 系统图标句柄（调用者负责释放）
    pub hicon: HICON,
    /// 显示名称
    pub display_name: String,
    /// 类型名称
    pub type_name: String,
}

/// 解析 .lnk 快捷方式的目标路径
///
/// 使用 COM IShellLink + IPersistFile 接口获取快捷方式指向的实际文件路径。
/// SHDefExtractIconW 无法直接处理 .lnk 文件，需要先解析目标。
pub fn resolve_shortcut_target(lnk_path: &str) -> Option<String> {
    let lnk_wide: Vec<u16> = lnk_path.encode_utf16().chain(std::iter::once(0)).collect();

    // COM 可能已被 eframe 初始化，RPC_E_CHANGED_MODE 表示已有 COM 环境可用
    let hr = unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    let we_initialized_com = hr == S_OK;

    let result = (|| -> Option<String> {
        // 创建 ShellLink COM 对象
        let mut link_ptr: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                CLSID_SHELLLINK.as_ptr(),
                std::ptr::null_mut(),
                1, // CLSCTX_INPROC_SERVER
                IID_ISHELLLINKW.as_ptr(),
                &mut link_ptr,
            )
        };
        if hr != S_OK || link_ptr.is_null() {
            return None;
        }

        let link = link_ptr as *mut IShellLinkW;

        // 获取 IPersistFile 接口用于加载 .lnk 文件
        let mut persist_ptr: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            ((*(*link).lpVtbl).query_interface)(
                link_ptr,
                IID_IPERSISTFILE.as_ptr(),
                &mut persist_ptr,
            )
        };
        if hr != S_OK || persist_ptr.is_null() {
            unsafe { ((*(*link).lpVtbl).release)(link_ptr) };
            return None;
        }

        let persist = persist_ptr as *mut IPersistFile;

        // 加载 .lnk 文件
        let hr = unsafe { ((*(*persist).lpVtbl).load)(persist_ptr, lnk_wide.as_ptr(), 0) };
        if hr != S_OK {
            unsafe {
                ((*(*persist).lpVtbl).release)(persist_ptr);
                ((*(*link).lpVtbl).release)(link_ptr);
            }
            return None;
        }

        // 获取目标路径
        let mut path_buf = vec![0u16; 1024];
        let hr = unsafe {
            ((*(*link).lpVtbl).get_path)(
                link_ptr,
                path_buf.as_mut_ptr(),
                1024,
                std::ptr::null_mut(),
                0,
            )
        };

        unsafe {
            ((*(*persist).lpVtbl).release)(persist_ptr);
            ((*(*link).lpVtbl).release)(link_ptr);
        }

        if hr != S_OK {
            return None;
        }

        let len = path_buf.iter().position(|&c| c == 0).unwrap_or(0);
        if len == 0 {
            return None;
        }

        Some(String::from_utf16_lossy(&path_buf[..len]))
    })();

    if we_initialized_com {
        unsafe { CoUninitialize() };
    }

    result
}

fn is_clsid_path(path: &str) -> bool {
    path.starts_with("::{")
}

/// 从 CLSID 路径提取虚拟 Shell 对象的图标
///
/// 使用 `SHParseDisplayName` 将 CLSID 字符串解析为 PIDL，
/// 然后通过 `SHGetFileInfoW` + `SHGFI_PIDL` 提取系统图标。
/// 这样可以正确获取回收站、此电脑、网络等虚拟对象的图标。
fn extract_clsid_icon(clsid_path: &str) -> Option<FileIconInfo> {
    let path_wide: Vec<u16> = clsid_path.encode_utf16().chain(std::iter::once(0)).collect();

    // 解析 CLSID 字符串为 PIDL
    let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    let mut sfgao_out: u32 = 0;
    let hr = unsafe {
        SHParseDisplayName(
            path_wide.as_ptr(),
            std::ptr::null_mut(),
            &mut pidl,
            0,
            &mut sfgao_out,
        )
    };
    if hr != S_OK || pidl.is_null() {
        log::warn!("SHParseDisplayName 失败 (hr={:#X}): {}", hr as u32, clsid_path);
        return None;
    }

    // 提取显示名称和类型名称
    let mut name_info = SHFILEINFOW::default();
    unsafe {
        SHGetFileInfoW(
            pidl as *const u16,
            0,
            &mut name_info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_DISPLAYNAME | SHGFI_TYPENAME,
        );
    }
    let name_len = name_info.sz_display_name.iter().position(|&c| c == 0).unwrap_or(0);
    let display_name = String::from_utf16_lossy(&name_info.sz_display_name[..name_len]);
    let type_len = name_info.sz_type_name.iter().position(|&c| c == 0).unwrap_or(0);
    let type_name = String::from_utf16_lossy(&name_info.sz_type_name[..type_len]);

    // 提取大图标 (32x32)
    let mut icon_info = SHFILEINFOW::default();
    let result = unsafe {
        SHGetFileInfoW(
            pidl as *const u16,
            0,
            &mut icon_info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_ICON | SHGFI_LARGEICON,
        )
    };

    let hicon = if result != 0 && !icon_info.h_icon.is_null() {
        icon_info.h_icon
    } else {
        // 回退：小图标 (16x16)
        let result2 = unsafe {
            SHGetFileInfoW(
                pidl as *const u16,
                0,
                &mut icon_info,
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_PIDL | SHGFI_ICON | SHGFI_SMALLICON,
            )
        };
        if result2 != 0 && !icon_info.h_icon.is_null() {
            icon_info.h_icon
        } else {
            unsafe { ILFree(pidl) };
            return None;
        }
    };

    unsafe { ILFree(pidl) };

    log::debug!("虚拟对象图标提取成功: {} -> {}", clsid_path, display_name);
    Some(FileIconInfo {
        hicon,
        display_name,
        type_name,
    })
}


/// 通过显示名称从 SysListView32 提取虚拟对象图标
///
/// 用于处理 shell::virtual:: 前缀的路径，这些对象不存在于文件系统中，
/// 只能通过 SysListView32 的图像列表提取图标。
fn extract_virtual_icon_by_name(display_name: &str) -> Option<FileIconInfo> {
    use crate::platform::windows::icon_manager::DesktopIconManager;

    let mut manager = DesktopIconManager::new().ok()?;
    let entries = manager.enumerate_names_with_images();

    // 查找匹配的显示名称，使用 image_index（而非 item_index）提取图标
    for (name, _item_idx, image_idx) in entries {
        if name.eq_ignore_ascii_case(display_name) {
            log::debug!(
                "Found virtual icon '{}' item={} image={}",
                display_name, _item_idx, image_idx
            );
            return crate::platform::windows::icon_manager::extract_icon_from_listview(image_idx);
        }
    }

    log::warn!("Virtual icon '{}' not found in SysListView32", display_name);
    None
}

/// 提取文件的系统图标（优先高质量大图标，支持高 DPI 缩放）
///
/// 对 .lnk 快捷方式，先解析目标路径再从实际 exe 提取图标。
/// 提取顺序：
/// 1. SHDefExtractIconW 提取 128×128 高清图标
/// 2. SHGetFileInfoW 大图标（32×32）
/// 3. SHGetFileInfoW 小图标（16×16）
pub fn extract_file_icon(file_path: &str) -> Option<FileIconInfo> {
    // 虚拟 Shell 对象（回收站、此电脑等）使用 PIDL 提取图标
    if is_clsid_path(file_path) {
        return extract_clsid_icon(file_path);
    }

    // Shell 命名空间扩展对象（从 SysListView32 发现的虚拟对象）
    // 通过显示名称在 SysListView32 中查找并提取图标
    if let Some(display_name) = file_path.strip_prefix("shell::virtual::") {
        return extract_virtual_icon_by_name(display_name);
    }

    // 获取显示名称和类型名称（始终使用原始路径）
    let display_path_wide: Vec<u16> = file_path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();
    unsafe {
        SHGetFileInfoW(
            display_path_wide.as_ptr(),
            0,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME | SHGFI_TYPENAME,
        );
    }
    let name_len = info
        .sz_display_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(0);
    let display_name = String::from_utf16_lossy(&info.sz_display_name[..name_len]);
    let type_len = info.sz_type_name.iter().position(|&c| c == 0).unwrap_or(0);
    let type_name = String::from_utf16_lossy(&info.sz_type_name[..type_len]);

    // 对 .lnk 快捷方式，始终解析目标路径并提取目标图标，避免 Shell 缓存图标
    // 自带的"快捷方式"小箭头覆层。
    let resolved_path = if file_path.to_lowercase().ends_with(".lnk") {
        let target = resolve_shortcut_target(file_path);
        if let Some(ref t) = target {
            log::debug!("快捷方式解析成功: {} → {}", file_path, t);
        } else {
            log::debug!("快捷方式解析失败: {}", file_path);
        }
        target
    } else {
        None
    };

    // 高清图标提取路径：优先用解析后的目标，否则用原始路径
    let icon_extract_path = resolved_path.as_deref().unwrap_or(file_path);
    let path_wide: Vec<u16> = icon_extract_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // 1. 尝试高质量图标提取（128×128，缩放后不失真）
    let mut hq_icon: HICON = std::ptr::null_mut();
    let icon_size: u32 = 128;
    let hq_result = unsafe {
        SHDefExtractIconW(
            path_wide.as_ptr(),
            0,
            0,
            &mut hq_icon,
            std::ptr::null_mut(),
            icon_size,
        )
    };
    if hq_result == 0 && !hq_icon.is_null() {
        log::debug!("高清图标提取成功: {} (128x128)", file_path);
        return Some(FileIconInfo {
            hicon: hq_icon,
            display_name,
            type_name,
        });
    }
    log::debug!(
        "高清图标提取失败 (hq_result={}), 回退到 32x32: {}",
        hq_result,
        file_path
    );

    // 2. 回退：SHGetFileInfoW 大图标（32×32）
    let mut fallback_info = SHFILEINFOW::default();
    let result = unsafe {
        SHGetFileInfoW(
            path_wide.as_ptr(),
            0,
            &mut fallback_info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if result != 0 && !fallback_info.h_icon.is_null() {
        return Some(FileIconInfo {
            hicon: fallback_info.h_icon,
            display_name,
            type_name,
        });
    }

    // 3. 回退：小图标（16×16）
    let result2 = unsafe {
        SHGetFileInfoW(
            path_wide.as_ptr(),
            0,
            &mut fallback_info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON,
        )
    };
    if result2 != 0 && !fallback_info.h_icon.is_null() {
        return Some(FileIconInfo {
            hicon: fallback_info.h_icon,
            display_name,
            type_name,
        });
    }

    None
}

/// 释放图标句柄
pub fn destroy_icon(hicon: HICON) {
    if !hicon.is_null() {
        unsafe {
            DestroyIcon(hicon);
        }
    }
}
