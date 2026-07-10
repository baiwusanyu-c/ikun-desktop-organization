//! 字符串工具

#![allow(dead_code)] // 字符串工具函数
//!
//! UTF-16 编码转换等跨平台字符串辅助函数。

/// 将 Rust 字符串编码为 UTF-16 并追加 null 终止符
pub fn to_wide_string(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 从 UTF-16 缓冲区读取字符串（遇到 null 终止）
pub fn from_wide_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}
