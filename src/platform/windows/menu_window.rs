//! 自定义分层窗口菜单

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! 使用 WS_POPUP + WS_EX_LAYERED + D2D/DirectWrite 实现深色主题菜单。
//! 替代原生 TrackPopupMenu，支持 per-pixel alpha 透明度、DirectWrite 清晰文字、
//! 无系统阴影、完美 DPI 适配。
//!
//! 位置计算与原生菜单一致：主菜单在按钮下方对齐，子菜单在父项右侧展开，
//! 屏幕边界自动翻转。

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::get_data;

use crate::config::app_config::{GroupBy, SortMode, ViewMode};

use std::ffi::c_void;

use windows::Win32::Graphics::Direct2D::{
    Common::D2D1_PIXEL_FORMAT, D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

// ===== 命令 ID 常量 =====
const CMD_DELETE: u32 = 1;
const CMD_SORT_BASE: u32 = 100;
const CMD_SORT_ASC: u32 = 110;
const CMD_SORT_DESC: u32 = 111;
const CMD_GROUP_BASE: u32 = 200;
const CMD_VIEW_BASE: u32 = 300;

// ===== 菜单样式常量 =====
const MENU_PADDING_X: i32 = 12;
const MENU_PADDING_Y: i32 = 6;
const MENU_ITEM_HEIGHT: i32 = 34;
const MENU_SEPARATOR_HEIGHT: i32 = 9;
const MENU_CHECK_WIDTH: i32 = 28;
const MENU_ARROW_WIDTH: i32 = 24;
const MENU_FONT_SIZE: i32 = 14;
const MENU_CORNER_RADIUS: f32 = 8.0;

// ===== 窗口类名 =====
static MENU_CLASS_NAME: &[u16] = &[
    68, 101, 115, 107, 116, 111, 112, 79, 114, 103, 77, 101, 110, 117, 0,
]; // "DesktopOrgMenu"

// ===== 公共 API =====

/// 显示深色主题弹出菜单（阻塞式，与 TrackPopupMenu 行为一致）
pub fn show_menu_window(parent_hwnd: HWND) {
    // 如果已有菜单打开，关闭它并 PostMessage 延迟打开新菜单
    // （旧菜单的模态循环退出后，主消息循环会处理 post 的消息）
    {
        let open_hwnd = OPEN_MENU_HWND.lock().unwrap();
        if !open_hwnd.0.is_null() {
            let existing_hwnd = open_hwnd.0;
            drop(open_hwnd);
            let data_ptr =
                unsafe { GetWindowLongPtrW(existing_hwnd, GWLP_USERDATA) } as *mut MenuWindowData;
            if !data_ptr.is_null() {
                unsafe {
                    (*data_ptr).is_closing = true;
                }
            }
            // 延迟打开新菜单（等旧菜单模态循环退出后由主消息循环处理）
            unsafe { PostMessageW(parent_hwnd, WM_USER_OPEN_MENU, 0, 0) };
            return;
        }
    }

    let parent_data = unsafe { get_data(parent_hwnd) };
    let parent_data = match parent_data {
        Some(d) => d,
        None => return,
    };

    let dpi = unsafe { GetDpiForWindow(parent_hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };

    // 获取当前窗口实际尺寸（而非 base_window_w，修复缩放后位置偏移）
    let mut parent_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(parent_hwnd, &mut parent_rect) };
    let win_w = parent_rect.right - parent_rect.left;

    let (btn_x, btn_y, btn_size) =
        super::grid_layout::more_btn_rect(win_w, parent_data.title_h, dpi);

    // 菜单位置：按钮下方，左边缘对齐
    let menu_x = parent_rect.left + btn_x;
    let menu_y = parent_rect.top + btn_y + btn_size;

    // 锁定分类框
    let was_locked = parent_data.locked;
    parent_data.locked = true;
    parent_data.menu.open = true;
    render_layered_window(parent_hwnd);

    let current_sort = parent_data.sort_mode.clone();
    let current_asc = parent_data.sort_ascending;
    let current_group = parent_data.group_by.clone();
    let current_view = parent_data.view_mode.clone();

    // 构建菜单项
    let items = build_menu_items(&current_sort, current_asc, &current_group, &current_view);

    // 计算菜单尺寸
    let (menu_w, menu_h) = calculate_menu_size(&items, dpi);

    // 屏幕边界检测（使用父窗口所在显示器）
    let (final_x, final_y) = adjust_menu_position(
        menu_x,
        menu_y,
        menu_w,
        menu_h,
        parent_rect.left + btn_x + btn_size,
    );

    // 创建并显示菜单窗口
    let cmd = run_menu_loop(parent_hwnd, final_x, final_y, menu_w, menu_h, items, dpi);

    // 恢复状态
    parent_data.locked = was_locked;
    parent_data.menu.open = false;

    if cmd > 0 {
        let should_destroy = handle_menu_command(parent_data, cmd);
        if should_destroy {
            // 1. 先读 config_id（窗口销毁前）
            let config_id = parent_data.config_id;
            let title = parent_data.title.clone();
            // 2. 销毁窗口
            super::grid_window::destroy_grid_window(parent_hwnd);
            // parent_data 已失效，不再访问
            // 3. 销毁后重绘所有剩余窗口（修复 DestroyWindow 导致其他分层窗口失效）
            super::grid_window::rerender_all_grid_windows();
            // 4. 按 UUID 精确删除配置
            match crate::config::AppConfig::load() {
                Ok(mut config) => {
                    config.layout.boxes.retain(|b| b.id != config_id);
                    if let Err(e) = config.save() {
                        log::warn!("删除后保存配置失败: {}", e);
                    } else {
                        log::info!("已删除 '{}', 剩余 {} 个", title, config.layout.boxes.len());
                    }
                }
                Err(e) => log::warn!("加载配置失败: {}", e),
            }
        } else {
            render_layered_window(parent_hwnd);
        }
    } else {
        render_layered_window(parent_hwnd);
    }
}

#[allow(dead_code)]
pub fn is_menu_visible() -> bool {
    false
}
#[allow(dead_code)]
pub fn hide_menu_window() {}

// ===== 菜单项构建 =====

fn build_menu_items(
    current_sort: &SortMode,
    current_asc: bool,
    current_group: &GroupBy,
    current_view: &ViewMode,
) -> Vec<MenuItem> {
    // 排序子菜单
    let sort_items_data = [
        ("名称", SortMode::Name),
        ("类型", SortMode::Type),
        ("修改时间", SortMode::ModifiedTime),
        ("作者", SortMode::Author),
        ("创建时间", SortMode::CreatedTime),
        ("大小", SortMode::Size),
        ("标记", SortMode::Tag),
        ("标题", SortMode::Title),
    ];
    let mut sort_children: Vec<MenuItem> = sort_items_data
        .iter()
        .enumerate()
        .map(|(i, (label, mode))| {
            MenuItem::new(label, CMD_SORT_BASE + i as u32).checked(*mode == *current_sort)
        })
        .collect();
    sort_children.push(MenuItem::separator());
    sort_children.push(
        MenuItem::new("递增", CMD_SORT_ASC)
            .checked(current_asc)
            .radio(true),
    );
    sort_children.push(
        MenuItem::new("递减", CMD_SORT_DESC)
            .checked(!current_asc)
            .radio(true),
    );

    // 分组子菜单
    let group_items_data = [
        ("名称", GroupBy::Name),
        ("类型", GroupBy::Type),
        ("修改时间", GroupBy::ModifiedTime),
        ("作者", GroupBy::Author),
        ("创建时间", GroupBy::CreatedTime),
        ("大小", GroupBy::Size),
        ("标记", GroupBy::Tag),
        ("标题", GroupBy::Title),
    ];
    let group_children: Vec<MenuItem> = group_items_data
        .iter()
        .enumerate()
        .map(|(i, (label, mode))| {
            MenuItem::new(label, CMD_GROUP_BASE + i as u32).checked(*mode == *current_group)
        })
        .collect();

    // 查看子菜单（全部 disabled，暂未实现）
    let view_items_data = [
        ("超大图标", ViewMode::ExtraLarge),
        ("大图标", ViewMode::Large),
        ("中图标", ViewMode::Medium),
        ("小图标", ViewMode::Small),
        ("列表", ViewMode::List),
        ("详细信息", ViewMode::Detail),
        ("平铺", ViewMode::Tiles),
        ("内容", ViewMode::Content),
    ];
    let view_children: Vec<MenuItem> = view_items_data
        .iter()
        .enumerate()
        .map(|(i, (label, mode))| {
            MenuItem::new(label, CMD_VIEW_BASE + i as u32).checked(*mode == *current_view)
        })
        .collect();

    // 主菜单
    vec![
        MenuItem::new("删除", CMD_DELETE),
        MenuItem::separator(),
        MenuItem::with_children("排序", sort_children),
        MenuItem::with_children("分组", group_children),
        MenuItem::with_children("查看", view_children),
    ]
}

// ===== 尺寸计算 =====

fn calculate_menu_size(items: &[MenuItem], dpi: u32) -> (i32, i32) {
    let padding_x = scale_for_dpi(MENU_PADDING_X, dpi);
    let padding_y = scale_for_dpi(MENU_PADDING_Y, dpi);
    let item_h = scale_for_dpi(MENU_ITEM_HEIGHT, dpi);
    let sep_h = scale_for_dpi(MENU_SEPARATOR_HEIGHT, dpi);
    let check_w = scale_for_dpi(MENU_CHECK_WIDTH, dpi);
    let arrow_w = scale_for_dpi(MENU_ARROW_WIDTH, dpi);
    let font_size = scale_for_dpi(MENU_FONT_SIZE, dpi);

    // 估算最大文字宽度（使用简单字符宽度估算，中文约 font_size 像素/字）
    let mut max_text_w: i32 = 0;
    for item in items {
        if !item.is_separator {
            let char_count = item.label.chars().count() as i32;
            // 中文字符约 font_size 宽，ASCII 约 font_size/2 宽
            let estimated_w = char_count * font_size;
            if estimated_w > max_text_w {
                max_text_w = estimated_w;
            }
        }
    }

    let content_w = check_w + max_text_w + arrow_w + padding_x * 2;
    let min_w = scale_for_dpi(180, dpi);
    let menu_w = content_w.max(min_w);

    let mut menu_h = padding_y * 2;
    for item in items {
        if item.is_separator {
            menu_h += sep_h;
        } else {
            menu_h += item_h;
        }
    }

    (menu_w, menu_h)
}

// ===== 位置计算（屏幕边界检测）=====

fn adjust_menu_position(x: i32, y: i32, w: i32, h: i32, btn_right_x: i32) -> (i32, i32) {
    // 获取显示器工作区域
    let pt = POINT { x, y };
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rcWork: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dwFlags: 0,
    };
    unsafe { GetMonitorInfoW(monitor, &mut mi) };

    let mut final_x = x;
    let mut final_y = y;

    // 右边界翻转
    if final_x + w > mi.rcMonitor.right {
        final_x = btn_right_x - w;
    }
    // 左边界保护
    if final_x < mi.rcMonitor.left {
        final_x = mi.rcMonitor.left;
    }

    // 下边界翻转
    if final_y + h > mi.rcMonitor.bottom {
        final_y = y - h - scale_for_dpi(26, 96); // 翻转到按钮上方
    }
    // 上边界保护
    if final_y < mi.rcMonitor.top {
        final_y = mi.rcMonitor.top;
    }

    (final_x, final_y)
}

