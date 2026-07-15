//! DPI 缩放工具

#![allow(dead_code)] // DPI 工具函数，部分为架构预留
//!
//! 从 grid_window 提取的 DPI 相关辅助函数。

/// 根据 DPI 缩放像素值（96 DPI 为基准）
pub fn scale_for_dpi(px: i32, dpi: u32) -> i32 {
    (px * dpi as i32) / 96
}

/// 从 LPARAM 提取 X 坐标
pub fn get_x_lparam(lp: isize) -> i32 {
    (lp & 0xFFFF) as i16 as i32
}

/// 从 LPARAM 提取 Y 坐标
pub fn get_y_lparam(lp: isize) -> i32 {
    ((lp >> 16) & 0xFFFF) as i16 as i32
}
