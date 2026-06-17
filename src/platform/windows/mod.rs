//! Windows 平台模块
//!
//! 提供 Windows 平台的桌面管理、窗口管理和渲染功能。
//!
//! # 子模块
//! - `ffi_types`: Win32 FFI 类型定义、C 结构体和常量
//! - `ffi_imports`: Win32 FFI 函数声明和 Shell COM 类型
//! - `desktop_manager`: Windows DesktopManager 实现
//! - `d2d_render`: Direct2D/DirectWrite 渲染
//! - `grid_window`: 格子窗口管理
//! - `icon_extract`: 文件图标提取
//! - `icon_hider`: 图标隐藏/恢复
//! - `shell_menu`: Shell 上下文菜单 + 注册表管理
//! - `sys_tray`: 系统托盘（TODO）

mod d2d_render;
pub mod desktop_manager;
pub mod ffi_imports;
pub mod ffi_types;
pub mod grid_window;
pub mod icon_extract;
pub mod icon_hider;
pub mod shell_menu;
mod sys_tray;

// Re-export key items
pub use desktop_manager::DesktopManager;
pub use grid_window::{
    collect_grid_layout, create_grid_window, destroy_all_grid_windows,
    get_cursor_monitor_work_area_phys, get_grid_count, get_screen_resolution,
    reposition_window_by_title, GridCreateParams, GridFileItem,
};
pub use icon_extract::extract_file_icon;
pub use icon_hider::{hide_native_icons, restore_native_icons};
pub use shell_menu::{cleanup_desktop_registry, setup_desktop_registry, take_pending_create_box};
