//! 深色主题 Owner-Draw 弹出菜单
//!
//! 使用 Win32 HMENU + MF_OWNERDRAW 实现深色主题原生菜单。
//! 保留原生菜单行为（子菜单展开、键盘导航、点击外部关闭），
//! 通过 WM_MEASUREITEM / WM_DRAWITEM 自定义绘制深色背景 + 浅色文字。

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::get_data;

use crate::config::app_config::{GroupBy, SortMode, ViewMode};

// ===== 命令 ID 常量 =====
const CMD_DELETE: u32 = 1;
const CMD_SORT_BASE: u32 = 100;
const CMD_SORT_ASC: u32 = 110;
const CMD_SORT_DESC: u32 = 111;
const CMD_GROUP_BASE: u32 = 200;
const CMD_VIEW_BASE: u32 = 300;

// ===== 公共 API =====

/// 显示深色主题弹出菜单
pub fn show_menu_window(parent_hwnd: HWND) {
    let parent_data = unsafe { get_data(parent_hwnd) };
    let parent_data = match parent_data {
        Some(d) => d,
        None => return,
    };

    let dpi = unsafe { GetDpiForWindow(parent_hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };

    let (btn_x, btn_y, btn_size) =
        super::grid_window::more_btn_rect(parent_data.base_window_w, parent_data.title_h, dpi);

    let mut parent_rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(parent_hwnd, &mut parent_rect) };

    let screen_x = parent_rect.left + btn_x;
    let screen_y = parent_rect.top + btn_y + btn_size;

    // 锁定分类框
    let was_locked = parent_data.locked;
    parent_data.locked = true;
    parent_data.menu.open = true;
    render_layered_window(parent_hwnd);

    let current_sort = parent_data.sort_mode.clone();
    let current_asc = parent_data.sort_ascending;
    let current_group = parent_data.group_by.clone();
    let current_view = parent_data.view_mode.clone();

    // 存储 owner-draw 数据（生命周期覆盖 TrackPopupMenu 调用）
    let mut draw_data: Vec<Box<MenuDrawItemData>> = Vec::new();

    let cmd = build_and_show_menu(
        parent_hwnd,
        screen_x,
        screen_y,
        &current_sort,
        current_asc,
        &current_group,
        &current_view,
        &mut draw_data,
    );

    // 恢复状态
    parent_data.locked = was_locked;
    parent_data.menu.open = false;
    render_layered_window(parent_hwnd);

    if cmd > 0 {
        handle_menu_command(parent_data, cmd);
        render_layered_window(parent_hwnd);
    }

    // draw_data 在此 drop，释放所有 owner-draw 数据
}

pub fn is_menu_visible() -> bool { false }
pub fn hide_menu_window() {}

// ===== 内部实现 =====

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 添加一个 owner-draw 菜单项
fn append_owner_draw_item(
    menu: HMENU,
    id: u32,
    label: &str,
    is_separator: bool,
    is_checked: bool,
    is_radio: bool,
    sub_menu: HMENU,
    draw_data: &mut Vec<Box<MenuDrawItemData>>,
) {
    let data = Box::new(MenuDrawItemData {
        label: to_wide(label),
        is_separator,
        is_checked,
        is_radio,
    });
    let data_ptr = Box::into_raw(data);
    // 同时保存到 vec 以便自动清理
    draw_data.push(unsafe { Box::from_raw(data_ptr) });

    let mii = MENUITEMINFOW {
        cb_size: std::mem::size_of::<MENUITEMINFOW>() as u32,
        f_mask: MIIM_FTYPE | MIIM_ID | MIIM_DATA | if !sub_menu.is_null() { MIIM_SUBMENU } else { 0 },
        f_type: if is_separator { MFT_SEPARATOR | MFT_OWNERDRAW } else { MFT_OWNERDRAW },
        f_state: if is_checked { MFS_CHECKED } else { 0 },
        w_id: id,
        h_submenu: sub_menu,
        hbmp_checked: std::ptr::null_mut(),
        hbmp_unchecked: std::ptr::null_mut(),
        dw_item_data: data_ptr as usize,
        dw_type_data: 0,
        cch: 0,
        hbmp_item: std::ptr::null_mut(),
    };

    unsafe {
        InsertMenuItemW(menu, u32::MAX, 1, &mii);
    }
}

