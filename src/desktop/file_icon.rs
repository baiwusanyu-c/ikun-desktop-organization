//! 文件图标提取
//!
//! 使用 SHGetFileInfoW 获取文件的系统图标句柄 (HICON)

use std::ffi::c_void;

type HICON = *mut c_void;

const SHGFI_ICON: u32 = 0x000000100;
const SHGFI_DISPLAYNAME: u32 = 0x000000200;
const SHGFI_LARGEICON: u32 = 0x000000000;
const SHGFI_SMALLICON: u32 = 0x000000001;

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

/// 提取文件的系统图标（优先大图标，确保 alpha 通道完整）
pub fn extract_file_icon(file_path: &str) -> Option<FileIconInfo> {
    let path_wide: Vec<u16> = file_path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut info = SHFILEINFOW::default();
    // 优先尝试大图标（32x32），alpha 通道更完整
    let result = unsafe {
        SHGetFileInfoW(
            path_wide.as_ptr(),
            0,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_DISPLAYNAME | SHGFI_LARGEICON,
        )
    };

    if result == 0 || info.h_icon.is_null() {
        // 回退到小图标
        let result2 = unsafe {
            SHGetFileInfoW(
                path_wide.as_ptr(),
                0,
                &mut info,
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_DISPLAYNAME | SHGFI_SMALLICON,
            )
        };
        if result2 == 0 || info.h_icon.is_null() {
            return None;
        }
    }

    let name_len = info
        .sz_display_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(0);
    let display_name = String::from_utf16_lossy(&info.sz_display_name[..name_len]);

    let type_len = info.sz_type_name.iter().position(|&c| c == 0).unwrap_or(0);
    let type_name = String::from_utf16_lossy(&info.sz_type_name[..type_len]);

    Some(FileIconInfo {
        hicon: info.h_icon,
        display_name,
        type_name,
    })
}

/// 释放图标句柄
pub fn destroy_icon(hicon: HICON) {
    if !hicon.is_null() {
        unsafe {
            DestroyIcon(hicon);
        }
    }
}
