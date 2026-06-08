//! 日志工具函数
//!
//! 提供统一的日志输出格式和便捷函数

use log::{debug, error, info, warn};

/// 初始化日志系统
///
/// 日志级别通过环境变量 RUST_LOG 控制：
/// - `RUST_LOG=debug` - 显示所有日志
/// - `RUST_LOG=info` - 只显示 info/warn/error
/// - `RUST_LOG=warn` - 只显示 warn/error
/// - `RUST_LOG=error` - 只显示 error
///
/// # 示例
///
/// ```bash
/// # 显示所有日志
/// RUST_LOG=debug cargo run
///
/// # 只显示模块特定日志
/// RUST_LOG=ikun_desktop_organization=debug cargo run
/// ```
pub fn init_logger() {
    // env_logger::init() 会读取 RUST_LOG 环境变量
    // 如果没有设置，默认只显示 error 级别
    env_logger::init();
}

/// 打印调试信息（仅在 debug 模式下显示）
///
/// 用于开发阶段的详细信息输出
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        log::debug!("[{}:{}] {}", file!(), line!(), format_args!($($arg)*))
    };
}

/// 打印信息日志
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        log::info!("[{}] {}", module_path!(), format_args!($($arg)*))
    };
}

/// 打印警告日志
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        log::warn!("[{}] {}", module_path!(), format_args!($($arg)*))
    };
}

/// 打印错误日志
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        log::error!("[{}] {}", module_path!(), format_args!($($arg)*))
    };
}

/// 打印带时间戳的性能日志
pub fn log_perf(operation: &str, elapsed_ms: u128) {
    info!("[性能] {} 耗时: {}ms", operation, elapsed_ms);
}

/// 打印分隔线（用于调试时区分不同阶段）
pub fn log_separator(title: &str) {
    debug!("{}", "=".repeat(50));
    debug!("  {}", title);
    debug!("{}", "=".repeat(50));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_macros() {
        init_logger();
        log_debug!("这是调试信息");
        log_info!("这是普通信息");
        log_warn!("这是警告信息");
        log_error!("这是错误信息");
    }
}
