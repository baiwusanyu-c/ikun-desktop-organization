//! 日志工具函数
//!
//! 提供统一的日志输出格式和便捷函数。
//! 日志写入文件 `~/.ikun_organizer_log/` 目录，按日期和序号分文件存储。
//!
//! ## 日志文件规则
//! - 文件命名：`.ikun_organizer_log_YYYYMMDD_N`（如 `.ikun_organizer_log_20260722_1`）
//! - 存储目录：`~/.ikun_organizer_log/`
//! - 大小限制：单文件超过 2MB 自动切换到下一个序号
//! - 自动清理：启动时删除超过 7 天的日志文件
//!
//! ## 发行版日志
//! - `release_info!` / `release_warn!` / `release_error!` 宏带有 `[RELEASE]` 前缀
//! - 用于生产环境问题排查，不应作为调试代码删除

#![allow(dead_code)]

use std::cell::RefCell;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 单个日志文件的最大大小（2MB）
const MAX_LOG_FILE_SIZE: u64 = 2 * 1024 * 1024;

/// 日志保留天数
const LOG_RETENTION_DAYS: u64 = 7;

/// 日志文件名前缀
const LOG_FILE_PREFIX: &str = ".ikun_organizer_log_";

// =============================================================================
// Window Context Logging
// =============================================================================

thread_local! {
    /// Current window label for log context tagging.
    static CURRENT_WINDOW: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Set the current log context window label.
///
/// Called at the entry of Tauri commands to tag all subsequent log lines
/// with the originating window (e.g., "control-panel", "create-box", "edit-box").
pub fn set_log_window(label: &str) {
    CURRENT_WINDOW.with(|w| *w.borrow_mut() = Some(label.to_string()));
}

/// Clear the current log context window label.
pub fn clear_log_window() {
    CURRENT_WINDOW.with(|w| *w.borrow_mut() = None);
}

/// Get the current window label (thread-local).
fn current_window_label() -> Option<String> {
    CURRENT_WINDOW.with(|w| w.borrow().clone())
}

/// RAII guard that sets a window log context on creation and clears it on drop.
///
/// # Example
/// ```rust
/// #[tauri::command]
/// pub fn get_config(window: tauri::Window, state: ...) -> ... {
///     let _guard = WindowLogGuard::new(window.label());
///     // All log::xxx! calls now include [window:control-panel]
///     ...
/// }
/// ```
pub struct WindowLogGuard {
    _label: String,
}

impl WindowLogGuard {
    /// Create a new guard, setting the window label for the current thread.
    pub fn new(label: &str) -> Self {
        set_log_window(label);
        Self {
            _label: label.to_string(),
        }
    }
}

impl Drop for WindowLogGuard {
    fn drop(&mut self) {
        clear_log_window();
    }
}

/// 初始化日志系统（写入文件）
///
/// 日志级别通过环境变量 RUST_LOG 控制：
/// - `RUST_LOG=debug` - 显示所有日志
/// - `RUST_LOG=info` - 只显示 info/warn/error
/// - `RUST_LOG=warn` - 只显示 warn/error
/// - `RUST_LOG=error` - 只显示 error
///
/// 日志目录：`~/.ikun_organizer_log/`
/// 日志文件命名：`.ikun_organizer_log_YYYYMMDD_N`
///
/// # 示例
///
/// ```bash
/// # PowerShell 设置日志级别
/// $env:RUST_LOG="debug"; cargo run
/// ```
pub fn init_logger() {
    let log_dir = get_log_dir();

    // 创建日志目录
    if let Err(e) = fs::create_dir_all(&log_dir) {
        eprintln!("无法创建日志目录 {}: {}", log_dir.display(), e);
        env_logger::init();
        return;
    }

    // 清理过期日志
    cleanup_old_logs(&log_dir, LOG_RETENTION_DAYS);

    // 创建滚动文件写入器
    match RollingFileWriter::new(log_dir) {
        Ok(writer) => {
            env_logger::Builder::from_default_env()
                .target(env_logger::Target::Pipe(Box::new(writer)))
                .format(|buf, record| {
                    let window_tag = match current_window_label() {
                        Some(label) => format!(" [window:{}]", label),
                        None => String::new(),
                    };
                    writeln!(
                        buf,
                        "[{} {} {}]{} {}",
                        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                        record.level(),
                        record.target(),
                        window_tag,
                        record.args()
                    )
                })
                .init();
        }
        Err(e) => {
            eprintln!("无法初始化日志系统: {}", e);
            env_logger::init();
        }
    }
}

/// 获取日志目录路径
fn get_log_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".ikun_organizer_log"))
        .unwrap_or_else(|| std::env::temp_dir().join(".ikun_organizer_log"))
}

