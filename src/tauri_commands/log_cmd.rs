//! Frontend IPC log command.
//!
//! Allows Svelte pages to send log messages to the Rust backend,
//! where they are written to the unified log file with window context.

/// Receive a log message from the frontend and write it to the Rust log.
///
/// The window label is automatically extracted from the Tauri window parameter
/// and included in the log line as `[frontend:<window-label>]`.
#[tauri::command]
pub fn log_message(window: tauri::Window, level: String, message: String) {
    let label = window.label();
    match level.as_str() {
        "debug" => log::debug!("[frontend:{}] {}", label, message),
        "info" => log::info!("[frontend:{}] {}", label, message),
        "warn" => log::warn!("[frontend:{}] {}", label, message),
        "error" => log::error!("[frontend:{}] {}", label, message),
        _ => log::info!("[frontend:{}] {}", label, message),
    }
}
