//! 原生弹出菜单 — 使用 Win32 HMENU + TrackPopupMenu
//!
//! 使用 Windows 系统原生菜单，自动处理定位、子菜单展开、键盘导航、点击外部关闭等。
//! 菜单命令 ID 映射：
//! - 1: 删除
//! - 100-107: 排序维度 (Name, Type, ModifiedTime, Author, CreatedTime, Size, Tag, Title)
//! - 110: 排序递增
//! - 111: 排序递减
//! - 200-207: 分组维度
//! - 300-307: 查看模式

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::get_data;

use crate::config::app_config::{GroupBy, SortMode, ViewMode};

// ===== 命令 ID 常量 =====
const CMD_DELETE: usize = 1;

const CMD_SORT_BASE: usize = 100;
const CMD_SORT_ASC: usize = 110;
const CMD_SORT_DESC: usize = 111;

const CMD_GROUP_BASE: usize = 200;

const CMD_VIEW_BASE: usize = 300;

// ===== 公共 API =====

/// 显示原生弹出菜单
///
/// 在"更多"按钮下方显示菜单，返回后根据选择更新分类框状态。
/// 菜单打开期间分类框处于锁定状态。
pub fn show_menu_window(parent_hwnd: HWND) {
    let parent_data = unsafe { get_data(parent_hwnd) };
    let parent_data = match parent_data {
        Some(d) => d,
        None => return,
    };

    let dpi = unsafe { GetDpiForWindow(parent_hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };

    // 获取"更多"按钮的屏幕坐标
    let (btn_x, btn_y, btn_size) =
        super::grid_window::more_btn_rect(parent_data.base_window_w, parent_data.title_h, dpi);

    let mut parent_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(parent_hwnd, &mut parent_rect) };

    let screen_x = parent_rect.left + btn_x;
    let screen_y = parent_rect.top + btn_y + btn_size;

    // 锁定分类框
    let was_locked = parent_data.locked;
    parent_data.locked = true;
    parent_data.menu.open = true;
    render_layered_window(parent_hwnd);

    // 读取当前状态
    let current_sort = parent_data.sort_mode.clone();
    let current_asc = parent_data.sort_ascending;
    let current_group = parent_data.group_by.clone();
    let current_view = parent_data.view_mode.clone();

    // 构建菜单
    let cmd = build_and_show_menu(
        parent_hwnd,
        screen_x,
        screen_y,
        &current_sort,
        current_asc,
        &current_group,
        &current_view,
    );

    // 恢复状态
    parent_data.locked = was_locked;
    parent_data.menu.open = false;
    render_layered_window(parent_hwnd);

    // 处理命令
    if cmd > 0 {
        handle_menu_command(parent_data, cmd);
        render_layered_window(parent_hwnd);
    }

    log::info!("菜单命令: {}", cmd);
}

/// 菜单窗口是否正在显示（原生菜单模式下始终返回 false，由 TrackPopupMenu 阻塞控制）
pub fn is_menu_visible() -> bool {
    false
}

/// 关闭菜单（原生菜单由系统自动关闭，此函数为空操作）
pub fn hide_menu_window() {
    // TrackPopupMenu 是阻塞调用，返回时菜单已关闭
}