/// 清理过期日志文件
///
/// 遍历日志目录，删除日期距今超过 `max_days` 天的日志文件。
/// 清理失败不会阻塞应用启动。
fn cleanup_old_logs(log_dir: &Path, max_days: u64) {
    let cutoff = chrono::Local::now() - chrono::Duration::days(max_days as i64);
    let cutoff_date = cutoff.format("%Y%m%d").to_string();

    log_cleanup_info(&cutoff_date, max_days);

    let entries = match fs::read_dir(log_dir) {
        Ok(entries) => entries,
        Err(e) => {
            log_cleanup_error(&e);
            return;
        }
    };

    for entry in entries.flatten() {
        cleanup_entry(&entry, &cutoff_date, log_dir);
    }
}

/// 输出清理开始信息（使用 eprintln 避免循环依赖）
fn log_cleanup_info(cutoff_date: &str, max_days: u64) {
    eprintln!(
        "[日志清理] 清理 {} 天前的日志（日期 < {}）",
        max_days, cutoff_date
    );
}

/// 输出清理错误信息（使用 eprintln 避免循环依赖）
fn log_cleanup_error(e: &io::Error) {
    eprintln!("[日志清理] 无法读取日志目录: {}", e);
}

/// 处理单个日志文件的清理：检查文件名日期并删除过期文件
fn cleanup_entry(entry: &fs::DirEntry, cutoff_date: &str, log_dir: &Path) {
    let file_name = entry.file_name();
    let name = file_name.to_string_lossy();

    // 只处理日志文件
    if !name.starts_with(LOG_FILE_PREFIX) {
        return;
    }

    // 从文件名中提取日期
    if let Some(date_str) = extract_date_from_filename(&name) {
        if date_str < cutoff_date {
            let path = log_dir.join(entry.file_name());
            match fs::remove_file(&path) {
                Ok(()) => {
                    eprintln!("[日志清理] 已删除过期日志: {}", name);
                }
                Err(e) => {
                    eprintln!("[日志清理] 删除失败 {}: {}", name, e);
                }
            }
        }
    }
}

/// 从文件名中提取日期字符串
///
/// 文件名格式：`.ikun_organizer_log_YYYYMMDD_N`
/// 返回日期部分（8位数字）的字符串切片，格式无效时返回 None。
fn extract_date_from_filename(name: &str) -> Option<&str> {
    let suffix = name.strip_prefix(LOG_FILE_PREFIX)?;
    // suffix 格式：YYYYMMDD_N
    let underscore_pos = suffix.find('_')?;
    let date_part = &suffix[..underscore_pos];
    if date_part.len() == 8 && date_part.chars().all(|c| c.is_ascii_digit()) {
        Some(date_part)
    } else {
        None
    }
}

