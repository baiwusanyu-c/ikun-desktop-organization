//! 平台抽象层

#![allow(dead_code)] // 平台抽象层，部分在当前构建路径未使用
//!
//! 提供跨平台的桌面管理接口，通过 trait 统一不同平台的实现。
//!
//! # 模块结构
//! - `types`: 跨平台数据类型（DesktopIcon, IconInfo 等）
//! - `windows`: Windows 平台实现（Shell API, D2D 渲染等）
//! - `macos`: macOS 平台实现（实验性）

pub mod types;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

// Re-export cross-platform types
pub use types::*;

/// 系统托盘菜单动作。
///
/// 平台层负责把原生托盘菜单事件转换为这些动作，UI 层负责消费并执行具体行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemTrayAction {
    /// 打开控制面板。
    OpenControlPanel,
    /// 创建分类框。
    CreateBox,
    /// 一键 AI 分类（当前仅占位）。
    AiClassify,
    /// 切换开机自启。
    ToggleAutoStart,
    /// 退出应用。
    Quit,
}

// Re-export platform-specific DesktopManager
#[cfg(target_os = "macos")]
pub use macos::DesktopManager;
#[cfg(windows)]
pub use windows::DesktopManager;

// Re-export platform-specific functions that app.rs uses
#[cfg(windows)]
#[allow(unused_imports)] // 部分仅在 eframe-ui feature 路径使用
pub use windows::{
    cleanup_desktop_registry, cleanup_system_tray, clsid_display_name, collect_grid_layout,
    create_desktop_overlay, create_grid_window, destroy_all_grid_windows, destroy_desktop_overlay,
    install_overlay_mouse_hook, take_pending_overlay_right_click, uninstall_overlay_mouse_hook,
    extract_file_icon, get_cursor_monitor_work_area_phys, get_grid_count, get_primary_monitor_dpi,
    get_primary_monitor_work_area_phys, get_screen_resolution, hide_control_panel_window,
    hide_native_icons, icon_exists_for_box, native_icons_hidden, register_tray_action_waker,
    reposition_and_resize_window, reposition_window_by_title, resolve_icon_read_path,
    restore_all_hidden_icons, restore_native_icons, set_tray_auto_start_state,
    setup_desktop_registry, setup_global_hotkey, setup_system_tray, show_control_panel_window,
    take_hotkey_toggle, show_desktop_context_menu, take_pending_create_box, take_pending_tray_action,
    unregister_global_hotkey, update_grid_window_title, GridCreateParams, GridFileItem,
};

// =============================================================================
// macOS 平台导出与 stub 函数
// =============================================================================

#[cfg(target_os = "macos")]
pub use macos::{
    create_desktop_overlay, destroy_desktop_overlay, get_screen_resolution, hide_native_icons,
    restore_native_icons,
};

/// macOS 平台 stub 模块（阶段 1 占位实现）
///
/// 这些函数签名与 Windows 版本完全一致，保证 app.rs 可以跨平台编译。
/// 后续阶段会逐步实现真正的 macOS 功能。
#[cfg(target_os = "macos")]
mod macos_stubs {
    use crate::config::app_config::{BoxLayoutData, GroupBy, SortMode, ViewMode};
    use crate::platform::types::DesktopIcon;
    use std::ffi::c_void;
    use std::path::PathBuf;

    /// 图标句柄（macOS 用 void 指针占位，对应 Windows HICON）
    pub type HICON = *mut c_void;

    /// 文件图标信息（签名与 Windows 一致）
    pub struct FileIconInfo {
        pub hicon: HICON,
        pub display_name: String,
        pub type_name: String,
    }

    /// 分类框文件项（签名与 Windows 一致）
    pub struct GridFileItem {
        pub file_path: String,
        pub display_name: String,
        pub icon_handle: HICON,
    }

    /// 分类框创建参数（签名与 Windows 一致）
    pub struct GridCreateParams {
        pub id: usize,
        pub title: String,
        pub x: i32,
        pub y: i32,
        pub width: i32,
        pub height: i32,
        pub items: Vec<GridFileItem>,
        pub view_mode: ViewMode,
        pub locked: bool,
        pub sort_mode: SortMode,
        pub sort_ascending: bool,
        pub group_by: GroupBy,
        pub config_id: uuid::Uuid,
    }

