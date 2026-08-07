//! Windows 平台模块
//!
//! 提供 Windows 平台的桌面管理、窗口管理和渲染功能�?
//!
//! # 子模�?
//! - `ffi_types`: Win32 FFI 类型定义、C 结构体和常量
//! - `ffi_imports`: Win32 FFI 函数声明�?Shell COM 类型
//! - `desktop_manager`: Windows DesktopManager 实现
//! - `d2d_render`: Direct2D/DirectWrite 渲染
//! - `icon_bitmap`: 图标透明边界与等比居中计算
//! - `grid_window`: 格子窗口管理
//! - `icon_extract`: 文件图标提取
//! - `icon_hider`: 图标仓库（managed_icons，无需管理员）
//! - `icon_manager`: 逐图标管理（LVM_SETITEMPOSITION 跨进程操作）
//! - `shell_menu`: Shell 上下文菜�?+ 注册表管�?
//! - `sys_tray`: 系统托盘（TODO�?

pub mod alignment_guide;
pub mod app_icon;
mod control_panel_window;
mod d2d_render;
pub mod desktop_manager;
pub mod desktop_utils;
pub mod drag_drop;
pub mod drag_handler;
mod drag_image;
pub mod drag_visual;
mod drop_target;
pub mod ffi_imports;
pub mod ffi_types;
pub mod global_hotkey;
pub mod grid_layout;
pub mod grid_window;
mod icon_bitmap;
pub mod icon_extract;
pub mod icon_hider;
pub mod icon_manager;
pub mod liquid_glass;
mod menu_window;
mod monitor_utils;
pub mod multi_select;
pub mod ole_dnd;
pub mod rename_editor;
pub mod shell_menu;
mod sys_tray;
mod wnd_proc;
pub mod zorder;

// Re-export key items
pub use control_panel_window::{hide_control_panel_window, show_control_panel_window};
pub use desktop_manager::{clsid_display_name, DesktopManager};
pub use desktop_utils::{
    is_desktop_auto_arrange_enabled, position_icon_at_screen as position_icon_on_desktop,
};
#[allow(unused_imports)] // 公共 API 预留
pub use drop_target::{
    register_box_drop_target, take_pending_drop, take_pending_source_removal,
    unregister_box_drop_target,
};
#[allow(unused_imports)] // 公共 API 预留
pub use ffi_types::{IkunBoxDragData, IkunDragItemData};
#[allow(unused_imports)]
pub use global_hotkey::{setup_global_hotkey, take_hotkey_toggle, unregister_global_hotkey};
pub use grid_window::{
    collect_grid_layout, create_grid_window, destroy_all_grid_windows, get_grid_count,
    update_grid_window_title, GridCreateParams, GridFileItem,
};
pub use icon_extract::extract_file_icon;
#[allow(unused_imports)] // 公共 API 预留
#[allow(unused_imports)] // 公共 API 预留
pub use icon_hider::{
    get_vault_display_names, icon_exists_for_box, list_vault_icons, resolve_icon_read_path,
    restore_all_hidden_icons,
};
#[allow(unused_imports)] // 公共 API 预留
pub use icon_manager::DesktopIconManager;
pub use monitor_utils::{
    get_cursor_monitor_work_area_phys, get_primary_monitor_dpi, get_primary_monitor_work_area_phys,
    get_screen_resolution, reposition_and_resize_window, reposition_window_by_title,
};
#[allow(unused_imports)] // 公共 API 预留
pub use ole_dnd::{get_box_drag_format_id, start_box_drag};
pub use shell_menu::{
    cleanup_desktop_registry, setup_desktop_registry, show_desktop_context_menu,
    take_pending_create_box,
};
#[allow(unused_imports)] // 公共 API 预留
pub use sys_tray::{
    cleanup_system_tray, register_tray_action_waker, set_tray_auto_start_state, setup_system_tray,
    take_pending_tray_action,
};
#[allow(unused_imports)] // Z-order 公共 API 预留
pub use zorder::{find_grid_window_at_point, push_other_grids_below, set_drag_in_progress};

#[allow(unused_imports)] // multi_select public API
pub use multi_select::{
    add, add_range, all_keys, anchor, clear, container_id_from_uuid, count, count_in_container,
    has_selection, is_selected, remove, selected_in_container, selected_paths_in_container,
    set_anchor, set_range, toggle, ContainerType, SelectionKey,
};


// =============================================================================
// WindowsPlatformService: implements crate::platform::PlatformService
// =============================================================================

/// Windows platform service implementation.
///
/// Consolidates Windows-specific platform operations into a single struct
/// that implements the cross-platform PlatformService trait.
pub struct WindowsPlatformService;

impl crate::platform::PlatformService for WindowsPlatformService {
    fn get_grid_count(&self) -> usize {
        grid_window::get_grid_count()
    }

    fn collect_grid_layout(&self) -> Vec<crate::config::app_config::BoxLayoutData> {
        grid_window::collect_grid_layout()
    }

    fn setup_system_tray(&self) -> anyhow::Result<()> {
        sys_tray::setup_system_tray()
    }

    fn cleanup_system_tray(&self) {
        sys_tray::cleanup_system_tray()
    }

    fn take_pending_tray_action(&self) -> Option<crate::platform::SystemTrayAction> {
        sys_tray::take_pending_tray_action()
    }

    fn set_tray_auto_start_state(&self, enabled: bool) {
        sys_tray::set_tray_auto_start_state(enabled)
    }

    fn show_control_panel(&self) -> bool {
        control_panel_window::show_control_panel_window("Desktop Organizer")
    }

    fn hide_control_panel(&self) -> bool {
        control_panel_window::hide_control_panel_window("Desktop Organizer")
    }

    fn set_auto_start(&self, enabled: bool) -> anyhow::Result<()> {
        crate::utils::platform::set_auto_start(enabled)
    }

    fn is_auto_start_enabled(&self) -> bool {
        crate::utils::platform::is_auto_start_enabled()
    }
}
