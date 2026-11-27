//! Tauri Commands module
//!
//! Front-end (Svelte) uses these commands through Tauri IPC.
//! Each sub-module contains commands related to a specific domain.

pub mod ai_cmd;
pub mod app_cmd;
pub mod box_cmd;
pub mod config_cmd;
pub mod log_cmd;
pub mod platform_runtime;
pub mod update_cmd;
pub mod window_cmd;

// Re-export all commands for easy registration
pub use ai_cmd::*;
pub use app_cmd::*;
pub use box_cmd::*;
pub use config_cmd::*;
pub use log_cmd::*;
pub use update_cmd::*;
pub use window_cmd::*;