/// 子菜单位置计算
fn adjust_submenu_position(
    parent_right: i32,
    parent_top: i32,
    item_top: i32,
    sub_w: i32,
    sub_h: i32,
    parent_left: i32,
) -> (i32, i32) {
    let pt = POINT {
        x: parent_right,
        y: item_top,
    };
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rcWork: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dwFlags: 0,
    };
    unsafe { GetMonitorInfoW(monitor, &mut mi) };

    let mut x = parent_right;
    let mut y = parent_top + item_top;

    // 右边界翻转：子菜单显示在父菜单左侧
    if x + sub_w > mi.rcMonitor.right {
        x = parent_left - sub_w;
    }
    // 下边界保护
    if y + sub_h > mi.rcMonitor.bottom {
        y = mi.rcMonitor.bottom - sub_h;
    }
    if y < mi.rcMonitor.top {
        y = mi.rcMonitor.top;
    }

    (x, y)
}

// ===== 菜单窗口数据 =====

struct MenuWindowData {
    items: Vec<MenuItem>,
    item_rects: Vec<(i32, i32, i32, i32)>, // (top, bottom, left, right) 每个 item 的矩形
    hovered_index: i32,
    menu_width: i32,
    menu_height: i32,
    dpi: u32,
    parent_hwnd: HWND,    // 分类框窗口
    main_menu_hwnd: HWND, // 主菜单窗口（子菜单用，用于传播命令）
    submenu_hwnd: HWND,   // 当前打开的子菜单窗口
    submenu_parent_index: i32,
    is_submenu: bool,             // 是否为子菜单
    ignore_first_lbuttonup: bool, // 跳过按钮原始点击的 WM_LBUTTONUP
    result_cmd: i32,
    is_closing: bool,
    // D2D/DW 设备缓存
    d2d_factory: Option<ID2D1Factory>,
    dw_factory: Option<IDWriteFactory>,
    wic_factory: Option<IWICImagingFactory>,
    d2d_target: Option<ID2D1DCRenderTarget>,
}

