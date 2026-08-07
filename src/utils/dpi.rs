//! DPI scaling utility
//!
//! Cross-platform DPI scaling helper.
//! Note: Windows-specific scale_for_dpi is in platform/windows/ffi_types.rs.
//! This module provides the same function for non-Windows platforms.

#![allow(dead_code)]

/// Scale pixel value by DPI (96 DPI baseline)
pub fn scale_for_dpi(px: i32, dpi: u32) -> i32 {
    (px * dpi as i32) / 96
}