fn build_and_show_menu(
    hwnd: HWND,
    x: i32,
    y: i32,
    current_sort: &SortMode,
    current_asc: bool,
    current_group: &GroupBy,
    current_view: &ViewMode,
    draw_data: &mut Vec<Box<MenuDrawItemData>>,
) -> i32 {
    // 排序子菜单
    let sort_menu = unsafe { CreatePopupMenu() };
    let sort_items = [
        ("名称", SortMode::Name), ("类型", SortMode::Type),
        ("修改时间", SortMode::ModifiedTime), ("作者", SortMode::Author),
        ("创建时间", SortMode::CreatedTime), ("大小", SortMode::Size),
        ("标记", SortMode::Tag), ("标题", SortMode::Title),
    ];
    for (i, (label, mode)) in sort_items.iter().enumerate() {
        append_owner_draw_item(
            sort_menu, CMD_SORT_BASE + i as u32, label, false,
            *mode == *current_sort, false, std::ptr::null_mut(), draw_data,
        );
    }
    // 分隔线
    append_owner_draw_item(
        sort_menu, 0, "", true, false, false, std::ptr::null_mut(), draw_data,
    );
    // 递增/递减
    append_owner_draw_item(
        sort_menu, CMD_SORT_ASC, "递增", false,
        current_asc, true, std::ptr::null_mut(), draw_data,
    );
    append_owner_draw_item(
        sort_menu, CMD_SORT_DESC, "递减", false,
        !current_asc, true, std::ptr::null_mut(), draw_data,
    );

    // 分组子菜单
    let group_menu = unsafe { CreatePopupMenu() };
    let group_items = [
        ("名称", GroupBy::Name), ("类型", GroupBy::Type),
        ("修改时间", GroupBy::ModifiedTime), ("作者", GroupBy::Author),
        ("创建时间", GroupBy::CreatedTime), ("大小", GroupBy::Size),
        ("标记", GroupBy::Tag), ("标题", GroupBy::Title),
    ];
    for (i, (label, mode)) in group_items.iter().enumerate() {
        append_owner_draw_item(
            group_menu, CMD_GROUP_BASE + i as u32, label, false,
            *mode == *current_group, false, std::ptr::null_mut(), draw_data,
        );
    }

    // 查看子菜单
    let view_menu = unsafe { CreatePopupMenu() };
    let view_items = [
        ("超大图标", ViewMode::ExtraLarge), ("大图标", ViewMode::Large),
        ("中图标", ViewMode::Medium), ("小图标", ViewMode::Small),
        ("列表", ViewMode::List), ("详细信息", ViewMode::Detail),
        ("平铺", ViewMode::Tiles), ("内容", ViewMode::Content),
    ];
    for (i, (label, mode)) in view_items.iter().enumerate() {
        append_owner_draw_item(
            view_menu, CMD_VIEW_BASE + i as u32, label, false,
            *mode == *current_view, false, std::ptr::null_mut(), draw_data,
        );
    }

    // 主菜单
    let main_menu = unsafe { CreatePopupMenu() };
    append_owner_draw_item(
        main_menu, CMD_DELETE, "删除", false,
        false, false, std::ptr::null_mut(), draw_data,
    );
    append_owner_draw_item(
        main_menu, 0, "", true, false, false, std::ptr::null_mut(), draw_data,
    );
    append_owner_draw_item(
        main_menu, 0, "排序", false,
        false, false, sort_menu, draw_data,
    );
    append_owner_draw_item(
        main_menu, 0, "分组", false,
        false, false, group_menu, draw_data,
    );
    append_owner_draw_item(
        main_menu, 0, "查看", false,
        false, false, view_menu, draw_data,
    );

    let cmd = unsafe {
        TrackPopupMenu(
            main_menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_LEFTBUTTON,
            x, y, 0, hwnd, std::ptr::null(),
        )
    };

    unsafe {
        DestroyMenu(main_menu);
        DestroyMenu(sort_menu);
        DestroyMenu(group_menu);
        DestroyMenu(view_menu);
    }

    cmd
}