/// 查找当前日期日志文件的下一个可用序号
///
/// 扫描日志目录中同一日期的已有文件，找到最大序号。
/// 如果最后一个文件未满 2MB，继续使用该序号；否则使用下一个序号。
/// 如果当天没有日志文件，返回 1。
fn find_next_index(log_dir: &Path, date_str: &str) -> u32 {
    let prefix = format!("{}{}_", LOG_FILE_PREFIX, date_str);
    let mut max_index: u32 = 0;

    if let Ok(entries) = fs::read_dir(log_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(suffix) = name.strip_prefix(&prefix) {
                if let Ok(idx) = suffix.parse::<u32>() {
                    max_index = max_index.max(idx);
                }
            }
        }
    }

    // 如果已有文件，检查最后一个文件是否已满 2MB
    if max_index > 0 {
        let filename = format!("{}{}_{}", LOG_FILE_PREFIX, date_str, max_index);
        let path = log_dir.join(&filename);
        if let Ok(metadata) = fs::metadata(&path) {
            if metadata.len() < MAX_LOG_FILE_SIZE {
                return max_index;
            }
        }
        return max_index + 1;
    }

    1
}

/// 滚动文件写入器
///
/// 实现 `std::io::Write`，在每次写入时检查：
/// 1. 日期是否变化 → 切换到新日期的文件（序号从 1 开始）
/// 2. 文件大小是否超过 2MB → 序号+1，打开新文件
///
/// 使用 `Mutex` 保护内部状态，保证多线程安全。
struct RollingFileWriter {
    inner: Mutex<RollingWriterInner>,
    log_dir: PathBuf,
}

/// `RollingFileWriter` 内部状态
struct RollingWriterInner {
    current_file: Option<File>,
    current_size: u64,
    current_date: String,
    current_index: u32,
}

impl RollingFileWriter {
    /// 创建新的滚动文件写入器
    ///
    /// `log_dir` 为日志存储目录路径，会立即打开当天的日志文件。
    fn new(log_dir: PathBuf) -> io::Result<Self> {
        let today = chrono::Local::now().format("%Y%m%d").to_string();
        let index = find_next_index(&log_dir, &today);

        let mut writer = RollingFileWriter {
            inner: Mutex::new(RollingWriterInner {
                current_file: None,
                current_size: 0,
                current_date: today,
                current_index: index,
            }),
            log_dir,
        };

        writer.open_current_file()?;
        Ok(writer)
    }

    /// 打开当前日期+序号对应的日志文件
    fn open_current_file(&mut self) -> io::Result<()> {
        let inner = self
            .inner
            .get_mut()
            .map_err(|_| io::Error::other("Mutex poisoned"))?;

        let filename = format!(
            "{}{}_{}",
            LOG_FILE_PREFIX, inner.current_date, inner.current_index
        );
        let path = self.log_dir.join(&filename);

        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let metadata = file.metadata()?;
        inner.current_file = Some(file);
        inner.current_size = metadata.len();

        Ok(())
    }

    /// 获取当前日志文件路径（用于测试和调试）
    fn current_path(&self) -> Option<PathBuf> {
        let inner = self.inner.lock().ok()?;
        let filename = format!(
            "{}{}_{}",
            LOG_FILE_PREFIX, inner.current_date, inner.current_index
        );
        Some(self.log_dir.join(&filename))
    }

    /// 获取当前日期和序号（用于测试和调试）
    fn current_date_index(&self) -> Option<(String, u32)> {
        let inner = self.inner.lock().ok()?;
        Some((inner.current_date.clone(), inner.current_index))
    }
}

