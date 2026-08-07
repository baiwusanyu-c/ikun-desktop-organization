//! Cross-platform data types
//!
//! Desktop icons, screen info, icon data etc shared across platforms.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Desktop icon info (cross-platform)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopIcon {
    pub id: uuid::Uuid,
    pub display_name: String,
    pub path: PathBuf,
    pub target_path: Option<PathBuf>,
    pub icon_path: Option<PathBuf>,
    pub position: (i32, i32),
    pub is_shortcut: bool,
}

/// Extra icon info
#[derive(Debug, Clone)]
pub struct IconInfo {
    pub exe_name: Option<String>,
    pub description: Option<String>,
    pub size: Option<u64>,
}

/// Screen info
#[derive(Debug, Clone)]
pub struct ScreenInfo {
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
}

/// Extracted icon pixel data (cross-platform)
#[derive(Debug, Clone)]
pub struct IconData {
    /// RGBA pixel data (row-major, top-to-bottom)
    pub pixels: Vec<u8>,
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
}

/// Raw icon handle (platform opaque)
#[cfg(windows)]
pub type RawIconHandle = *mut std::ffi::c_void;
#[cfg(not(windows))]
pub type RawIconHandle = *mut std::ffi::c_void; // placeholder for macOS