    /// 清理桌面注册表（macOS 无操作）
    pub fn cleanup_desktop_registry() -> anyhow::Result<()> {
        log::debug!("macOS: cleanup_desktop_registry (no-op)");
        Ok(())
    }

    /// 收集网格布局（macOS stub，返回类型与 Windows 一致）
    pub fn collect_grid_layout() -> Vec<BoxLayoutData> {
        log::debug!("macOS: collect_grid_layout (stub)");
        Vec::new()
    }

    /// 更新格子窗口标题（macOS stub）
    pub fn update_grid_window_title(_config_id: uuid::Uuid, _new_title: &str) {
        log::debug!("macOS: update_grid_window_title (stub)");
    }

    /// 创建分类框窗口（macOS stub）
    pub fn create_grid_window(_params: GridCreateParams) -> anyhow::Result<()> {
        log::warn!("macOS: create_grid_window 未实现");
        Ok(())
    }

    /// 销毁所有分类框窗口（macOS stub）
    pub fn destroy_all_grid_windows() {
        log::debug!("macOS: destroy_all_grid_windows (stub)");
    }

    /// 提取文件图标（macOS stub，返回类型与 Windows 一致）
    pub fn extract_file_icon(_path: &str) -> Option<FileIconInfo> {
        log::debug!("macOS: extract_file_icon (stub)");
        None
    }

    /// 获取鼠标所在显示器的工作区域（macOS 实现，返回物理像素坐标）
    pub fn get_cursor_monitor_work_area_phys() -> (i32, i32, i32, i32) {
        use cocoa::appkit::NSEvent;
        use cocoa::base::{id, nil};
        use cocoa::foundation::NSRect;
        use objc::{class, msg_send, sel, sel_impl};

        unsafe {
            let mouse_location: cocoa::foundation::NSPoint =
                msg_send![class!(NSEvent), mouseLocation];
            let screens: id = msg_send![class!(NSScreen), screens];
            let count: usize = msg_send![screens, count];

            for i in 0..count {
                let screen: id = msg_send![screens, objectAtIndex: i];
                let frame: NSRect = msg_send![screen, frame];
                if mouse_location.x >= frame.origin.x
                    && mouse_location.x < frame.origin.x + frame.size.width
                    && mouse_location.y >= frame.origin.y
                    && mouse_location.y < frame.origin.y + frame.size.height
                {
                    let visible_frame: NSRect = msg_send![screen, visibleFrame];
                    let scale: f64 = msg_send![screen, backingScaleFactor];
                    return (
                        (visible_frame.origin.x * scale) as i32,
                        (visible_frame.origin.y * scale) as i32,
                        (visible_frame.size.width * scale) as i32,
                        (visible_frame.size.height * scale) as i32,
                    );
                }
            }

            let main_screen: id = msg_send![screens, objectAtIndex: 0];
            let visible_frame: NSRect = msg_send![main_screen, visibleFrame];
            let scale: f64 = msg_send![main_screen, backingScaleFactor];
            (
                (visible_frame.origin.x * scale) as i32,
                (visible_frame.origin.y * scale) as i32,
                (visible_frame.size.width * scale) as i32,
                (visible_frame.size.height * scale) as i32,
            )
        }
    }

    /// 获取主显示器的工作区域（macOS 实现，返回物理像素坐标）
    pub fn get_primary_monitor_work_area_phys() -> (i32, i32, i32, i32) {
        use cocoa::base::nil;
        use cocoa::foundation::NSRect;
        use objc::{msg_send, sel, sel_impl};

        unsafe {
            let screens: cocoa::base::id = msg_send![class!(NSScreen), screens];
            let count: usize = msg_send![screens, count];
            if count == 0 {
                return (0, 0, 1920, 1080);
            }
            // 第一个 screen 是主显示器
            let main_screen: cocoa::base::id = msg_send![screens, objectAtIndex: 0];
            let visible_frame: NSRect = msg_send![main_screen, visibleFrame];
            let scale: f64 = msg_send![main_screen, backingScaleFactor];
            (
                (visible_frame.origin.x * scale) as i32,
                (visible_frame.origin.y * scale) as i32,
                (visible_frame.size.width * scale) as i32,
                (visible_frame.size.height * scale) as i32,
            )
        }
    }

    /// 获取分类框数量（macOS stub）
    pub fn get_grid_count() -> usize {
        0
    }