fn handle_menu_command(pd: &mut super::grid_window::GridWindowData, cmd: i32) {
    let cmd = cmd as u32;
    if cmd == CMD_DELETE {
        log::info!("[预留] 分类框删除: {}", pd.title);
    } else if cmd >= CMD_SORT_BASE && cmd < CMD_SORT_BASE + 8 {
        let modes = [
            SortMode::Name, SortMode::Type, SortMode::ModifiedTime, SortMode::Author,
            SortMode::CreatedTime, SortMode::Size, SortMode::Tag, SortMode::Title,
        ];
        pd.sort_mode = modes[(cmd - CMD_SORT_BASE) as usize].clone();
        log::info!("排序: {:?}", pd.sort_mode);
    } else if cmd == CMD_SORT_ASC {
        pd.sort_ascending = true;
        log::info!("排序: 递增");
    } else if cmd == CMD_SORT_DESC {
        pd.sort_ascending = false;
        log::info!("排序: 递减");
    } else if cmd >= CMD_GROUP_BASE && cmd < CMD_GROUP_BASE + 8 {
        let modes = [
            GroupBy::Name, GroupBy::Type, GroupBy::ModifiedTime, GroupBy::Author,
            GroupBy::CreatedTime, GroupBy::Size, GroupBy::Tag, GroupBy::Title,
        ];
        pd.group_by = modes[(cmd - CMD_GROUP_BASE) as usize].clone();
        log::info!("分组: {:?}", pd.group_by);
    } else if cmd >= CMD_VIEW_BASE && cmd < CMD_VIEW_BASE + 8 {
        let modes = [
            ViewMode::ExtraLarge, ViewMode::Large, ViewMode::Medium, ViewMode::Small,
            ViewMode::List, ViewMode::Detail, ViewMode::Tiles, ViewMode::Content,
        ];
        pd.view_mode = modes[(cmd - CMD_VIEW_BASE) as usize].clone();
        log::info!("查看: {:?}", pd.view_mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_wide_null_terminated() {
        let wide = to_wide("测试");
        assert_eq!(*wide.last().unwrap(), 0u16);
        assert_eq!(wide.len(), 3); // 2 chars + null
    }

    #[test]
    fn test_to_wide_empty() {
        let wide = to_wide("");
        assert_eq!(wide.len(), 1); // just null terminator
        assert_eq!(wide[0], 0u16);
    }

    #[test]
    fn test_to_wide_ascii() {
        let wide = to_wide("AB");
        assert_eq!(wide, vec![65u16, 66u16, 0u16]);
    }

    #[test]
    fn test_command_id_ranges_no_overlap() {
        // 验证命令 ID 范围不重叠
        assert!(CMD_DELETE < CMD_SORT_BASE);
        assert!(CMD_SORT_BASE + 8 <= CMD_SORT_ASC);
        assert!(CMD_SORT_ASC < CMD_SORT_DESC);
        assert!(CMD_SORT_DESC < CMD_GROUP_BASE);
        assert!(CMD_GROUP_BASE + 8 <= CMD_VIEW_BASE);
        assert!(CMD_VIEW_BASE + 8 < 1000); // 安全上限
    }

    #[test]
    fn test_command_id_coverage() {
        // 排序维度 8 个
        for i in 0..8u32 {
            let cmd = CMD_SORT_BASE + i;
            assert!(cmd >= CMD_SORT_BASE && cmd < CMD_SORT_BASE + 8);
        }
        // 分组维度 8 个
        for i in 0..8u32 {
            let cmd = CMD_GROUP_BASE + i;
            assert!(cmd >= CMD_GROUP_BASE && cmd < CMD_GROUP_BASE + 8);
        }
        // 查看模式 8 个
        for i in 0..8u32 {
            let cmd = CMD_VIEW_BASE + i;
            assert!(cmd >= CMD_VIEW_BASE && cmd < CMD_VIEW_BASE + 8);
        }
    }

    #[test]
    fn test_handle_menu_command_sort() {
        let mut data = super::super::grid_window::GridWindowData::test_default();
        // 排序维度切换
        handle_menu_command(&mut data, CMD_SORT_BASE as i32); // Name
        assert_eq!(data.sort_mode, SortMode::Name);
        handle_menu_command(&mut data, (CMD_SORT_BASE + 1) as i32); // Type
        assert_eq!(data.sort_mode, SortMode::Type);
        handle_menu_command(&mut data, (CMD_SORT_BASE + 5) as i32); // Size
        assert_eq!(data.sort_mode, SortMode::Size);

        // 排序方向
        handle_menu_command(&mut data, CMD_SORT_ASC as i32);
        assert!(data.sort_ascending);
        handle_menu_command(&mut data, CMD_SORT_DESC as i32);
        assert!(!data.sort_ascending);
    }

    #[test]
    fn test_handle_menu_command_group() {
        let mut data = super::super::grid_window::GridWindowData::test_default();
        handle_menu_command(&mut data, CMD_GROUP_BASE as i32); // Name
        assert_eq!(data.group_by, GroupBy::Name);
        handle_menu_command(&mut data, (CMD_GROUP_BASE + 3) as i32); // Author
        assert_eq!(data.group_by, GroupBy::Author);
    }

    #[test]
    fn test_handle_menu_command_view() {
        let mut data = super::super::grid_window::GridWindowData::test_default();
        handle_menu_command(&mut data, CMD_VIEW_BASE as i32); // ExtraLarge
        assert_eq!(data.view_mode, ViewMode::ExtraLarge);
        handle_menu_command(&mut data, (CMD_VIEW_BASE + 5) as i32); // Detail
        assert_eq!(data.view_mode, ViewMode::Detail);
    }

    #[test]
    fn test_handle_menu_command_unknown_id() {
        let mut data = super::super::grid_window::GridWindowData::test_default();
        let original_sort = data.sort_mode.clone();
        let original_group = data.group_by.clone();
        let original_view = data.view_mode.clone();

        // 未知命令不应改变状态
        handle_menu_command(&mut data, 999);
        assert_eq!(data.sort_mode, original_sort);
        assert_eq!(data.group_by, original_group);
        assert_eq!(data.view_mode, original_view);
    }
}
