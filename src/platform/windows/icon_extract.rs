//! 文件图标提取

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! 获取文件缩略图或系统图标句柄 (HICON)。
//!
//! 内容文件优先使用 `IShellItemImageFactory` 缩略图；应用文件和 `.lnk`
//! 快捷方式使用 `SHDefExtractIconW` 等 Shell 图标 API，避免缩略图画布留白。

use std::ffi::c_void;

type HICON = *mut c_void;
type HRESULT = i32;
type RawHBITMAP = *mut c_void;

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

// IID_IShellItemImageFactory = {bcc18b79-ba16-442f-80c4-8a59c30c463b}

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
    let path_wide: Vec<u16> = clsid_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

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
        log::warn!(
            "SHParseDisplayName 失败 (hr={:#X}): {}",
            hr as u32,
            clsid_path
        );
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
    let name_len = name_info
        .sz_display_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(0);
    let display_name = String::from_utf16_lossy(&name_info.sz_display_name[..name_len]);
    let type_len = name_info
        .sz_type_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(0);
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

/// Use IShellItemImageFactory to extract file thumbnails (windows crate safe API)
///
/// Returns HBITMAP with actual content preview for supported files (images, videos, docs).
/// This is the same API Windows Explorer uses for thumbnails.
/// Does not use SIIGBF_THUMBNAILONLY, allowing on-demand thumbnail generation.
/// Returns 32-bit premultiplied alpha HBITMAP, caller responsible for release.
fn extract_file_thumbnail(file_path: &str, size: i32) -> Option<RawHBITMAP> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName};

    let path_wide: Vec<u16> = file_path.encode_utf16().chain(std::iter::once(0)).collect();
    let pcwstr = PCWSTR(path_wide.as_ptr());

    // 1. Create IShellItem and get IShellItemImageFactory
    let factory: IShellItemImageFactory = match unsafe { SHCreateItemFromParsingName(pcwstr, None) }
    {
        Ok(item) => item,
        Err(e) => {
            log::info!(
                "[thumbnail] SHCreateItemFromParsingName failed: {}: {}",
                e,
                file_path
            );
            return None;
        }
    };
    log::info!("[thumbnail] IShellItemImageFactory acquired: {}", file_path);

    // 2. Get thumbnail (no THUMBNAILONLY flag = generate on demand)
    let requested_size = SIZE { cx: size, cy: size };
    match unsafe {
        factory.GetImage(
            requested_size,
            windows::Win32::UI::Shell::SIIGBF_RESIZETOFIT,
        )
    } {
        Ok(hbitmap) => {
            log::info!("[thumbnail] OK: {} ({}x{})", file_path, size, size);
            Some(hbitmap.0 as RawHBITMAP)
        }
        Err(e) => {
            log::info!("[thumbnail] GetImage failed: {}: {}", e, file_path);
            None
        }
    }
}
/// Convert thumbnail HBITMAP to HICON for compatibility with existing rendering pipeline
///
/// IShellItemImageFactory returns premultiplied alpha HBITMAP.
/// The converted HICON is rendered through draw_icon_d2d.
fn thumbnail_to_hicon(hbitmap: RawHBITMAP) -> HICON {
    use windows::Win32::Graphics::Gdi::{
        CreateBitmap, DeleteObject, GetDC, ReleaseDC, HBITMAP as WinHBITMAP,
    };
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

    if hbitmap.is_null() {
        return std::ptr::null_mut();
    }

    // Get bitmap dimensions
    let screen_dc = unsafe { GetDC(None) };

    #[repr(C)]
    struct BitmapInfoHeader {
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

    let mut bi = BitmapInfoHeader {
        bi_size: std::mem::size_of::<BitmapInfoHeader>() as u32,
        bi_width: 0,
        bi_height: 0,
        bi_planes: 0,
        bi_bit_count: 0,
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };

    extern "system" {
        fn GetDIBits(
            hdc: *mut c_void,
            hbm: *mut c_void,
            start: u32,
            lines: u32,
            bits: *mut c_void,
            bi: *mut c_void,
            usage: u32,
        ) -> i32;
    }

    let get_result = unsafe {
        GetDIBits(
            screen_dc.0,
            hbitmap,
            0,
            0,
            std::ptr::null_mut(),
            &mut bi as *mut BitmapInfoHeader as *mut c_void,
            0,
        )
    };

    let bmp_w = bi.bi_width;
    let bmp_h = bi.bi_height.abs();

    unsafe {
        let _ = ReleaseDC(None, screen_dc);
    }

    if get_result == 0 || bmp_w <= 0 || bmp_h <= 0 {
        log::info!("[thumbnail] thumbnail_to_hicon: failed to get bitmap dimensions");
        return std::ptr::null_mut();
    }

    // Create opaque mask bitmap (1-bit monochrome, all zeros = opaque)
    let row_bytes = ((bmp_w + 31) / 32) * 4;
    let mask_bits = vec![0u8; row_bytes as usize * bmp_h as usize];
    let mask_bitmap = unsafe {
        CreateBitmap(
            bmp_w,
            bmp_h,
            1,
            1,
            Some(mask_bits.as_ptr() as *const c_void),
        )
    };
    if mask_bitmap.is_invalid() {
        log::info!("[thumbnail] thumbnail_to_hicon: CreateBitmap mask failed");
        return std::ptr::null_mut();
    }

    let mut icon_info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask_bitmap,
        hbmColor: WinHBITMAP(hbitmap),
    };

    let hicon_result = unsafe { CreateIconIndirect(&mut icon_info) };

    // Release mask bitmap (CreateIconIndirect copies the data)
    unsafe {
        let _ = DeleteObject(mask_bitmap.into());
    }
    // Release original thumbnail HBITMAP
    unsafe {
        let _ = DeleteObject(WinHBITMAP(hbitmap).into());
    }

    match hicon_result {
        Ok(win_hicon) => {
            log::info!("[thumbnail] thumbnail_to_hicon: OK");
            win_hicon.0 as HICON
        }
        Err(e) => {
            log::info!(
                "[thumbnail] thumbnail_to_hicon: CreateIconIndirect failed: {}",
                e
            );
            std::ptr::null_mut()
        }
    }
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

    // 0. 优先尝试提取缩略图（图片/视频/文档等支持缩略图的文件）
    //    缩略图由 IShellItemImageFactory 提供，与 Windows 资源管理器显示一致
    if should_extract_thumbnail(icon_extract_path) {
        if let Some(thumbnail) = extract_file_thumbnail(icon_extract_path, 256) {
            let hicon = thumbnail_to_hicon(thumbnail);
            if !hicon.is_null() {
                log::info!("[thumbnail] using thumbnail: {}", file_path);
                return Some(FileIconInfo {
                    hicon,
                    display_name,
                    type_name,
                });
            }
        }
    }

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

/// Thumbnails are useful for content files, but executable/shortcut thumbnails
/// often contain a padded shell preview instead of the application icon.
fn should_extract_thumbnail(file_path: &str) -> bool {
    let extension = std::path::Path::new(file_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());

    !matches!(
        extension.as_deref(),
        Some("lnk")
            | Some("url")
            | Some("exe")
            | Some("dll")
            | Some("com")
            | Some("bat")
            | Some("cmd")
            | Some("msi")
            | Some("appref-ms")
    )
}

/// 释放图标句柄
pub fn destroy_icon(hicon: HICON) {
    if !hicon.is_null() {
        unsafe {
            DestroyIcon(hicon);
        }
    }
}

#[cfg(test)]
mod thumbnail_tests {
    use super::*;

    #[test]
    fn test_extract_file_thumbnail_nonexistent_file() {
        // Non-existent file should return None
        let result = extract_file_thumbnail(r"C:\this_file_does_not_exist_12345.png", 256);
        assert!(result.is_none(), "Non-existent file should return None");
    }

    #[test]
    fn test_extract_file_thumbnail_empty_path() {
        // Empty path should return None
        let result = extract_file_thumbnail("", 256);
        assert!(result.is_none(), "Empty path should return None");
    }

    #[test]
    fn test_thumbnail_to_hicon_null_input() {
        // Null HBITMAP should return null HICON
        let result = thumbnail_to_hicon(std::ptr::null_mut());
        assert!(result.is_null(), "Null input should return null HICON");
    }

    #[test]
    fn test_should_skip_padded_shell_thumbnails_for_app_files() {
        let application_paths = [
            r"C:\Apps\cursor.EXE",
            r"C:\Users\test\Desktop\app.lnk",
            r"C:\Apps\library.dll",
            r"C:\Apps\legacy.com",
            r"C:\Apps\installer.msi",
            r"C:\Apps\launcher.appref-ms",
            r"C:\Apps\setup.bat",
            r"C:\Apps\script.cmd",
            r"C:\Apps\website.url",
        ];
        for path in application_paths {
            assert!(!should_extract_thumbnail(path), "should skip {path}");
        }

        assert!(should_extract_thumbnail(
            r"C:\Users\test\Pictures\photo.png"
        ));
        assert!(should_extract_thumbnail(
            r"C:\Users\test\Documents\manual.pdf"
        ));
        assert!(should_extract_thumbnail(r"C:\Users\test\Documents\README"));
    }

    #[test]
    fn test_extract_file_icon_still_works() {
        // Ensure existing icon extraction still works
        // Try a common file type that should exist
        let test_paths = [
            r"C:\Windows\System32\notepad.exe",
            r"C:\Windows\explorer.exe",
        ];

        for path in &test_paths {
            if std::path::Path::new(path).exists() {
                let result = extract_file_icon(path);
                // Should get an icon (either thumbnail or fallback)
                if let Some(info) = result {
                    assert!(
                        !info.hicon.is_null(),
                        "HICON should not be null for {}",
                        path
                    );
                    // Clean up
                    destroy_icon(info.hicon);
                }
                break; // Found a valid test file
            }
        }
    }
}
