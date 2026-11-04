//! File icon extraction.
//!
//! Content files prefer `IShellItemImageFactory` thumbnails. Applications and
//! shortcuts use the typed Shell icon APIs from the `windows` crate.

#![allow(dead_code)]

use std::ffi::c_void;

use windows::core::{Interface, GUID, PCWSTR};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ILFree, IShellLinkW, SHDefExtractIconW, SHGetFileInfoW, SHParseDisplayName, SHFILEINFOW,
    SHGFI_DISPLAYNAME, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_PIDL, SHGFI_SMALLICON, SHGFI_TYPENAME,
};
use windows::Win32::UI::WindowsAndMessaging::HICON as WinHICON;

type HICON = *mut c_void;
type RawHBITMAP = *mut c_void;

// CLSID_ShellLink = {00021401-0000-0000-C000-000000000046}
const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_C000_000000000046);

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

    // COM may already be initialized on this thread. A successful call, including
    // S_FALSE, must be paired with CoUninitialize.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let we_initialized_com = hr.is_ok();

    let result = (|| -> Option<String> {
        let link: IShellLinkW =
            unsafe { CoCreateInstance(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER) }.ok()?;
        let persist: IPersistFile = link.cast().ok()?;

        unsafe { persist.Load(PCWSTR(lnk_wide.as_ptr()), STGM_READ) }.ok()?;

        let mut path_buf = [0u16; 1024];
        unsafe { link.GetPath(&mut path_buf, std::ptr::null_mut(), 0) }.ok()?;

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

    let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    let mut sfgao_out = 0u32;
    if let Err(e) = unsafe {
        SHParseDisplayName(
            PCWSTR(path_wide.as_ptr()),
            None,
            &mut pidl,
            0,
            Some(&mut sfgao_out),
        )
    } {
        log::warn!("SHParseDisplayName failed: {}: {}", e, clsid_path);
        return None;
    }
    if pidl.is_null() {
        log::warn!("SHParseDisplayName returned a null PIDL: {}", clsid_path);
        return None;
    }

    let mut name_info = SHFILEINFOW::default();
    unsafe {
        SHGetFileInfoW(
            PCWSTR(pidl as *const u16),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut name_info as *mut SHFILEINFOW),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_DISPLAYNAME | SHGFI_TYPENAME,
        );
    }
    let name_len = name_info
        .szDisplayName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(0);
    let display_name = String::from_utf16_lossy(&name_info.szDisplayName[..name_len]);
    let type_len = name_info
        .szTypeName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(0);
    let type_name = String::from_utf16_lossy(&name_info.szTypeName[..type_len]);

    let mut icon_info = SHFILEINFOW::default();
    let result = unsafe {
        SHGetFileInfoW(
            PCWSTR(pidl as *const u16),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut icon_info as *mut SHFILEINFOW),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_ICON | SHGFI_LARGEICON,
        )
    };

    let hicon = if result != 0 && !icon_info.hIcon.0.is_null() {
        icon_info.hIcon.0 as HICON
    } else {
        let result2 = unsafe {
            SHGetFileInfoW(
                PCWSTR(pidl as *const u16),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut icon_info as *mut SHFILEINFOW),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_PIDL | SHGFI_ICON | SHGFI_SMALLICON,
            )
        };
        if result2 != 0 && !icon_info.hIcon.0.is_null() {
            icon_info.hIcon.0 as HICON
        } else {
            unsafe { ILFree(Some(pidl)) };
            return None;
        }
    };

    unsafe { ILFree(Some(pidl)) };

    log::debug!(
        "virtual shell icon extracted: {} -> {}",
        clsid_path,
        display_name
    );
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
        CreateBitmap, DeleteObject, GetDC, GetDIBits, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
        DIB_RGB_COLORS, HBITMAP as WinHBITMAP,
    };
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

    if hbitmap.is_null() {
        return std::ptr::null_mut();
    }

    // Get bitmap dimensions
    let screen_dc = unsafe { GetDC(None) };

    let mut bi = BITMAPINFO::default();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;

    let get_result = unsafe {
        GetDIBits(
            screen_dc,
            WinHBITMAP(hbitmap),
            0,
            0,
            None,
            &mut bi,
            DIB_RGB_COLORS,
        )
    };

    let bmp_w = bi.bmiHeader.biWidth;
    let bmp_h = bi.bmiHeader.biHeight.abs();

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

    let icon_info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask_bitmap,
        hbmColor: WinHBITMAP(hbitmap),
    };

    let hicon_result = unsafe { CreateIconIndirect(&icon_info) };

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
            PCWSTR(display_path_wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info as *mut SHFILEINFOW),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME | SHGFI_TYPENAME,
        );
    }
    let name_len = info.szDisplayName.iter().position(|&c| c == 0).unwrap_or(0);
    let display_name = String::from_utf16_lossy(&info.szDisplayName[..name_len]);
    let type_len = info.szTypeName.iter().position(|&c| c == 0).unwrap_or(0);
    let type_name = String::from_utf16_lossy(&info.szTypeName[..type_len]);

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
    let mut hq_icon = WinHICON::default();
    let icon_size: u32 = 128;
    let hq_result = unsafe {
        SHDefExtractIconW(
            PCWSTR(path_wide.as_ptr()),
            0,
            0,
            Some(&mut hq_icon as *mut WinHICON),
            None,
            icon_size,
        )
    };
    if hq_result.is_ok() && !hq_icon.0.is_null() {
        log::debug!("高清图标提取成功: {} (128x128)", file_path);
        return Some(FileIconInfo {
            hicon: hq_icon.0 as HICON,
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
            PCWSTR(path_wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut fallback_info as *mut SHFILEINFOW),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if result != 0 && !fallback_info.hIcon.0.is_null() {
        return Some(FileIconInfo {
            hicon: fallback_info.hIcon.0 as HICON,
            display_name,
            type_name,
        });
    }

    // 3. 回退：小图标（16×16）
    let result2 = unsafe {
        SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut fallback_info as *mut SHFILEINFOW),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON,
        )
    };
    if result2 != 0 && !fallback_info.hIcon.0.is_null() {
        return Some(FileIconInfo {
            hicon: fallback_info.hIcon.0 as HICON,
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
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(WinHICON(hicon));
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
