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
//! - `icon_hider`: 图标仓库（managed_icons，无需管理员）
//! - `icon_manager`: 逐图标管理（LVM_SETITEMPOSITION 跨进程操作）
//! - `shell_menu`: Shell 上下文菜单 + 注册表管理
//! - `sys_tray`: 系统托盘（TODO）

mod control_panel_window;
mod d2d_render;
pub mod desktop_manager;
pub mod desktop_overlay;
pub mod drag_handler;
mod drag_image;
mod drop_target;
pub mod ffi_imports;
pub mod ffi_types;
pub mod grid_window;
pub mod icon_extract;
pub mod icon_hider;
pub mod icon_manager;
mod menu_window;
pub mod ole_dnd;
pub mod shell_menu;
mod sys_tray;
pub mod app_icon;
mod wnd_proc;

// Re-export key items
pub use control_panel_window::{hide_control_panel_window, show_control_panel_window};
pub use desktop_manager::DesktopManager;
pub use desktop_overlay::{create_desktop_overlay, destroy_desktop_overlay};
pub use drop_target::{
    register_box_drop_target, take_pending_drop, take_pending_source_removal,
    unregister_box_drop_target,
};
pub use ffi_types::{IkunBoxDragData, IkunDragItemData};
pub use grid_window::{
    collect_grid_layout, create_grid_window, destroy_all_grid_windows,
    get_cursor_monitor_work_area_phys, get_grid_count, get_screen_resolution,
    reposition_window_by_title, GridCreateParams, GridFileItem,
};
pub use icon_extract::extract_file_icon;
pub use icon_hider::{
    hide_native_icons, icon_exists_for_box, native_icons_hidden, resolve_icon_read_path,
    restore_all_hidden_icons, restore_native_icons,
};
pub use icon_manager::DesktopIconManager;
pub use ole_dnd::{get_box_drag_format_id, start_box_drag};
pub use shell_menu::{cleanup_desktop_registry, setup_desktop_registry, take_pending_create_box};
pub use sys_tray::{
    cleanup_system_tray, register_tray_action_waker, setup_system_tray, take_pending_tray_action,
};
