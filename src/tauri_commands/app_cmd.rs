//! Application control Tauri Commands
//!
//! Commands for application lifecycle management and system integration.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::tauri_commands::platform_runtime::SharedTauriRuntime;

/// Application information
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppInfo {
    pub version: String,
    pub name: String,
    pub build_date: String,
}

/// Get application information
#[tauri::command]
pub async fn get_app_info() -> Result<AppInfo, String> {
    log::info!("[tauri_cmd] get_app_info called");
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: env!("CARGO_PKG_NAME").to_string(),
        build_date: "2026-08-07".to_string(),
    })
}

pub(crate) fn request_quit(runtime: &SharedTauriRuntime) -> Result<(), String> {
    runtime
        .lock()
        .map_err(|error| format!("运行时状态不可用: {error}"))?
        .quit_requested = true;
    Ok(())
}

/// Quit the application
#[tauri::command]
pub fn quit_app(runtime: State<'_, SharedTauriRuntime>) -> Result<(), String> {
    log::info!("[tauri_cmd] quit_app called");
    request_quit(runtime.inner())
}

/// Get primary monitor resolution
#[tauri::command]
pub async fn get_screen_resolution() -> Result<(u32, u32), String> {
    use crate::platform::get_screen_resolution as platform_get_resolution;
    let (width, height) = platform_get_resolution();
    Ok((width as u32, height as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tauri_commands::platform_runtime::new_shared_runtime;

    #[test]
    fn quit_request_uses_runtime_cleanup_path() {
        let runtime = new_shared_runtime();
        request_quit(&runtime).unwrap();

        assert!(runtime.lock().unwrap().quit_requested);
    }
}