    /// 获取主显示器 DPI（macOS stub，Retina 返回 192，普通返回 96）
    pub fn get_primary_monitor_dpi() -> u32 {
        use cocoa::base::nil;
        use objc::{msg_send, sel, sel_impl};

        unsafe {
            let screen: cocoa::base::id = msg_send![class!(NSScreen), mainScreen];
            if screen == nil {
                return 96;
            }
            let scale: f64 = msg_send![screen, backingScaleFactor];
            (scale * 96.0) as u32
        }
    }

    /// 图标是否存在于分类框（macOS stub）
    pub fn icon_exists_for_box(_box_id: &str) -> bool {
        false
    }

    /// 原生图标是否隐藏（macOS stub）
    pub fn native_icons_hidden() -> bool {
        // macOS: 我们已成功隐藏了 PENDING_FEATURES.md，返回 true 避免循环重试
        true
    }

    /// 重新定位窗口（macOS stub）
    pub fn reposition_window_by_title(_title: &str, _x: i32, _y: i32) {
        log::debug!("macOS: reposition_window_by_title (stub)");
    }

    /// 解析图标读取路径（macOS stub，签名与 Windows 一致）
    pub fn resolve_icon_read_path(logical_path: &str) -> Option<PathBuf> {
        Some(PathBuf::from(logical_path))
    }

    /// 恢复所有隐藏图标（macOS stub）
    pub fn restore_all_hidden_icons() {
        log::debug!("macOS: restore_all_hidden_icons (stub)");
    }

    /// 设置桌面注册表（macOS 无操作）
    pub fn setup_desktop_registry() -> anyhow::Result<()> {
        log::debug!("macOS: setup_desktop_registry (no-op)");
        Ok(())
    }

    /// 获取待创建分类框信号（macOS stub，返回类型与 Windows 一致）
    pub fn take_pending_create_box() -> bool {
        false
    }

    /// 初始化系统托盘（macOS stub，无操作）。
    pub fn setup_system_tray() -> anyhow::Result<()> {
        Ok(())
    }

    /// 清理系统托盘（macOS stub，无操作）。
    pub fn cleanup_system_tray() {}

    /// 获取待处理的托盘动作（macOS stub）。
    pub fn take_pending_tray_action() -> Option<super::SystemTrayAction> {
        None
    }

    /// 注册托盘动作唤醒器（macOS stub，无操作）。
    pub fn register_tray_action_waker(_waker: impl Fn() + Send + Sync + 'static) {
        log::debug!("macOS: register_tray_action_waker (stub)");
    }

    /// 隐藏控制面板窗口（macOS stub）。
    pub fn hide_control_panel_window(_title: &str) -> bool {
        false
    }

    /// 显示控制面板窗口（macOS stub）。
    pub fn show_control_panel_window(_title: &str) -> bool {
        false
    }

    pub fn setup_global_hotkey(_hotkey_str: &str) -> anyhow::Result<()> {
        log::debug!("macOS: setup_global_hotkey (no-op)");
        Ok(())
    }

    pub fn unregister_global_hotkey() {
        log::debug!("macOS: unregister_global_hotkey (no-op)");
    }

    pub fn take_hotkey_toggle() -> bool {
        false
    }

    pub fn set_tray_auto_start_state(_enabled: bool) {
        log::debug!("macOS: set_tray_auto_start_state (no-op)");
    }
}

#[cfg(target_os = "macos")]
pub use macos_stubs::{
    cleanup_desktop_registry, cleanup_system_tray, collect_grid_layout, create_grid_window,
    destroy_all_grid_windows, extract_file_icon, get_cursor_monitor_work_area_phys, get_grid_count,
    get_primary_monitor_dpi, get_primary_monitor_work_area_phys, hide_control_panel_window,
    icon_exists_for_box, native_icons_hidden, reposition_and_resize_window,
    reposition_window_by_title, resolve_icon_read_path, restore_all_hidden_icons,
    set_tray_auto_start_state, setup_desktop_registry, setup_global_hotkey, setup_system_tray,
    show_control_panel_window, take_hotkey_toggle, take_pending_create_box,
    take_pending_tray_action, unregister_global_hotkey, update_grid_window_title, FileIconInfo,
    GridCreateParams, GridFileItem, HICON,
};

/// 桌面管理器 trait（跨平台）
pub trait DesktopManagerTrait {
    fn new() -> anyhow::Result<Self>
    where
        Self: Sized;
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;
    fn watch_desktop(&self) -> anyhow::Result<()>;
}