impl Write for RollingFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("Mutex poisoned"))?;

        let today = chrono::Local::now().format("%Y%m%d").to_string();

        // 检查日期是否变化
        if today != inner.current_date {
            inner.current_date = today.clone();
            inner.current_index = find_next_index(&self.log_dir, &today);
            inner.current_file = None;
            inner.current_size = 0;
        }

        // 检查文件大小是否超过限制
        if inner.current_size + buf.len() as u64 > MAX_LOG_FILE_SIZE {
            inner.current_index += 1;
            inner.current_file = None;
            inner.current_size = 0;
        }

        // 如果需要，打开新文件
        if inner.current_file.is_none() {
            let filename = format!(
                "{}{}_{}",
                LOG_FILE_PREFIX, inner.current_date, inner.current_index
            );
            let path = self.log_dir.join(&filename);
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            inner.current_file = Some(file);
            inner.current_size = 0;
        }

        // 写入数据
        if let Some(ref mut file) = inner.current_file {
            let written = file.write(buf)?;
            inner.current_size += written as u64;
            Ok(written)
        } else {
            Err(io::Error::other("无法打开日志文件"))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("Mutex poisoned"))?;
        if let Some(ref mut file) = inner.current_file {
            file.flush()?;
        }
        Ok(())
    }
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

// =====================================================================
// 发行版日志宏
// =====================================================================
//
// 以下宏带有 `[RELEASE]` 前缀，用于生产环境问题排查。
// 这些日志调用是永久性代码，不应作为调试代码或死代码删除。
// 在 release 构建中，这些日志始终会输出（受 RUST_LOG 级别控制）。

/// 发行版信息日志 — 用于记录生产环境中的关键操作
///
/// 带有 `[RELEASE]` 前缀，用于生产环境问题排查。
/// 这些日志是永久性代码，不应被删除。
///
/// # 示例
/// ```rust
/// release_info!("应用启动，版本: {}", env!("CARGO_PKG_VERSION"));
/// release_info!("配置加载完成，分类框数量: {}", count);
/// ```
#[macro_export]
macro_rules! release_info {
    ($($arg:tt)*) => {
        log::info!("[RELEASE] [{}] {}", module_path!(), format_args!($($arg)*))
    };
}

/// 发行版警告日志 — 用于记录生产环境中的非致命异常
///
/// 带有 `[RELEASE]` 前缀，用于生产环境问题排查。
/// 这些日志是永久性代码，不应被删除。
///
/// # 示例
/// ```rust
/// release_warn!("配置加载失败: {}，使用默认配置", e);
/// ```
#[macro_export]
macro_rules! release_warn {
    ($($arg:tt)*) => {
        log::warn!("[RELEASE] [{}] {}", module_path!(), format_args!($($arg)*))
    };
}

/// 发行版错误日志 — 用于记录生产环境中的严重错误
///
/// 带有 `[RELEASE]` 前缀，用于生产环境问题排查。
/// 这些日志是永久性代码，不应被删除。
///
/// # 示例
/// ```rust
/// release_error!("窗口创建失败: {}", e);
/// ```
#[macro_export]
macro_rules! release_error {
    ($($arg:tt)*) => {
        log::error!("[RELEASE] [{}] {}", module_path!(), format_args!($($arg)*))
    };
}

/// 打印带时间戳的性能日志
pub fn log_perf(operation: &str, elapsed_ms: u128) {
    log::info!("[性能] {} 耗时: {}ms", operation, elapsed_ms);
}