// ===== 窗口类注册 =====

static MENU_CLASS_REGISTERED: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

/// 线程安全的 HWND 包装（用于全局静态变量）
struct SendHwnd(HWND);
unsafe impl Send for SendHwnd {}

/// 当前打开的菜单窗口句柄（全局互斥，确保同时只有一个菜单打开）
static OPEN_MENU_HWND: std::sync::Mutex<SendHwnd> =
    std::sync::Mutex::new(SendHwnd(std::ptr::null_mut()));

fn ensure_menu_class_registered() -> HINSTANCE {
    let mut registered = MENU_CLASS_REGISTERED.lock().unwrap();
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

    if !*registered {
        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfn_wnd_proc: Some(menu_wnd_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: hinstance,
            h_icon: std::ptr::null_mut(),
            h_cursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as *const u16) },
            hbr_background: std::ptr::null_mut(),
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: MENU_CLASS_NAME.as_ptr(),
            h_icon_sm: std::ptr::null_mut(),
        };
        unsafe { RegisterClassExW(&wc) };
        *registered = true;
    }

    hinstance
}

// ===== 菜单消息循环（阻塞式）=====

fn run_menu_loop(
    parent_hwnd: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    items: Vec<MenuItem>,
    dpi: u32,
) -> i32 {
    let hinstance = ensure_menu_class_registered();

    // 计算 item_rects
    let item_rects = calculate_item_rects(&items, dpi);

    // 创建窗口
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            MENU_CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            w,
            h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    };

    if hwnd.is_null() {
        log::error!("创建菜单窗口失败");
        return 0;
    }

    // 初始化 D2D 设备
    let d2d_factory =
        unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok() };
    let dw_factory =
        unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED).ok() };
    let wic_factory = unsafe {
        CoCreateInstance::<_, IWICImagingFactory>(
            &CLSID_WICImagingFactory,
            None,
            CLSCTX_INPROC_SERVER,
        )
        .ok()
    };
    let d2d_target = if let Some(ref factory) = d2d_factory {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode:
                    windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 0.0,
            dpiY: 0.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        unsafe { factory.CreateDCRenderTarget(&props).ok() }
    } else {
        None
    };

    // 存储窗口数据
    let data = Box::new(MenuWindowData {
        items,
        item_rects,
        hovered_index: -1,
        menu_width: w,
        menu_height: h,
        dpi,
        parent_hwnd,
        main_menu_hwnd: hwnd,
        submenu_hwnd: std::ptr::null_mut(),
        submenu_parent_index: -1,
        is_submenu: false,
        ignore_first_lbuttonup: true,
        result_cmd: 0,
        is_closing: false,
        d2d_factory,
        dw_factory,
        wic_factory,
        d2d_target,
    });

    let data_ptr = Box::into_raw(data);
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize) };

    // 初始渲染
    render_menu(hwnd);

    // 注册当前打开的菜单（用于互斥关闭）
    {
        let mut open_hwnd = OPEN_MENU_HWND.lock().unwrap();
        open_hwnd.0 = hwnd;
    }

    // 设置前台窗口（使菜单获得焦点，点击外部时触发 WM_ACTIVATE 关闭）
    unsafe { SetForegroundWindow(hwnd) };

    // 模态消息循环
    let mut msg = MSG {
        hwnd: std::ptr::null_mut(),
        message: 0,
        wParam: 0,
        lParam: 0,
        time: 0,
        pt: POINT { x: 0, y: 0 },
    };
    loop {
        let data_ref = unsafe { &*(data_ptr as *const MenuWindowData) };
        if data_ref.is_closing || data_ref.result_cmd != 0 {
            break;
        }

        if unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let cmd = unsafe { (*data_ptr).result_cmd };

    // 清理全局菜单句柄
    {
        let mut open_hwnd = OPEN_MENU_HWND.lock().unwrap();
        if open_hwnd.0 == hwnd {
            open_hwnd.0 = std::ptr::null_mut();
        }
    }

    // 清理
    if !unsafe { (*data_ptr).submenu_hwnd }.is_null() {
        unsafe { DestroyWindow((*data_ptr).submenu_hwnd) };
    }
    unsafe { DestroyWindow(hwnd) };

    cmd
}

// ===== 菜单窗口消息处理 =====