// ===== 内部实现 =====

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn build_and_show_menu(
    hwnd: HWND,
    x: i32,
    y: i32,
    current_sort: &SortMode,
    current_asc: bool,
    current_group: &GroupBy,
    current_view: &ViewMode,
) -> i32 {
    // 排序子菜单
    let sort_menu = unsafe { CreatePopupMenu() };
    let sort_items = [
        ("名称", SortMode::Name),
        ("类型", SortMode::Type),
        ("修改时间", SortMode::ModifiedTime),
        ("作者", SortMode::Author),
        ("创建时间", SortMode::CreatedTime),
        ("大小", SortMode::Size),
        ("标记", SortMode::Tag),
        ("标题", SortMode::Title),
    ];
    for (i, (label, mode)) in sort_items.iter().enumerate() {
        let flags = MF_STRING | if *mode == *current_sort { MF_CHECKED } else { MF_UNCHECKED };
        let text = to_wide(label);
        unsafe { AppendMenuW(sort_menu, flags, CMD_SORT_BASE + i, text.as_ptr()) };
    }
    // 分隔线
    unsafe { AppendMenuW(sort_menu, MF_SEPARATOR, 0, std::ptr::null()) };
    // 递增/递减（使用 radio check）
    let asc_flags = MF_STRING | MF_RADIOCHECK | if current_asc { MF_CHECKED } else { MF_UNCHECKED };
    let desc_flags = MF_STRING | MF_RADIOCHECK | if !current_asc { MF_CHECKED } else { MF_UNCHECKED };
    unsafe {
        AppendMenuW(sort_menu, asc_flags, CMD_SORT_ASC, to_wide("递增").as_ptr());
        AppendMenuW(sort_menu, desc_flags, CMD_SORT_DESC, to_wide("递减").as_ptr());
    }

    // 分组子菜单
    let group_menu = unsafe { CreatePopupMenu() };
    let group_items = [
        ("名称", GroupBy::Name),
        ("类型", GroupBy::Type),
        ("修改时间", GroupBy::ModifiedTime),
        ("作者", GroupBy::Author),
        ("创建时间", GroupBy::CreatedTime),
        ("大小", GroupBy::Size),
        ("标记", GroupBy::Tag),
        ("标题", GroupBy::Title),
    ];
    for (i, (label, mode)) in group_items.iter().enumerate() {
        let flags = MF_STRING | if *mode == *current_group { MF_CHECKED } else { MF_UNCHECKED };
        let text = to_wide(label);
        unsafe { AppendMenuW(group_menu, flags, CMD_GROUP_BASE + i, text.as_ptr()) };
    }

    // 查看子菜单
    let view_menu = unsafe { CreatePopupMenu() };
    let view_items = [
        ("超大图标", ViewMode::ExtraLarge),
        ("大图标", ViewMode::Large),
        ("中图标", ViewMode::Medium),
        ("小图标", ViewMode::Small),
        ("列表", ViewMode::List),
        ("详细信息", ViewMode::Detail),
        ("平铺", ViewMode::Tiles),
        ("内容", ViewMode::Content),
    ];
    for (i, (label, mode)) in view_items.iter().enumerate() {
        let flags = MF_STRING | if *mode == *current_view { MF_CHECKED } else { MF_UNCHECKED };
        let text = to_wide(label);
        unsafe { AppendMenuW(view_menu, flags, CMD_VIEW_BASE + i, text.as_ptr()) };
    }

    // 主菜单
    let main_menu = unsafe { CreatePopupMenu() };
    unsafe {
        AppendMenuW(main_menu, MF_STRING, CMD_DELETE, to_wide("删除").as_ptr());
        AppendMenuW(main_menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(
            main_menu,
            MF_POPUP | MF_STRING,
            sort_menu as usize,
            to_wide("排序").as_ptr(),
        );
        AppendMenuW(
            main_menu,
            MF_POPUP | MF_STRING,
            group_menu as usize,
            to_wide("分组").as_ptr(),
        );
        AppendMenuW(
            main_menu,
            MF_POPUP | MF_STRING,
            view_menu as usize,
            to_wide("查看").as_ptr(),
        );
    }

    // 显示菜单（阻塞直到用户选择或取消）
    let cmd = unsafe {
        TrackPopupMenu(
            main_menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_LEFTBUTTON,
            x,
            y,
            0,
            hwnd,
            std::ptr::null(),
        )
    };

    // 清理
    unsafe {
        DestroyMenu(main_menu);
        DestroyMenu(sort_menu);
        DestroyMenu(group_menu);
        DestroyMenu(view_menu);
    }

    cmd
}

fn handle_menu_command(pd: &mut super::grid_window::GridWindowData, cmd: i32) {
    let cmd = cmd as usize;

    if cmd == CMD_DELETE {
        log::info!("[预留] 分类框删除: {}", pd.title);
        return;
    }

    // 排序维度
    if cmd >= CMD_SORT_BASE && cmd < CMD_SORT_BASE + 8 {
        let modes = [
            SortMode::Name, SortMode::Type, SortMode::ModifiedTime, SortMode::Author,
            SortMode::CreatedTime, SortMode::Size, SortMode::Tag, SortMode::Title,
        ];
        let idx = cmd - CMD_SORT_BASE;
        if idx < modes.len() {
            pd.sort_mode = modes[idx].clone();
            log::info!("排序方式切换为: {:?}", pd.sort_mode);
        }
        return;
    }

    // 排序方向
    if cmd == CMD_SORT_ASC {
        pd.sort_ascending = true;
        log::info!("排序: 递增");
        return;
    }
    if cmd == CMD_SORT_DESC {
        pd.sort_ascending = false;
        log::info!("排序: 递减");
        return;
    }

    // 分组
    if cmd >= CMD_GROUP_BASE && cmd < CMD_GROUP_BASE + 8 {
        let modes = [
            GroupBy::Name, GroupBy::Type, GroupBy::ModifiedTime, GroupBy::Author,
            GroupBy::CreatedTime, GroupBy::Size, GroupBy::Tag, GroupBy::Title,
        ];
        let idx = cmd - CMD_GROUP_BASE;
        if idx < modes.len() {
            pd.group_by = modes[idx].clone();
            log::info!("分组方式切换为: {:?}", pd.group_by);
        }
        return;
    }

    // 查看
    if cmd >= CMD_VIEW_BASE && cmd < CMD_VIEW_BASE + 8 {
        let modes = [
            ViewMode::ExtraLarge, ViewMode::Large, ViewMode::Medium, ViewMode::Small,
            ViewMode::List, ViewMode::Detail, ViewMode::Tiles, ViewMode::Content,
        ];
        let idx = cmd - CMD_VIEW_BASE;
        if idx < modes.len() {
            pd.view_mode = modes[idx].clone();
            log::info!("查看模式切换为: {:?}", pd.view_mode);
        }
        return;
    }
}