/// 打印分隔线（用于调试时区分不同阶段）
pub fn log_separator(title: &str) {
    log::debug!("{}", "=".repeat(50));
    log::debug!("  {}", title);
    log::debug!("{}", "=".repeat(50));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 全局递增计数器，保证并行测试时目录名唯一
    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 创建隔离的临时测试目录（使用唯一 ID 防止并行测试冲突）
    fn make_test_dir(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join("ikun_logger_test").join(format!(
            "{}_{}_{}",
            name,
            std::process::id(),
            id
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 清理测试目录
    fn cleanup_test_dir(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    // =====================================================================
    // extract_date_from_filename
    // =====================================================================

    #[test]
    fn test_extract_date_valid_single_digit_index() {
        assert_eq!(
            extract_date_from_filename(".ikun_organizer_log_20260722_1"),
            Some("20260722")
        );
    }

    #[test]
    fn test_extract_date_valid_multi_digit_index() {
        assert_eq!(
            extract_date_from_filename(".ikun_organizer_log_20260722_15"),
            Some("20260722")
        );
    }

    #[test]
    fn test_extract_date_valid_large_index() {
        assert_eq!(
            extract_date_from_filename(".ikun_organizer_log_20260101_999"),
            Some("20260101")
        );
    }

    #[test]
    fn test_extract_date_invalid_no_prefix() {
        assert_eq!(extract_date_from_filename("invalid_file"), None);
    }

    #[test]
    fn test_extract_date_invalid_empty_suffix() {
        assert_eq!(extract_date_from_filename(".ikun_organizer_log_"), None);
    }

    #[test]
    fn test_extract_date_invalid_short_date() {
        assert_eq!(
            extract_date_from_filename(".ikun_organizer_log_2026072_1"),
            None
        );
    }

    #[test]
    fn test_extract_date_invalid_long_date() {
        assert_eq!(
            extract_date_from_filename(".ikun_organizer_log_202607220_1"),
            None
        );
    }

    #[test]
    fn test_extract_date_invalid_non_digit_date() {
        assert_eq!(
            extract_date_from_filename(".ikun_organizer_log_2026072a_1"),
            None
        );
    }

    #[test]
    fn test_extract_date_invalid_no_underscore_after_date() {
        assert_eq!(
            extract_date_from_filename(".ikun_organizer_log_20260722"),
            None
        );
    }

    // =====================================================================
    // get_log_dir
    // =====================================================================

    #[test]
    fn test_get_log_dir_contains_name() {
        let dir = get_log_dir();
        assert!(dir.to_string_lossy().contains(".ikun_organizer_log"));
    }

    #[test]
    fn test_get_log_dir_is_absolute() {
        let dir = get_log_dir();
        assert!(dir.is_absolute(), "日志目录应为绝对路径");
    }

    // =====================================================================
    // find_next_index
    // =====================================================================

    #[test]
    fn test_find_next_index_empty_dir() {
        let dir = make_test_dir("fni_empty");
        let index = find_next_index(&dir, "20260722");
        assert_eq!(index, 1, "空目录应从序号 1 开始");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_find_next_index_one_file_under_limit() {
        let dir = make_test_dir("fni_one_under");
        File::create(dir.join(".ikun_organizer_log_20260722_1")).unwrap();
        let index = find_next_index(&dir, "20260722");
        assert_eq!(index, 1, "未满 2MB 应继续使用当前序号");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_find_next_index_one_file_over_limit() {
        let dir = make_test_dir("fni_one_over");
        let path = dir.join(".ikun_organizer_log_20260722_1");
        let data = vec![0u8; (MAX_LOG_FILE_SIZE + 1) as usize];
        File::create(&path).unwrap().write_all(&data).unwrap();

        let index = find_next_index(&dir, "20260722");
        assert_eq!(index, 2, "超过 2MB 应使用下一个序号");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_find_next_index_multiple_files_last_under_limit() {
        let dir = make_test_dir("fni_multi_under");
        File::create(dir.join(".ikun_organizer_log_20260722_1")).unwrap();
        File::create(dir.join(".ikun_organizer_log_20260722_2")).unwrap();
        File::create(dir.join(".ikun_organizer_log_20260722_3")).unwrap();
        let index = find_next_index(&dir, "20260722");
        assert_eq!(index, 3);
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_find_next_index_multiple_files_last_over_limit() {
        let dir = make_test_dir("fni_multi_over");
        File::create(dir.join(".ikun_organizer_log_20260722_1")).unwrap();
        let data = vec![0u8; (MAX_LOG_FILE_SIZE + 1) as usize];
        File::create(dir.join(".ikun_organizer_log_20260722_2"))
            .unwrap()
            .write_all(&data)
            .unwrap();

        let index = find_next_index(&dir, "20260722");
        assert_eq!(index, 3, "最后一个文件超 2MB 应跳到下一序号");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_find_next_index_ignores_other_dates() {
        let dir = make_test_dir("fni_other_date");
        File::create(dir.join(".ikun_organizer_log_20260721_5")).unwrap();
        File::create(dir.join(".ikun_organizer_log_20260721_6")).unwrap();
        let index = find_next_index(&dir, "20260722");
        assert_eq!(index, 1, "不同日期的文件不应影响序号");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_find_next_index_ignores_non_log_files() {
        let dir = make_test_dir("fni_non_log");
        File::create(dir.join("random_file.txt")).unwrap();
        File::create(dir.join("not_a_log_20260722_1")).unwrap();
        let index = find_next_index(&dir, "20260722");
        assert_eq!(index, 1, "非日志文件不应影响序号");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_find_next_index_nonexistent_dir() {
        let dir = PathBuf::from("/nonexistent_path_for_test_12345");
        let index = find_next_index(&dir, "20260722");
        assert_eq!(index, 1, "不存在的目录应返回 1");
    }

    // =====================================================================
    // cleanup_old_logs
    // =====================================================================

    #[test]
    fn test_cleanup_removes_old_files() {
        let dir = make_test_dir("cleanup_old");
        File::create(dir.join(".ikun_organizer_log_20260101_1")).unwrap();
        let today = chrono::Local::now().format("%Y%m%d").to_string();
        File::create(dir.join(format!(".ikun_organizer_log_{}_1", today))).unwrap();

        cleanup_old_logs(&dir, 7);

        assert!(
            !dir.join(".ikun_organizer_log_20260101_1").exists(),
            "30 天前的文件应被清理"
        );
        assert!(
            dir.join(format!(".ikun_organizer_log_{}_1", today))
                .exists(),
            "今天的文件应保留"
        );
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_cleanup_keeps_recent_files() {
        let dir = make_test_dir("cleanup_recent");
        let three_days_ago = (chrono::Local::now() - chrono::Duration::days(3))
            .format("%Y%m%d")
            .to_string();
        File::create(dir.join(format!(".ikun_organizer_log_{}_1", three_days_ago))).unwrap();

        cleanup_old_logs(&dir, 7);

        assert!(
            dir.join(format!(".ikun_organizer_log_{}_1", three_days_ago))
                .exists(),
            "3 天前的文件不应被清理"
        );
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_cleanup_ignores_non_log_files() {
        let dir = make_test_dir("cleanup_nonlog");
        File::create(dir.join("important_data.txt")).unwrap();
        File::create(dir.join("config.json")).unwrap();

        cleanup_old_logs(&dir, 7);

        assert!(dir.join("important_data.txt").exists());
        assert!(dir.join("config.json").exists());
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_cleanup_handles_empty_dir() {
        let dir = make_test_dir("cleanup_empty");
        cleanup_old_logs(&dir, 7);
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_cleanup_handles_nonexistent_dir() {
        let dir = PathBuf::from("/nonexistent_test_dir_99999");
        cleanup_old_logs(&dir, 7);
    }

    // =====================================================================
    // RollingFileWriter
    // =====================================================================

    #[test]
    fn test_writer_creates_file_on_init() {
        let dir = make_test_dir("w_init");
        let writer = RollingFileWriter::new(dir.clone()).unwrap();
        let path = writer.current_path().unwrap();
        assert!(path.exists(), "初始化后应创建日志文件");

        let filename = path.file_name().unwrap().to_string_lossy();
        assert!(filename.starts_with(LOG_FILE_PREFIX));
        assert!(extract_date_from_filename(&filename).is_some());
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_writer_basic_write() {
        let dir = make_test_dir("w_write");
        let mut writer = RollingFileWriter::new(dir.clone()).unwrap();

        let data = b"Hello, log!\n";
        let written = writer.write(data).unwrap();
        writer.flush().unwrap();
        assert_eq!(written, data.len());

        let path = writer.current_path().unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Hello, log!\n");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_writer_multiple_writes_append() {
        let dir = make_test_dir("w_append");
        let mut writer = RollingFileWriter::new(dir.clone()).unwrap();

        writer.write_all(b"line1\n").unwrap();
        writer.write_all(b"line2\n").unwrap();
        writer.write_all(b"line3\n").unwrap();
        writer.flush().unwrap();

        let path = writer.current_path().unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "line1\nline2\nline3\n");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_writer_size_rotation() {
        let dir = make_test_dir("w_rotate");
        let today = chrono::Local::now().format("%Y%m%d").to_string();
        let file1 = dir.join(format!(".ikun_organizer_log_{}_1", today));

        // 预先填充文件到 MAX - 100 bytes
        {
            let mut f = File::create(&file1).unwrap();
            f.write_all(&vec![b'.'; (MAX_LOG_FILE_SIZE - 100) as usize])
                .unwrap();
        }

        // Writer 检测到 _1 未满 2MB，继续使用序号 1
        let mut writer = RollingFileWriter::new(dir.clone()).unwrap();
        let (_, initial_index) = writer.current_date_index().unwrap();
        assert_eq!(initial_index, 1, "未满 2MB 应继续使用序号 1");

        // 写入 50 字节 → 文件大小变为 MAX - 50，不超过限制
        writer.write_all(&[b'X'; 50]).unwrap();
        writer.flush().unwrap();
        let (_, idx1) = writer.current_date_index().unwrap();
        assert_eq!(idx1, 1, "未超过限制时不应轮转");

        // 写入 200 字节 → MAX - 50 + 200 > MAX → 触发轮转到序号 2
        writer.write_all(&[b'Y'; 200]).unwrap();
        writer.flush().unwrap();

        let (_, new_index) = writer.current_date_index().unwrap();
        assert_eq!(new_index, 2, "超过 2MB 后序号应递增到 2");

        // 新文件包含刚写入的 200 字节
        let new_path = writer.current_path().unwrap();
        assert!(new_path.exists(), "轮转后应创建新文件");
        let content = fs::read_to_string(&new_path).unwrap();
        assert_eq!(content.len(), 200, "新文件应仅包含轮转后的数据");

        // 继续写入应正常追加
        writer.write_all(b"more\n").unwrap();
        writer.flush().unwrap();
        let content = fs::read_to_string(&new_path).unwrap();
        assert_eq!(content.len(), 205, "追加写入应正常");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_writer_exact_size_limit() {
        let dir = make_test_dir("w_exact");
        let today = chrono::Local::now().format("%Y%m%d").to_string();
        let file1 = dir.join(format!(".ikun_organizer_log_{}_1", today));

        // 预先填充文件到 MAX - 1 bytes
        {
            let mut f = File::create(&file1).unwrap();
            f.write_all(&vec![b'.'; (MAX_LOG_FILE_SIZE - 1) as usize])
                .unwrap();
        }

        let mut writer = RollingFileWriter::new(dir.clone()).unwrap();
        let (_, initial_index) = writer.current_date_index().unwrap();
        assert_eq!(initial_index, 1);

        // 写入 1 字节 → 文件大小恰好等于 MAX（条件是 > MAX，不是 >=）→ 不轮转
        writer.write_all(b"Z").unwrap();
        writer.flush().unwrap();
        let (_, idx_exact) = writer.current_date_index().unwrap();
        assert_eq!(idx_exact, 1, "刚好到达限制时不应轮转");

        // 再写入 1 字节 → MAX + 1 > MAX → 触发轮转
        writer.write_all(b"A").unwrap();
        writer.flush().unwrap();
        let (_, idx_over) = writer.current_date_index().unwrap();
        assert_eq!(idx_over, 2, "超过限制后应轮转");

        // 新文件只包含 "A"
        let new_path = writer.current_path().unwrap();
        let content = fs::read_to_string(&new_path).unwrap();
        assert_eq!(content, "A", "新文件应仅包含触发轮转后写入的数据");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_writer_empty_write() {
        let dir = make_test_dir("w_empty");
        let mut writer = RollingFileWriter::new(dir.clone()).unwrap();

        let (_, initial_index) = writer.current_date_index().unwrap();
        let written = writer.write(b"").unwrap();
        assert_eq!(written, 0);

        let (_, idx_after) = writer.current_date_index().unwrap();
        assert_eq!(idx_after, initial_index, "空写入不应触发轮转");
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_writer_flush_is_safe() {
        let dir = make_test_dir("w_flush");
        let mut writer = RollingFileWriter::new(dir.clone()).unwrap();

        writer.flush().unwrap();
        writer.write_all(b"data\n").unwrap();
        writer.flush().unwrap();
        writer.flush().unwrap();
        cleanup_test_dir(&dir);
    }

    #[test]
    fn test_writer_oversized_single_write() {
        // 单次写入超过 MAX_LOG_FILE_SIZE 时，应触发轮转但数据完整写入新文件
        let dir = make_test_dir("w_oversized");
        let mut writer = RollingFileWriter::new(dir.clone()).unwrap();
        let (_, initial_index) = writer.current_date_index().unwrap();

        // 写入 MAX + 10 字节（单次超大写入）
        let big = vec![b'B'; (MAX_LOG_FILE_SIZE + 10) as usize];
        writer.write_all(&big).unwrap();
        writer.flush().unwrap();

        // 应轮转到下一个序号
        let (_, new_index) = writer.current_date_index().unwrap();
        assert_eq!(new_index, initial_index + 1, "单次超大写入应触发轮转");

        // 新文件应包含完整数据
        let new_path = writer.current_path().unwrap();
        let metadata = fs::metadata(&new_path).unwrap();
        assert_eq!(
            metadata.len(),
            MAX_LOG_FILE_SIZE + 10,
            "新文件应包含完整的超大写入数据"
        );
        cleanup_test_dir(&dir);
    }

    // =========================================================================
    // Window Context Logging Tests
    // =========================================================================

    #[test]
    fn test_window_context_set_and_get() {
        set_log_window("control-panel");
        let label = current_window_label();
        assert_eq!(label, Some("control-panel".to_string()));

        clear_log_window();
        let label_after = current_window_label();
        assert_eq!(label_after, None);
    }

    #[test]
    fn test_window_context_thread_isolation() {
        use std::sync::Arc;
        use std::thread;

        set_log_window("control-panel");

        let result = Arc::new(Mutex::new(None));
        let result_clone = result.clone();

        let handle = thread::spawn(move || {
            let label = current_window_label();
            *result_clone.lock().unwrap() = label;
        });

        handle.join().unwrap();

        assert_eq!(current_window_label(), Some("control-panel".to_string()));

        let spawned_label = result.lock().unwrap().clone();
        assert_eq!(spawned_label, None);

        clear_log_window();
    }

    #[test]
    fn test_window_log_guard_cleanup() {
        {
            let _guard = WindowLogGuard::new("create-box");
            assert_eq!(current_window_label(), Some("create-box".to_string()));
        }
        assert_eq!(current_window_label(), None);
    }

    #[test]
    fn test_window_log_guard_nested() {
        let _outer = WindowLogGuard::new("control-panel");
        assert_eq!(current_window_label(), Some("control-panel".to_string()));

        {
            let _inner = WindowLogGuard::new("edit-box");
            assert_eq!(current_window_label(), Some("edit-box".to_string()));
        }
        assert_eq!(current_window_label(), None);
    }

    #[test]
    fn test_window_label_switch() {
        set_log_window("control-panel");
        assert_eq!(current_window_label(), Some("control-panel".to_string()));

        set_log_window("create-box");
        assert_eq!(current_window_label(), Some("create-box".to_string()));

        clear_log_window();
        assert_eq!(current_window_label(), None);
    }
}