unsafe extern "system" fn menu_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuWindowData;
            if data_ptr.is_null() {
                return 0;
            }
            let data = unsafe { &mut *data_ptr };

            let mx = get_x_lparam(lparam);
            let my = get_y_lparam(lparam);

            // 查找 hover 的 item
            let new_hover = hit_test_item(data, mx, my);
            if new_hover != data.hovered_index {
                data.hovered_index = new_hover;
                render_menu(hwnd);

                // 处理子菜单
                if new_hover >= 0 && (new_hover as usize) < data.items.len() {
                    let item = &data.items[new_hover as usize];
                    if item.has_submenu() {
                        open_submenu(hwnd, data, new_hover);
                    } else {
                        close_submenu(data);
                    }
                } else {
                    close_submenu(data);
                }
            }

            // 注册 WM_MOUSELEAVE 跟踪
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe { TrackMouseEvent(&mut tme) };

            0
        }

        WM_LBUTTONUP => {
            let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuWindowData;
            if data_ptr.is_null() {
                return 0;
            }
            let data = unsafe { &mut *data_ptr };

            let mx = get_x_lparam(lparam);
            let my = get_y_lparam(lparam);

            // 只处理菜单窗口内部的点击，外部点击由 WM_ACTIVATE 处理
            if mx < 0 || mx >= data.menu_width || my < 0 || my >= data.menu_height {
                return 0;
            }

            let idx = hit_test_item(data, mx, my);
            if idx >= 0 && (idx as usize) < data.items.len() {
                let item = &data.items[idx as usize];
                if !item.is_separator && !item.is_disabled && !item.has_submenu() && item.id > 0 {
                    if data.is_submenu {
                        // 子菜单点击：传播命令到主菜单，关闭整个菜单
                        let main_hwnd = data.main_menu_hwnd;
                        if !main_hwnd.is_null() {
                            let main_ptr = unsafe { GetWindowLongPtrW(main_hwnd, GWLP_USERDATA) }
                                as *mut MenuWindowData;
                            if !main_ptr.is_null() {
                                unsafe {
                                    (*main_ptr).result_cmd = item.id as i32;
                                    (*main_ptr).is_closing = true;
                                }
                            }
                        }
                    } else {
                        // 主菜单点击：直接设置命令
                        data.result_cmd = item.id as i32;
                    }
                }
            }
            0
        }

        WM_MOUSELEAVE => {
            let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuWindowData;
            if data_ptr.is_null() {
                return 0;
            }
            let data = unsafe { &mut *data_ptr };

            // 检查鼠标是否在子菜单上
            if !data.submenu_hwnd.is_null() {
                let mut pt = POINT { x: 0, y: 0 };
                unsafe { GetCursorPos(&mut pt) };
                let mut sub_rect = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetWindowRect(data.submenu_hwnd, &mut sub_rect) };
                if pt.x >= sub_rect.left
                    && pt.x < sub_rect.right
                    && pt.y >= sub_rect.top
                    && pt.y < sub_rect.bottom
                {
                    return 0; // 鼠标在子菜单上，不关闭
                }
            }

            data.hovered_index = -1;
            render_menu(hwnd);
            close_submenu(data);
            0
        }

        WM_ACTIVATE => {
            let activate_state = (wparam & 0xFFFF) as u32;
            if activate_state == WA_INACTIVE {
                let data_ptr =
                    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuWindowData;
                if data_ptr.is_null() {
                    return 0;
                }
                let data = unsafe { &mut *data_ptr };

                // 检查新激活的窗口是否是我们的子菜单
                let activated_hwnd = lparam as HWND;
                if !data.submenu_hwnd.is_null() && activated_hwnd == data.submenu_hwnd {
                    return 0; // 子菜单获得焦点，不关闭
                }

                data.is_closing = true;
            }
            0
        }

        WM_KEYDOWN => {
            let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuWindowData;
            if data_ptr.is_null() {
                return 0;
            }
            let data = unsafe { &mut *data_ptr };

            let vk = wparam as u32;
            match vk {
                VK_ESCAPE => {
                    data.is_closing = true;
                }
                VK_UP => {
                    // 键盘导航：上移
                    let mut idx = data.hovered_index - 1;
                    while idx >= 0 {
                        if !(data.items[idx as usize].is_separator) {
                            break;
                        }
                        idx -= 1;
                    }
                    if idx >= 0 {
                        data.hovered_index = idx;
                        render_menu(hwnd);
                    }
                }
                VK_DOWN => {
                    // 键盘导航：下移
                    let mut idx = data.hovered_index + 1;
                    while (idx as usize) < data.items.len() {
                        if !(data.items[idx as usize].is_separator) {
                            break;
                        }
                        idx += 1;
                    }
                    if (idx as usize) < data.items.len() {
                        data.hovered_index = idx;
                        render_menu(hwnd);
                    }
                }
                VK_RIGHT | VK_RETURN => {
                    // 展开子菜单或确认
                    if data.hovered_index >= 0 && (data.hovered_index as usize) < data.items.len() {
                        let item = &data.items[data.hovered_index as usize];
                        if item.has_submenu() {
                            open_submenu(hwnd, data, data.hovered_index);
                        } else if !item.is_separator && !item.is_disabled && item.id > 0 {
                            data.result_cmd = item.id as i32;
                        }
                    }
                }
                _ => {}
            }
            0
        }

        WM_DESTROY => {
            let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuWindowData;
            if !data_ptr.is_null() {
                let data = unsafe { Box::from_raw(data_ptr) };
                // D2D/DW/WIC 资源通过 Drop 自动释放
                drop(data.d2d_target);
                drop(data.d2d_factory);
                drop(data.dw_factory);
                drop(data.wic_factory);
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            0
        }

        // 子菜单不抢焦点（点击子菜单时主菜单保持前台，确保点击外部能关闭）
        WM_MOUSEACTIVATE => {
            let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuWindowData;
            if !data_ptr.is_null() {
                let data = unsafe { &*data_ptr };
                if data.is_submenu {
                    return MA_NOACTIVATE as LRESULT;
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

// ===== Hit Test =====

fn hit_test_item(data: &MenuWindowData, mx: i32, my: i32) -> i32 {
    for (i, &(top, bottom, left, right)) in data.item_rects.iter().enumerate() {
        if my >= top && my < bottom && mx >= left && mx < right {
            let item = &data.items[i];
            if item.is_separator || item.is_disabled {
                return -1;
            }
            return i as i32;
        }
    }
    -1
}

// ===== Item Rects 计算 =====

fn calculate_item_rects(items: &[MenuItem], dpi: u32) -> Vec<(i32, i32, i32, i32)> {
    let padding_y = scale_for_dpi(MENU_PADDING_Y, dpi);
    let item_h = scale_for_dpi(MENU_ITEM_HEIGHT, dpi);
    let sep_h = scale_for_dpi(MENU_SEPARATOR_HEIGHT, dpi);

    let mut y = padding_y;
    let mut rects = Vec::new();

    for item in items {
        let h = if item.is_separator { sep_h } else { item_h };
        // (top, bottom, left, right) — left=0, right 很大（整个窗口宽度）
        rects.push((y, y + h, 0, 10000));
        y += h;
    }

    rects
}

// ===== 子菜单管理 =====

fn open_submenu(parent_hwnd: HWND, data: &mut MenuWindowData, item_index: i32) {
    // 如果已有同一项的子菜单打开，不重复创建
    if data.submenu_parent_index == item_index && !data.submenu_hwnd.is_null() {
        return;
    }

    // 关闭旧子菜单
    close_submenu(data);

    let item = &data.items[item_index as usize];
    if item.children.is_empty() {
        return;
    }

    // 计算子菜单尺寸
    let (sub_w, sub_h) = calculate_menu_size(&item.children, data.dpi);

    // 获取父菜单位置
    let mut parent_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(parent_hwnd, &mut parent_rect) };

    // 子菜单位置
    let item_rect = &data.item_rects[item_index as usize];
    let (sub_x, sub_y) = adjust_submenu_position(
        parent_rect.right,
        parent_rect.top,
        item_rect.0, // item top
        sub_w,
        sub_h,
        parent_rect.left,
    );

    // 创建子菜单窗口（先不可见，避免 WM_ACTIVATE 时序问题）
    let hinstance = ensure_menu_class_registered();
    let sub_hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            MENU_CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            sub_x,
            sub_y,
            sub_w,
            sub_h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    };

    if sub_hwnd.is_null() {
        log::error!("创建子菜单窗口失败");
        return;
    }

    // 先记录子菜单句柄（在 ShowWindow 之前，确保 WM_ACTIVATE 检查能通过）
    data.submenu_hwnd = sub_hwnd;
    data.submenu_parent_index = item_index;

    // 初始化子菜单数据
    let sub_item_rects = calculate_item_rects(&item.children, data.dpi);

    let sub_d2d_factory =
        unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok() };
    let sub_dw_factory =
        unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED).ok() };
    let sub_wic_factory = unsafe {
        CoCreateInstance::<_, IWICImagingFactory>(
            &CLSID_WICImagingFactory,
            None,
            CLSCTX_INPROC_SERVER,
        )
        .ok()
    };
    let sub_d2d_target = if let Some(ref factory) = sub_d2d_factory {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode:
                    windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 0.0,
            dpiY: 0.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        unsafe { factory.CreateDCRenderTarget(&props).ok() }
    } else {
        None
    };

    let sub_data = Box::new(MenuWindowData {
        items: item
            .children
            .iter()
            .map(|c| MenuItem {
                label: c.label.clone(),
                id: c.id,
                is_separator: c.is_separator,
                is_checked: c.is_checked,
                is_radio: c.is_radio,
                is_disabled: c.is_disabled,
                children: Vec::new(), // 子菜单不支持多级
            })
            .collect(),
        item_rects: sub_item_rects,
        hovered_index: -1,
        menu_width: sub_w,
        menu_height: sub_h,
        dpi: data.dpi,
        parent_hwnd: data.parent_hwnd,
        main_menu_hwnd: parent_hwnd, // 主菜单窗口句柄
        submenu_hwnd: std::ptr::null_mut(),
        submenu_parent_index: -1,
        is_submenu: true,
        ignore_first_lbuttonup: false,
        result_cmd: 0,
        is_closing: false,
        d2d_factory: sub_d2d_factory,
        dw_factory: sub_dw_factory,
        wic_factory: sub_wic_factory,
        d2d_target: sub_d2d_target,
    });

    let sub_data_ptr = Box::into_raw(sub_data);
    unsafe { SetWindowLongPtrW(sub_hwnd, GWLP_USERDATA, sub_data_ptr as isize) };

    // 渲染子菜单（UpdateLayeredWindow 设置内容）
    render_menu(sub_hwnd);

    // 显示子菜单窗口（不激活，主菜单保持前台）
    unsafe { ShowWindow(sub_hwnd, SW_SHOWNOACTIVATE) };
}

