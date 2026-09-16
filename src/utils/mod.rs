//! 工具层
//!
//! - `logger` — 日志系统（自定义宏、性能日志）
//! - `platform` — 跨平台工具函数
//! - `alloc_probe` — Rust 堆占用探针（全局分配器计数）
//! - `dpi` — DPI 缩放工具
//! - `string_utils` — 字符串工具（UTF-16 转换）

pub mod alloc_probe;
pub mod dpi;
mod logger;
pub mod platform;
pub mod string_utils;

pub use logger::*;