fn close_submenu(data: &mut MenuWindowData) {
    if !data.submenu_hwnd.is_null() {
        unsafe { DestroyWindow(data.submenu_hwnd) };
        data.submenu_hwnd = std::ptr::null_mut();
        data.submenu_parent_index = -1;
    }
}

// ===== D2D 渲染 =====

fn render_menu(hwnd: HWND) {
    let data = unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MenuWindowData;
        if ptr.is_null() {
            return;
        }
        &mut *ptr
    };

    let (Some(ref _d2d_factory), Some(ref dw_factory), Some(ref target)) =
        (&data.d2d_factory, &data.dw_factory, &data.d2d_target)
    else {
        return;
    };

    let w = data.menu_width;
    let h = data.menu_height;
    if w <= 0 || h <= 0 {
        return;
    }

    let ww = w as f32;
    let hh = h as f32;
    let dpi = data.dpi;

    // 创建 DIB Section
    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    let mem_dc = unsafe { CreateCompatibleDC(screen_dc) };

    let bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: w,
        bi_height: -h,
        bi_planes: 1,
        bi_bit_count: 32,
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };

    let mut bits: *mut Pixel32 = std::ptr::null_mut();
    let dib = unsafe {
        CreateDIBSection(
            mem_dc,
            &bi as *const BITMAPINFOHEADER as *const c_void,
            DIB_RGB_COLORS,
            &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
            std::ptr::null_mut(),
            0,
        )
    };
    if dib.is_null() || bits.is_null() {
        unsafe {
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc)
        };
        return;
    }

    let old_bmp = unsafe { SelectObject(mem_dc, dib as HGDIOBJ) };

    // 绑定 D2D render target
    let bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    if unsafe { target.BindDC(GdiHDC(mem_dc), &bind_rect) }.is_err() {
        unsafe {
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc)
        };
        return;
    }

    // D2D 渲染开始
    unsafe { target.BeginDraw() };

    // 清空透明
    let transparent = windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    unsafe { target.Clear(Some(&transparent)) };

    // 圆角背景（半透明深色）
    let bg_brush = unsafe { target.CreateSolidColorBrush(&d2d_color(45, 48, 62, 235), None) };
    if let Ok(ref brush) = bg_brush {
        let rounded_rect = D2D1_ROUNDED_RECT {
            rect: d2d_rect(0.0, 0.0, ww, hh),
            radiusX: MENU_CORNER_RADIUS,
            radiusY: MENU_CORNER_RADIUS,
        };
        unsafe { target.FillRoundedRectangle(&rounded_rect, brush) };
    }

    // 绘制菜单项
    let padding_x = scale_for_dpi(MENU_PADDING_X, dpi) as f32;
    let padding_y = scale_for_dpi(MENU_PADDING_Y, dpi);
    let item_h = scale_for_dpi(MENU_ITEM_HEIGHT, dpi);
    let sep_h = scale_for_dpi(MENU_SEPARATOR_HEIGHT, dpi);
    let check_w = scale_for_dpi(MENU_CHECK_WIDTH, dpi);
    let arrow_w = scale_for_dpi(MENU_ARROW_WIDTH, dpi);
    let font_size = scale_for_dpi(MENU_FONT_SIZE, dpi) as f32;

    let mut y = padding_y;

    for (i, item) in data.items.iter().enumerate() {
        if item.is_separator {
            // 分隔线
            let line_brush =
                unsafe { target.CreateSolidColorBrush(&d2d_color(100, 105, 125, 140), None) };
            if let Ok(ref brush) = line_brush {
                let line_y = (y + sep_h / 2) as f32;
                let line_rect = d2d_rect(padding_x, line_y, ww - padding_x, line_y + 1.0);
                unsafe { target.FillRectangle(&line_rect, brush) };
            }
            y += sep_h;
            continue;
        }

        let is_hovered = i as i32 == data.hovered_index;
        let item_top = y as f32;
        let item_bottom = (y + item_h) as f32;

        // Hover 高亮背景
        if is_hovered && !item.is_disabled {
            let hover_brush =
                unsafe { target.CreateSolidColorBrush(&d2d_color(75, 80, 105, 240), None) };
            if let Ok(ref brush) = hover_brush {
                let hover_rect = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(4.0, item_top + 1.0, ww - 4.0, item_bottom - 1.0),
                    radiusX: 4.0,
                    radiusY: 4.0,
                };
                unsafe { target.FillRoundedRectangle(&hover_rect, brush) };
            }
        }

        // 勾选标记
        if item.is_checked {
            let check_str = if item.is_radio { "●" } else { "✓" };
            let check_text: Vec<u16> = check_str.encode_utf16().collect();
            let check_format = unsafe {
                let font_name: Vec<u16> = "Segoe UI Symbol\0".encode_utf16().collect();
                let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
                dw_factory.CreateTextFormat::<_, _, _>(
                    windows::core::PCWSTR(font_name.as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    windows::core::PCWSTR(locale.as_ptr()),
                )
            };
            if let Ok(ref cf) = check_format {
                let _ = unsafe { cf.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) };
                let _ = unsafe { cf.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };
                let check_layout = unsafe {
                    target_inner_create_text_layout(
                        dw_factory,
                        &check_text,
                        cf,
                        check_w as f32,
                        item_h as f32,
                        DWRITE_TEXT_ALIGNMENT_CENTER,
                        DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                    )
                };
                if let Some(ref cl) = check_layout {
                    let check_brush = unsafe {
                        target.CreateSolidColorBrush(&d2d_color(100, 210, 255, 255), None)
                    };
                    if let Ok(ref cb) = check_brush {
                        let origin = windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F {
                            x: 0.0,
                            y: item_top,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, cl, cb, D2D1_DRAW_TEXT_OPTIONS_NONE)
                        };
                    }
                }
            }
        }

        // 标签文字
        let label_text: Vec<u16> = item.label.encode_utf16().collect();
        let label_format = unsafe {
            let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
            let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
            dw_factory.CreateTextFormat::<_, _, _>(
                windows::core::PCWSTR(font_name.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                font_size,
                windows::core::PCWSTR(locale.as_ptr()),
            )
        };
        if let Ok(ref lf) = label_format {
            let _ = unsafe { lf.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP) };
            let text_max_w = ww - check_w as f32 - arrow_w as f32 - padding_x * 2.0;
            let label_layout = unsafe {
                target_inner_create_text_layout(
                    dw_factory,
                    &label_text,
                    lf,
                    text_max_w.max(40.0),
                    item_h as f32,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                )
            };
            if let Some(ref ll) = label_layout {
                let text_color = if item.is_disabled {
                    d2d_color(130, 135, 155, 180)
                } else {
                    d2d_color(220, 220, 230, 255)
                };
                let text_brush = unsafe { target.CreateSolidColorBrush(&text_color, None) };
                if let Ok(ref tb) = text_brush {
                    let origin = windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F {
                        x: check_w as f32 + padding_x,
                        y: item_top,
                    };
                    unsafe { target.DrawTextLayout(origin, ll, tb, D2D1_DRAW_TEXT_OPTIONS_NONE) };
                }
            }
        }

        // 子菜单箭头
        if item.has_submenu() {
            let arrow_text: Vec<u16> = "▶".encode_utf16().collect();
            let arrow_format = unsafe {
                let font_name: Vec<u16> = "Segoe UI Symbol\0".encode_utf16().collect();
                let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
                dw_factory.CreateTextFormat::<_, _, _>(
                    windows::core::PCWSTR(font_name.as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size * 0.7,
                    windows::core::PCWSTR(locale.as_ptr()),
                )
            };
            if let Ok(ref af) = arrow_format {
                let _ = unsafe { af.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) };
                let _ = unsafe { af.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };
                let arrow_layout = unsafe {
                    target_inner_create_text_layout(
                        dw_factory,
                        &arrow_text,
                        af,
                        arrow_w as f32,
                        item_h as f32,
                        DWRITE_TEXT_ALIGNMENT_CENTER,
                        DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                    )
                };
                if let Some(ref al) = arrow_layout {
                    let arrow_color = if item.is_disabled {
                        d2d_color(90, 95, 115, 120)
                    } else {
                        d2d_color(160, 165, 185, 255)
                    };
                    let arrow_brush = unsafe { target.CreateSolidColorBrush(&arrow_color, None) };
                    if let Ok(ref ab) = arrow_brush {
                        let origin = windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F {
                            x: ww - arrow_w as f32 - 4.0,
                            y: item_top,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, al, ab, D2D1_DRAW_TEXT_OPTIONS_NONE)
                        };
                    }
                }
            }
        }

        y += item_h;
    }

    // D2D 渲染结束
    let mut tag1: u64 = 0;
    let mut tag2: u64 = 0;
    if let Err(e) = unsafe { target.EndDraw(Some(&mut tag1), Some(&mut tag2)) } {
        log::error!("菜单 D2D EndDraw 失败: {:?}", e);
    }

    // UpdateLayeredWindow
    let size = POINT { x: w, y: h };
    let src_pt = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 255,
        alpha_format: AC_SRC_ALPHA,
    };
    unsafe {
        UpdateLayeredWindow(
            hwnd,
            screen_dc,
            std::ptr::null(),
            &size,
            mem_dc,
            &src_pt,
            0,
            &blend,
            LWA_ALPHA,
        );
    }

    // 清理 GDI 资源
    unsafe {
        SelectObject(mem_dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }
}

// ===== 命令处理 =====

/// 处理菜单命令
///
/// 返回 `true` 表示分类框已被删除，调用方应销毁窗口（不再渲染）。
/// 返回 `false` 表示仅修改了状态，调用方应重新渲染窗口。
fn handle_menu_command(pd: &mut super::grid_window::GridWindowData, cmd: i32) -> bool {
    let cmd = cmd as u32;
    if cmd == CMD_DELETE {
        log::info!("删除分类框: {}", pd.title);
        return true; // 通知调用方销毁窗口
    } else if cmd >= CMD_SORT_BASE && cmd < CMD_SORT_BASE + 8 {
        let modes = [
            SortMode::Name,
            SortMode::Type,
            SortMode::ModifiedTime,
            SortMode::Author,
            SortMode::CreatedTime,
            SortMode::Size,
            SortMode::Tag,
            SortMode::Title,
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
            GroupBy::Name,
            GroupBy::Type,
            GroupBy::ModifiedTime,
            GroupBy::Author,
            GroupBy::CreatedTime,
            GroupBy::Size,
            GroupBy::Tag,
            GroupBy::Title,
        ];
        pd.group_by = modes[(cmd - CMD_GROUP_BASE) as usize].clone();
        log::info!("分组: {:?}", pd.group_by);
    } else if cmd >= CMD_VIEW_BASE && cmd < CMD_VIEW_BASE + 8 {
        let modes = [
            ViewMode::ExtraLarge,
            ViewMode::Large,
            ViewMode::Medium,
            ViewMode::Small,
            ViewMode::List,
            ViewMode::Detail,
            ViewMode::Tiles,
            ViewMode::Content,
        ];
        let mode = &modes[(cmd - CMD_VIEW_BASE) as usize];
        log::info!("[预留] 查看模式切换: {:?}（暂未实现）", mode);
    }
    false // 非删除操作，窗口保留
}

// ===== 测试 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_menu_items_structure() {
        let items = build_menu_items(&SortMode::Name, true, &GroupBy::None, &ViewMode::Medium);
        assert_eq!(items.len(), 5); // 删除 + 分隔线 + 排序 + 分组 + 查看
        assert_eq!(items[0].label, "删除");
        assert_eq!(items[0].id, CMD_DELETE);
        assert!(items[1].is_separator);
        assert!(items[2].has_submenu()); // 排序
        assert!(items[3].has_submenu()); // 分组
        assert!(items[4].has_submenu()); // 查看
    }

    #[test]
    fn test_view_items_enabled() {
        let items = build_menu_items(&SortMode::Name, true, &GroupBy::None, &ViewMode::Medium);
        let view_item = &items[4]; // 查看
        assert!(view_item.has_submenu());
        for child in &view_item.children {
            assert!(!child.is_disabled, "查看子菜单项应全部可用");
        }
    }

    #[test]
    fn test_sort_checked_state() {
        let items = build_menu_items(&SortMode::Type, false, &GroupBy::None, &ViewMode::Medium);
        let sort_item = &items[2];
        // 第二个子项应该是"类型"且被勾选
        assert!(sort_item.children[1].is_checked);
        assert_eq!(sort_item.children[1].label, "类型");
        // 递减应该被勾选
        let desc_item = sort_item
            .children
            .iter()
            .find(|c| c.label == "递减")
            .unwrap();
        assert!(desc_item.is_checked);
    }

    #[test]
    fn test_calculate_menu_size() {
        let items = vec![
            MenuItem::new("删除", 1),
            MenuItem::separator(),
            MenuItem::new("排序", 0),
        ];
        let (w, h) = calculate_menu_size(&items, 96);
        assert!(w >= 180); // 最小宽度
        assert!(h > 0);
    }

    #[test]
    fn test_command_id_ranges_no_overlap() {
        assert!(CMD_DELETE < CMD_SORT_BASE);
        assert!(CMD_SORT_BASE + 8 <= CMD_SORT_ASC);
        assert!(CMD_SORT_ASC < CMD_SORT_DESC);
        assert!(CMD_SORT_DESC < CMD_GROUP_BASE);
        assert!(CMD_GROUP_BASE + 8 <= CMD_VIEW_BASE);
        assert!(CMD_VIEW_BASE + 8 < 1000);
    }

    #[test]
    fn test_hit_test_item() {
        let data = MenuWindowData {
            items: vec![
                MenuItem::new("删除", 1),
                MenuItem::separator(),
                MenuItem::new("排序", 0),
            ],
            item_rects: vec![(6, 40, 0, 10000), (40, 49, 0, 10000), (49, 83, 0, 10000)],
            hovered_index: -1,
            menu_width: 200,
            menu_height: 89,
            dpi: 96,
            parent_hwnd: std::ptr::null_mut(),
            main_menu_hwnd: std::ptr::null_mut(),
            submenu_hwnd: std::ptr::null_mut(),
            submenu_parent_index: -1,
            is_submenu: false,
            ignore_first_lbuttonup: false,
            result_cmd: 0,
            is_closing: false,
            d2d_factory: None,
            dw_factory: None,
            wic_factory: None,
            d2d_target: None,
        };

        assert_eq!(hit_test_item(&data, 50, 20), 0); // 点击"删除"
        assert_eq!(hit_test_item(&data, 50, 44), -1); // 点击分隔线
        assert_eq!(hit_test_item(&data, 50, 60), 2); // 点击"排序"
        assert_eq!(hit_test_item(&data, 50, 5), -1); // 点击 padding 区域
    }

    #[test]
    fn test_hit_test_disabled_item() {
        let data = MenuWindowData {
            items: vec![
                MenuItem::new("可用", 1),
                MenuItem::new("禁用", 2).disabled(true),
            ],
            item_rects: vec![(6, 40, 0, 10000), (40, 74, 0, 10000)],
            hovered_index: -1,
            menu_width: 200,
            menu_height: 80,
            dpi: 96,
            parent_hwnd: std::ptr::null_mut(),
            main_menu_hwnd: std::ptr::null_mut(),
            submenu_hwnd: std::ptr::null_mut(),
            submenu_parent_index: -1,
            is_submenu: false,
            ignore_first_lbuttonup: false,
            result_cmd: 0,
            is_closing: false,
            d2d_factory: None,
            dw_factory: None,
            wic_factory: None,
            d2d_target: None,
        };

        assert_eq!(hit_test_item(&data, 50, 20), 0); // 可用项
        assert_eq!(hit_test_item(&data, 50, 50), -1); // 禁用项返回 -1
    }

    #[test]
    fn test_calculate_item_rects() {
        let items = vec![
            MenuItem::new("删除", 1),
            MenuItem::separator(),
            MenuItem::new("排序", 0),
        ];
        let rects = calculate_item_rects(&items, 96);
        assert_eq!(rects.len(), 3);

        // 第一项从 padding_y 开始
        let padding_y = scale_for_dpi(MENU_PADDING_Y, 96);
        let item_h = scale_for_dpi(MENU_ITEM_HEIGHT, 96);
        let sep_h = scale_for_dpi(MENU_SEPARATOR_HEIGHT, 96);

        assert_eq!(rects[0].0, padding_y); // 第一项 top
        assert_eq!(rects[0].1, padding_y + item_h); // 第一项 bottom

        assert_eq!(rects[1].0, padding_y + item_h); // 分隔线 top
        assert_eq!(rects[1].1, padding_y + item_h + sep_h); // 分隔线 bottom

        assert_eq!(rects[2].0, padding_y + item_h + sep_h); // 第三项 top
    }

    #[test]
    fn test_calculate_menu_size_dpi_scaling() {
        let items = vec![MenuItem::new("测试", 1)];

        let (w96, h96) = calculate_menu_size(&items, 96);
        let (w144, h144) = calculate_menu_size(&items, 144);
        let (w192, h192) = calculate_menu_size(&items, 192);

        // 高 DPI 下尺寸应该更大
        assert!(w144 > w96);
        assert!(h144 > h96);
        assert!(w192 > w144);
        assert!(h192 > h144);
    }

    #[test]
    fn test_handle_menu_command_view_no_state_change() {
        let mut data = super::super::grid_window::GridWindowData::test_default();
        let original_view = data.view_mode.clone();

        // 查看命令不应改变 view_mode
        handle_menu_command(&mut data, CMD_VIEW_BASE as i32);
        assert_eq!(data.view_mode, original_view, "查看命令不应改变 view_mode");

        handle_menu_command(&mut data, (CMD_VIEW_BASE + 3) as i32);
        assert_eq!(data.view_mode, original_view, "查看命令不应改变 view_mode");
    }

    #[test]
    fn test_handle_menu_command_sort_changes_state() {
        let mut data = super::super::grid_window::GridWindowData::test_default();

        handle_menu_command(&mut data, CMD_SORT_BASE as i32); // Name
        assert_eq!(data.sort_mode, SortMode::Name);

        handle_menu_command(&mut data, (CMD_SORT_BASE + 5) as i32); // Size
        assert_eq!(data.sort_mode, SortMode::Size);

        handle_menu_command(&mut data, CMD_SORT_ASC as i32);
        assert!(data.sort_ascending);

        handle_menu_command(&mut data, CMD_SORT_DESC as i32);
        assert!(!data.sort_ascending);
    }

    #[test]
    fn test_handle_menu_command_group_changes_state() {
        let mut data = super::super::grid_window::GridWindowData::test_default();

        handle_menu_command(&mut data, CMD_GROUP_BASE as i32); // Name
        assert_eq!(data.group_by, GroupBy::Name);

        handle_menu_command(&mut data, (CMD_GROUP_BASE + 3) as i32); // Author
        assert_eq!(data.group_by, GroupBy::Author);
    }

    #[test]
    fn test_handle_menu_command_delete_returns_true() {
        let mut data = super::super::grid_window::GridWindowData::test_default();
        // CMD_DELETE 应返回 true（通知调用方销毁窗口）
        assert!(handle_menu_command(&mut data, CMD_DELETE as i32));
    }

    #[test]
    fn test_handle_menu_command_non_delete_returns_false() {
        let mut data = super::super::grid_window::GridWindowData::test_default();
        // 排序命令应返回 false（不删除窗口）
        assert!(!handle_menu_command(&mut data, CMD_SORT_BASE as i32));
        // 分组命令应返回 false
        assert!(!handle_menu_command(&mut data, CMD_GROUP_BASE as i32));
        // 查看命令应返回 false
        assert!(!handle_menu_command(&mut data, CMD_VIEW_BASE as i32));
    }

    #[test]
    fn test_menu_item_builder_chaining() {
        let item = MenuItem::new("测试", 42)
            .checked(true)
            .radio(true)
            .disabled(false);
        assert_eq!(item.label, "测试");
        assert_eq!(item.id, 42);
        assert!(item.is_checked);
        assert!(item.is_radio);
        assert!(!item.is_disabled);
        assert!(!item.has_submenu());
    }

    #[test]
    fn test_menu_group_checked_state() {
        let items = build_menu_items(&SortMode::Name, true, &GroupBy::Author, &ViewMode::Medium);
        let group_item = &items[3]; // 分组
        let author_child = group_item
            .children
            .iter()
            .find(|c| c.label == "作者")
            .unwrap();
        assert!(author_child.is_checked, "当前分组为 Author，应被勾选");

        let name_child = group_item
            .children
            .iter()
            .find(|c| c.label == "名称")
            .unwrap();
        assert!(!name_child.is_checked, "非当前分组不应被勾选");
    }
}
