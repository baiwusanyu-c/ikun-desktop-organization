//! 菜单弹出窗口 — 独立分层窗口（WS_POPUP | WS_EX_LAYERED）
//!
//! 菜单以独立窗口形式悬浮在分类框上方，不影响分类框的尺寸和布局。
//! 使用 Direct2D/DirectWrite 渲染，与分类框使用相同的渲染管线。
//!
//! # 交互
//! - 主菜单项：删除（叶子）、排序/分组/查看（展开子菜单）
//! - 子菜单：hover 展开，300ms 延迟收起
//! - 点击菜单外区域自动关闭（WM_ACTIVATE 检测失焦）
//! - 菜单打开期间分类框处于锁定状态

use std::sync::Mutex;

use windows::Win32::Graphics::Direct2D::{
    Common::{D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_SIZE_U},
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::{get_data, GridWindowData};

use crate::config::app_config::{GroupBy, SortMode, ViewMode};
use std::ffi::c_void;

// ===== 常量 =====
const MENU_TIMER_COLLAPSE: usize = 9901;
const MENU_COLLAPSE_DELAY_MS: u32 = 300;

static CLASS_NAME: &[u16] = &[
    68, 101, 115, 107, 116, 111, 112, 79, 114, 103, 77, 101, 110, 117, 0,
]; // "DesktopOrgMenu"

static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

/// HWND 包装器，允许跨线程传递
#[repr(transparent)]
struct SendHwnd(HWND);
unsafe impl Send for SendHwnd {}

static MENU_HWND: Mutex<SendHwnd> = Mutex::new(SendHwnd(std::ptr::null_mut()));

// ===== 菜单窗口数据 =====

/// 菜单窗口内部数据（存储在 GWLP_USERDATA）
struct MenuWindowData {
    /// 父分类框窗口句柄
    parent_hwnd: HWND,
    /// 菜单布局尺寸
    item_h: i32,
    main_w: i32,
    sub_w: i32,
    sep_h: i32,
    /// 子菜单收起延时定时器 ID
    collapse_timer_id: usize,
    // ===== D2D/DW 设备 =====
    d2d_factory: Option<ID2D1Factory>,
    dw_factory: Option<IDWriteFactory>,
    wic_factory: Option<IWICImagingFactory>,
    d2d_target: Option<ID2D1DCRenderTarget>,
}

unsafe impl Send for MenuWindowData {}

// ===== 公共 API =====

/// 显示菜单弹出窗口
///
/// 根据"更多"按钮的屏幕坐标定位，创建独立分层窗口渲染菜单。
/// 子菜单默认在右侧，右侧空间不足时自动切换到左侧。
pub fn show_menu_window(parent_hwnd: HWND) {
    // 防止重复创建
    {
        let existing = MENU_HWND.lock().unwrap();
        if !existing.0.is_null() {
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

    // 计算菜单位置（基于"更多"按钮的屏幕坐标）
    let (btn_x, btn_y, btn_size) =
        super::grid_window::more_btn_rect(parent_data.base_window_w, parent_data.title_h, dpi);

    let mut parent_rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(parent_hwnd, &mut parent_rect) };

    // 按钮的屏幕坐标
    let btn_screen_x = parent_rect.left + btn_x;
    let btn_screen_y = parent_rect.top + btn_y;

    let main_w = scale_for_dpi(160, dpi);
    let sub_w = scale_for_dpi(160, dpi);
    let item_h = scale_for_dpi(36, dpi);
    let sep_h = scale_for_dpi(1, dpi);
    let menu_h = item_h * 4 + sep_h + scale_for_dpi(16, dpi);
    let gap = scale_for_dpi(2, dpi);

    // 主菜单位置（紧贴"更多"按钮下方，右对齐）
    let menu_x = btn_screen_x + btn_size - main_w;
    let menu_y = btn_screen_y + btn_size + gap;

    // 子菜单方向：默认右侧，空间不足时左侧
    let screen_w = unsafe { GetSystemMetrics(0) };
    let sub_on_left = menu_x + main_w + gap + sub_w > screen_w;

    // 菜单窗口总尺寸
    let total_w = if sub_on_left {
        sub_w + gap + main_w
    } else {
        main_w + gap + sub_w
    };
    let total_h = menu_h.max(item_h * 11 + scale_for_dpi(16, dpi));

    // 窗口位置
    let win_x = if sub_on_left {
        menu_x - sub_w - gap
    } else {
        menu_x
    };
    let win_y = menu_y;

    // 保存菜单状态到父窗口
    parent_data.menu.open = true;
    parent_data.menu.sub_on_left = sub_on_left;
    parent_data.menu.hovered_item = -1;
    parent_data.menu.expanded_item = -1;
    parent_data.menu.sub_hovered = -1;

    // 锁定分类框
    parent_data.locked = true;

    // 注册窗口类
    ensure_class_registered();

    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

    // 创建分层弹出窗口
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | 0x00080000, // WS_EX_NOACTIVATE
            CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_POPUP | WS_VISIBLE,
            win_x,
            win_y,
            total_w,
            total_h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    };

    if hwnd.is_null() {
        log::error!("创建菜单窗口失败");
        parent_data.menu.open = false;
        parent_data.locked = false;
        return;
    }

    // 存储窗口数据
    let data = Box::new(MenuWindowData {
        parent_hwnd,
        item_h,
        main_w,
        sub_w,
        sep_h,
        collapse_timer_id: 0,
        d2d_factory: None,
        dw_factory: None,
        wic_factory: None,
        d2d_target: None,
    });
    let data_ptr = Box::into_raw(data);
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize) };

    // 保存到全局
    MENU_HWND.lock().unwrap().0 = hwnd;

    // 首次渲染
    render_menu(hwnd);

    // 重绘父窗口（显示按钮高亮）
    render_layered_window(parent_hwnd);

    log::info!(
        "菜单窗口已创建: pos=({},{}), size={}x{}, sub_on_left={}",
        win_x, win_y, total_w, total_h, sub_on_left
    );
}

/// 关闭菜单弹出窗口
pub fn hide_menu_window() {
    let hwnd = MENU_HWND.lock().unwrap().0;
    if !hwnd.is_null() {
        unsafe {
            if (*get_menu_data(hwnd)).collapse_timer_id != 0 {
                KillTimer(hwnd, (*get_menu_data(hwnd)).collapse_timer_id);
            }
        }
        // 获取父窗口并解锁
        let parent_hwnd = unsafe { (*get_menu_data(hwnd)).parent_hwnd };
        unsafe { DestroyWindow(hwnd) };
        MENU_HWND.lock().unwrap().0 = std::ptr::null_mut();

        // 解锁父窗口并更新状态
        if let Some(pd) = unsafe { get_data(parent_hwnd) } {
            pd.menu.open = false;
            pd.menu.hovered_item = -1;
            pd.menu.expanded_item = -1;
            pd.menu.sub_hovered = -1;
            pd.locked = false;
            render_layered_window(parent_hwnd);
        }
    }
}

/// 菜单窗口是否正在显示
pub fn is_menu_visible() -> bool {
    !MENU_HWND.lock().unwrap().0.is_null()
}

// ===== 窗口类注册 =====

fn ensure_class_registered() {
    let mut registered = CLASS_REGISTERED.lock().unwrap();
    if !*registered {
        let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
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
            lpsz_class_name: CLASS_NAME.as_ptr(),
            h_icon_sm: std::ptr::null_mut(),
        };
        unsafe { RegisterClassExW(&wc) };
        *registered = true;
    }
}

// ===== 辅助函数 =====

unsafe fn get_menu_data<'a>(hwnd: HWND) -> &'a mut MenuWindowData {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MenuWindowData;
    &mut *ptr
}

unsafe fn get_parent_data<'a>(hwnd: HWND) -> Option<&'a mut GridWindowData> {
    let menu_data = get_menu_data(hwnd);
    get_data(menu_data.parent_hwnd)
}

// ===== 渲染 =====

fn render_menu(hwnd: HWND) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RENDERING: AtomicBool = AtomicBool::new(false);
    if RENDERING.swap(true, Ordering::SeqCst) {
        return;
    }

    let data = unsafe { get_menu_data(hwnd) };

    // 初始化 D2D 设备
    init_menu_d2d(data);

    let (Some(ref _d2d), Some(ref dw), Some(ref _wic), Some(ref target)) = (
        &data.d2d_factory,
        &data.dw_factory,
        &data.wic_factory,
        &data.d2d_target,
    ) else {
        RENDERING.store(false, Ordering::SeqCst);
        return;
    };

    let mut wr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut wr) };
    let w = wr.right - wr.left;
    let h = wr.bottom - wr.top;
    if w <= 0 || h <= 0 {
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    let ww = w as f32;
    let hh = h as f32;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };

    // 创建 DIB
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
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }
    let old_bmp = unsafe { SelectObject(mem_dc, dib as HGDIOBJ) };

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
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    unsafe { target.BeginDraw() };
    unsafe {
        target.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }))
    };

    // 获取父窗口状态
    let parent_data = unsafe { get_parent_data(hwnd) };
    let (
        sort_mode,
        sort_ascending,
        group_by,
        view_mode,
        hovered_item,
        expanded_item,
        sub_hovered,
        sub_on_left,
    ) = if let Some(pd) = &parent_data {
        let m = &pd.menu;
        (
            pd.sort_mode.clone(),
            pd.sort_ascending,
            pd.group_by.clone(),
            pd.view_mode.clone(),
            m.hovered_item,
            m.expanded_item,
            m.sub_hovered,
            m.sub_on_left,
        )
    } else {
        (
            SortMode::default(),
            true,
            GroupBy::default(),
            ViewMode::default(),
            -1,
            -1,
            -1,
            false,
        )
    };

    let item_h = data.item_h;
    let main_w = data.main_w;
    let padding = scale_for_dpi(4, dpi);

    // 计算主菜单在窗口内的位置
    let mx = if sub_on_left {
        data.sub_w + scale_for_dpi(2, dpi)
    } else {
        0
    };

    // ===== 主菜单背景 =====
    let menu_bg = unsafe { target.CreateSolidColorBrush(&d2d_color(10, 10, 15, 180), None) };
    let menu_border = unsafe { target.CreateSolidColorBrush(&d2d_color(60, 60, 80, 120), None) };
    let mh = item_h * 4 + data.sep_h + scale_for_dpi(16, dpi);
    if let Ok(ref bg) = menu_bg {
        let rounded = D2D1_ROUNDED_RECT {
            rect: d2d_rect(mx as f32, 0.0, (mx + main_w) as f32, mh as f32),
            radiusX: 8.0,
            radiusY: 8.0,
        };
        unsafe { target.FillRoundedRectangle(&rounded, bg) };
    }
    if let Ok(ref bb) = menu_border {
        let border_rect = D2D1_ROUNDED_RECT {
            rect: d2d_rect(mx as f32, 0.0, (mx + main_w) as f32, mh as f32),
            radiusX: 8.0,
            radiusY: 8.0,
        };
        unsafe {
            target.DrawRoundedRectangle(&border_rect, bb, 1.0, None);
        }
    }

    // ===== 主菜单项 =====
    let labels = ["删除", "排序", "分组", "查看"];
    let has_sub = [false, true, true, true];
    let text_brush =
        unsafe { target.CreateSolidColorBrush(&d2d_color(240, 240, 255, 255), None) };
    let hover_brush =
        unsafe { target.CreateSolidColorBrush(&d2d_color(60, 70, 100, 180), None) };

    let menu_font_size = scale_for_dpi(14, dpi) as f32;
    let menu_format = unsafe {
        let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
        let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
        dw.CreateTextFormat::<_, _, _>(
            windows::core::PCWSTR(font_name.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            menu_font_size,
            windows::core::PCWSTR(locale.as_ptr()),
        )
    };
    let arrow_format = unsafe {
        let font_name: Vec<u16> = "Segoe UI Symbol\0".encode_utf16().collect();
        let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
        dw.CreateTextFormat::<_, _, _>(
            windows::core::PCWSTR(font_name.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            scale_for_dpi(10, dpi) as f32,
            windows::core::PCWSTR(locale.as_ptr()),
        )
    };

    if let Ok(ref mf) = menu_format {
        let _ = unsafe { mf.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };
        let mut cur_y = padding;

        for (i, label) in labels.iter().enumerate() {
            let iy = cur_y;
            // hover 背景
            if hovered_item == i as i32 {
                if let Ok(ref hb) = hover_brush {
                    let hl = d2d_rect(
                        (mx + 2) as f32,
                        iy as f32,
                        (mx + main_w - 2) as f32,
                        (iy + item_h) as f32,
                    );
                    unsafe { target.FillRectangle(&hl, hb) };
                }
            }
            // 文字
            let text: Vec<u16> = label.encode_utf16().collect();
            if let Some(tl) = unsafe {
                target_inner_create_text_layout(
                    dw,
                    &text,
                    mf,
                    (main_w - scale_for_dpi(40, dpi)) as f32,
                    item_h as f32,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                )
            } {
                if let Ok(ref tb) = text_brush {
                    let origin = D2D_POINT_2F {
                        x: (mx + scale_for_dpi(16, dpi)) as f32,
                        y: iy as f32,
                    };
                    unsafe {
                        target.DrawTextLayout(origin, &tl, tb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
            // 箭头
            if has_sub[i] {
                if let Ok(ref af) = arrow_format {
                    let arrow: Vec<u16> = "▶".encode_utf16().collect();
                    if let Some(al) = unsafe {
                        target_inner_create_text_layout(
                            dw,
                            &arrow,
                            af,
                            scale_for_dpi(20, dpi) as f32,
                            item_h as f32,
                            DWRITE_TEXT_ALIGNMENT_CENTER,
                            DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                        )
                    } {
                        if let Ok(ref tb) = text_brush {
                            let origin = D2D_POINT_2F {
                                x: (mx + main_w - scale_for_dpi(24, dpi)) as f32,
                                y: iy as f32,
                            };
                            unsafe {
                                target.DrawTextLayout(
                                    origin,
                                    &al,
                                    tb,
                                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                                );
                            }
                        }
                    }
                }
            }
            cur_y += item_h;
            // 分隔线（删除后）
            if i == 0 {
                let sep_brush =
                    unsafe { target.CreateSolidColorBrush(&d2d_color(80, 80, 100, 120), None) };
                if let Ok(ref sb) = sep_brush {
                    let sr = d2d_rect(
                        (mx + scale_for_dpi(8, dpi)) as f32,
                        cur_y as f32,
                        (mx + main_w - scale_for_dpi(8, dpi)) as f32,
                        (cur_y + data.sep_h) as f32,
                    );
                    unsafe { target.FillRectangle(&sr, sb) };
                }
                cur_y += data.sep_h;
            }
        }
    }

    // ===== 子菜单渲染 =====
    if expanded_item >= 1 && expanded_item <= 3 {
        let sort_modes = [
            SortMode::Name,
            SortMode::Type,
            SortMode::ModifiedTime,
            SortMode::Author,
            SortMode::CreatedTime,
            SortMode::Size,
            SortMode::Tag,
            SortMode::Title,
        ];
        let group_modes = [
            GroupBy::Name,
            GroupBy::Type,
            GroupBy::ModifiedTime,
            GroupBy::Author,
            GroupBy::CreatedTime,
            GroupBy::Size,
            GroupBy::Tag,
            GroupBy::Title,
        ];
        let view_modes = [
            ViewMode::ExtraLarge,
            ViewMode::Large,
            ViewMode::Medium,
            ViewMode::Small,
            ViewMode::List,
            ViewMode::Detail,
            ViewMode::Tiles,
            ViewMode::Content,
        ];

        let sub_labels: Vec<&str> = match expanded_item {
            1 => sort_modes
                .iter()
                .map(|m| match m {
                    SortMode::Name => "名称",
                    SortMode::Type => "类型",
                    SortMode::ModifiedTime => "修改时间",
                    SortMode::Author => "作者",
                    SortMode::CreatedTime => "创建时间",
                    SortMode::Size => "大小",
                    SortMode::Tag => "标记",
                    SortMode::Title => "标题",
                })
                .collect(),
            2 => group_modes
                .iter()
                .map(|g| match g {
                    GroupBy::Name => "名称",
                    GroupBy::Type => "类型",
                    GroupBy::ModifiedTime => "修改时间",
                    GroupBy::Author => "作者",
                    GroupBy::CreatedTime => "创建时间",
                    GroupBy::Size => "大小",
                    GroupBy::Tag => "标记",
                    GroupBy::Title => "标题",
                    _ => "无",
                })
                .collect(),
            3 => view_modes
                .iter()
                .map(|v| match v {
                    ViewMode::ExtraLarge => "超大图标",
                    ViewMode::Large => "大图标",
                    ViewMode::Medium => "中图标",
                    ViewMode::Small => "小图标",
                    ViewMode::List => "列表",
                    ViewMode::Detail => "详细信息",
                    ViewMode::Tiles => "平铺",
                    ViewMode::Content => "内容",
                })
                .collect(),
            _ => Vec::new(),
        };

        let has_order = expanded_item == 1;
        let dim_count = sub_labels.len() as i32;
        let sub_count = if has_order { dim_count + 2 } else { dim_count };

        let sx = if sub_on_left {
            0
        } else {
            main_w + scale_for_dpi(2, dpi)
        };
        let sub_sh = item_h * sub_count
            + scale_for_dpi(8, dpi)
            + if has_order { scale_for_dpi(24, dpi) } else { 0 };

        // 子菜单背景
        if let Ok(ref bg) = menu_bg {
            let rounded = D2D1_ROUNDED_RECT {
                rect: d2d_rect(sx as f32, 0.0, (sx + data.sub_w) as f32, sub_sh as f32),
                radiusX: 8.0,
                radiusY: 8.0,
            };
            unsafe { target.FillRoundedRectangle(&rounded, bg) };
        }
        if let Ok(ref bb) = menu_border {
            let border_rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(sx as f32, 0.0, (sx + data.sub_w) as f32, sub_sh as f32),
                radiusX: 8.0,
                radiusY: 8.0,
            };
            unsafe {
                target.DrawRoundedRectangle(&border_rect, bb, 1.0, None);
            }
        }

        // 子菜单项
        if let Ok(ref mf) = menu_format {
            let checked_brush =
                unsafe { target.CreateSolidColorBrush(&d2d_color(100, 210, 255, 255), None) };
            let dim_label_brush =
                unsafe { target.CreateSolidColorBrush(&d2d_color(140, 140, 160, 200), None) };
            let mut cur_y = padding;

            // 维度项
            for (j, label) in sub_labels.iter().enumerate() {
                let iy = cur_y;
                if sub_hovered == j as i32 {
                    if let Ok(ref hb) = hover_brush {
                        let hl = d2d_rect(
                            (sx + 2) as f32,
                            iy as f32,
                            (sx + data.sub_w - 2) as f32,
                            (iy + item_h) as f32,
                        );
                        unsafe { target.FillRectangle(&hl, hb) };
                    }
                }
                // 选中检查
                let is_checked = match expanded_item {
                    1 => sort_modes[j] == sort_mode,
                    2 => group_modes[j] == group_by,
                    3 => view_modes[j] == view_mode,
                    _ => false,
                };
                let display = if is_checked {
                    format!("✓ {}", label)
                } else {
                    format!("  {}", label)
                };
                let text: Vec<u16> = display.encode_utf16().collect();
                if let Some(tl) = unsafe {
                    target_inner_create_text_layout(
                        dw,
                        &text,
                        mf,
                        (data.sub_w - scale_for_dpi(16, dpi)) as f32,
                        item_h as f32,
                        DWRITE_TEXT_ALIGNMENT_LEADING,
                        DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                    )
                } {
                    let brush = if is_checked { &checked_brush } else { &text_brush };
                    if let Ok(ref b) = brush {
                        let origin = D2D_POINT_2F {
                            x: (sx + scale_for_dpi(12, dpi)) as f32,
                            y: iy as f32,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, &tl, b, D2D1_DRAW_TEXT_OPTIONS_NONE);
                        }
                    }
                }
                cur_y += item_h;
            }

            // 排序方向区域
            if has_order {
                // 分隔线 + 标题
                cur_y += scale_for_dpi(4, dpi);
                let sep_brush =
                    unsafe { target.CreateSolidColorBrush(&d2d_color(80, 80, 100, 140), None) };
                if let Ok(ref sb) = sep_brush {
                    let sr = d2d_rect(
                        (sx + scale_for_dpi(8, dpi)) as f32,
                        cur_y as f32,
                        (sx + data.sub_w - scale_for_dpi(8, dpi)) as f32,
                        (cur_y + 1) as f32,
                    );
                    unsafe { target.FillRectangle(&sr, sb) };
                }
                cur_y += scale_for_dpi(4, dpi);
                // "排序方向" 标签
                let header_text = "  排序方向";
                let header: Vec<u16> = header_text.encode_utf16().collect();
                let header_fmt = unsafe {
                    let font_name: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
                    let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
                    dw.CreateTextFormat::<_, _, _>(
                        windows::core::PCWSTR(font_name.as_ptr()),
                        None,
                        DWRITE_FONT_WEIGHT_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        scale_for_dpi(12, dpi) as f32,
                        windows::core::PCWSTR(locale.as_ptr()),
                    )
                };
                if let Ok(ref hf) = header_fmt {
                    if let Some(hl) = unsafe {
                        target_inner_create_text_layout(
                            dw,
                            &header,
                            hf,
                            (data.sub_w - scale_for_dpi(16, dpi)) as f32,
                            scale_for_dpi(16, dpi) as f32,
                            DWRITE_TEXT_ALIGNMENT_LEADING,
                            DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                        )
                    } {
                        if let Ok(ref db) = dim_label_brush {
                            let origin = D2D_POINT_2F {
                                x: (sx + scale_for_dpi(4, dpi)) as f32,
                                y: cur_y as f32,
                            };
                            unsafe {
                                target.DrawTextLayout(
                                    origin,
                                    &hl,
                                    db,
                                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                                );
                            }
                        }
                    }
                }
                cur_y += scale_for_dpi(16, dpi);

                // 递增/递减
                let order_labels = ["递增", "递减"];
                for (k, label) in order_labels.iter().enumerate() {
                    let iy = cur_y;
                    let order_idx = dim_count + k as i32;
                    if sub_hovered == order_idx {
                        if let Ok(ref hb) = hover_brush {
                            let hl = d2d_rect(
                                (sx + 2) as f32,
                                iy as f32,
                                (sx + data.sub_w - 2) as f32,
                                (iy + item_h) as f32,
                            );
                            unsafe { target.FillRectangle(&hl, hb) };
                        }
                    }
                    let is_checked = (k == 0 && sort_ascending) || (k == 1 && !sort_ascending);
                    let display = if is_checked {
                        format!("✓ {}", label)
                    } else {
                        format!("  {}", label)
                    };
                    let text: Vec<u16> = display.encode_utf16().collect();
                    if let Some(tl) = unsafe {
                        target_inner_create_text_layout(
                            dw,
                            &text,
                            mf,
                            (data.sub_w - scale_for_dpi(16, dpi)) as f32,
                            item_h as f32,
                            DWRITE_TEXT_ALIGNMENT_LEADING,
                            DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                        )
                    } {
                        let brush = if is_checked {
                            &checked_brush
                        } else {
                            &text_brush
                        };
                        if let Ok(ref b) = brush {
                            let origin = D2D_POINT_2F {
                                x: (sx + scale_for_dpi(12, dpi)) as f32,
                                y: iy as f32,
                            };
                            unsafe {
                                target.DrawTextLayout(
                                    origin,
                                    &tl,
                                    b,
                                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                                );
                            }
                        }
                    }
                    cur_y += item_h;
                }
            }
        }
    }

    // EndDraw + UpdateLayeredWindow
    let mut tag1: u64 = 0;
    let mut tag2: u64 = 0;
    let _ = unsafe { target.EndDraw(Some(&mut tag1), Some(&mut tag2)) };

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
        SelectObject(mem_dc, old_bmp);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }
    RENDERING.store(false, Ordering::SeqCst);
}

// ===== D2D 设备初始化 =====

fn init_menu_d2d(data: &mut MenuWindowData) {
    if data.d2d_factory.is_none() {
        if let Ok(f) = unsafe {
            D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
        } {
            data.d2d_factory = Some(f);
        }
    }
    if data.dw_factory.is_none() {
        if let Ok(f) = unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) }
        {
            data.dw_factory = Some(f);
        }
    }
    if data.wic_factory.is_none() {
        if let Ok(f) = unsafe {
            CoCreateInstance::<_, IWICImagingFactory>(
                &CLSID_WICImagingFactory,
                None,
                CLSCTX_INPROC_SERVER,
            )
        } {
            data.wic_factory = Some(f);
        }
    }
    if data.d2d_target.is_none() {
        if let Some(ref factory) = data.d2d_factory {
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
            if let Ok(rt) = unsafe { factory.CreateDCRenderTarget(&props) } {
                data.d2d_target = Some(rt);
            }
        }
    }
}

// ===== Hit Test =====

/// 命中测试：返回 (主菜单项索引, 子菜单项索引)
fn hit_test_menu(hwnd: HWND, mx: i32, my: i32) -> (i32, i32) {
    let data = unsafe { get_menu_data(hwnd) };
    let parent_data = unsafe { get_parent_data(hwnd) };
    let (expanded_item, sub_on_left) = parent_data
        .map(|pd| (pd.menu.expanded_item, pd.menu.sub_on_left))
        .unwrap_or((-1, false));

    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let padding = scale_for_dpi(4, dpi);
    let item_h = data.item_h;
    let sep_h = data.sep_h;
    let main_w = data.main_w;

    let main_x0 = if sub_on_left {
        data.sub_w + scale_for_dpi(2, dpi)
    } else {
        0
    };
    let main_mh = item_h * 4 + sep_h + scale_for_dpi(16, dpi);

    // 主菜单 hit test
    let mut main_hit: i32 = -1;
    if mx >= main_x0 && mx < main_x0 + main_w && my >= 0 && my < main_mh {
        let rel_y = my - padding;
        if rel_y >= 0 {
            let mut iy = 0;
            for i in 0..4i32 {
                if rel_y >= iy && rel_y < iy + item_h {
                    main_hit = i;
                    break;
                }
                iy += item_h;
                if i == 0 {
                    iy += sep_h;
                }
            }
        }
    }

    // 子菜单 hit test
    let mut sub_hit: i32 = -1;
    if expanded_item >= 1 && expanded_item <= 3 {
        let sub_x0 = if sub_on_left {
            0
        } else {
            main_w + scale_for_dpi(2, dpi)
        };
        let has_order = expanded_item == 1;
        let dim_count: i32 = 8;
        let sub_count = if has_order { dim_count + 2 } else { dim_count };
        let sub_sh = item_h * sub_count
            + scale_for_dpi(8, dpi)
            + if has_order {
                scale_for_dpi(24, dpi)
            } else {
                0
            };

        if mx >= sub_x0 && mx < sub_x0 + data.sub_w && my >= 0 && my < sub_sh {
            let rel_y = my - padding;
            if rel_y >= 0 {
                let idx = rel_y / item_h;
                if idx < dim_count {
                    sub_hit = idx;
                } else if has_order {
                    let after_dims = item_h * dim_count + scale_for_dpi(24, dpi);
                    let order_rel = rel_y - after_dims;
                    if order_rel >= 0 {
                        let order_idx = order_rel / item_h;
                        if order_idx == 0 {
                            sub_hit = dim_count;
                        } else if order_idx == 1 {
                            sub_hit = dim_count + 1;
                        }
                    }
                }
            }
        }
    }

    (main_hit, sub_hit)
}

// ===== 窗口过程 =====

unsafe extern "system" fn menu_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            // 全部返回 HTCLIENT 以接收所有鼠标事件
            HTCLIENT as LRESULT
        }

        WM_MOUSEMOVE => {
            let mx = get_x_lparam(lparam);
            let my = get_y_lparam(lparam);

            let (main_hit, sub_hit) = hit_test_menu(hwnd, mx, my);

            let parent_data = get_parent_data(hwnd);
            if let Some(pd) = parent_data {
                let mut need_rerender = false;

                // 更新主菜单 hover
                if pd.menu.hovered_item != main_hit {
                    pd.menu.hovered_item = main_hit;
                    need_rerender = true;

                    if main_hit >= 1 && main_hit <= 3 {
                        // hover 到有子菜单的项 → 取消定时器，展开
                        if pd.menu.collapse_timer_id != 0 {
                            KillTimer(hwnd, pd.menu.collapse_timer_id);
                            pd.menu.collapse_timer_id = 0;
                        }
                        if pd.menu.expanded_item != main_hit {
                            pd.menu.expanded_item = main_hit;
                            pd.menu.sub_hovered = -1;
                            resize_menu_for_sub(hwnd);
                        }
                    } else if main_hit == 0 || main_hit == -1 {
                        // hover 到无子菜单项 → 启动延迟收起
                        if pd.menu.expanded_item >= 1 && pd.menu.collapse_timer_id == 0 {
                            pd.menu.collapse_timer_id = SetTimer(
                                hwnd,
                                MENU_TIMER_COLLAPSE,
                                MENU_COLLAPSE_DELAY_MS,
                                std::ptr::null_mut(),
                            );
                        }
                    }
                }

                // 更新子菜单 hover
                if pd.menu.sub_hovered != sub_hit {
                    pd.menu.sub_hovered = sub_hit;
                    need_rerender = true;
                    if sub_hit >= 0 && pd.menu.collapse_timer_id != 0 {
                        KillTimer(hwnd, pd.menu.collapse_timer_id);
                        pd.menu.collapse_timer_id = 0;
                    }
                }

                if need_rerender {
                    render_menu(hwnd);
                    let parent_hwnd = get_menu_data(hwnd).parent_hwnd;
                    render_layered_window(parent_hwnd);
                }
            }

            // TrackMouseEvent for WM_MOUSELEAVE
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut tme);
            0
        }

        WM_MOUSELEAVE => {
            if let Some(pd) = get_parent_data(hwnd) {
                pd.menu.hovered_item = -1;
                pd.menu.sub_hovered = -1;
                if pd.menu.expanded_item >= 1 && pd.menu.collapse_timer_id == 0 {
                    pd.menu.collapse_timer_id = SetTimer(
                        hwnd,
                        MENU_TIMER_COLLAPSE,
                        MENU_COLLAPSE_DELAY_MS,
                        std::ptr::null_mut(),
                    );
                }
                let parent_hwnd = get_menu_data(hwnd).parent_hwnd;
                render_layered_window(parent_hwnd);
            }
            render_menu(hwnd);
            0
        }

        WM_LBUTTONDOWN => {
            let mx = get_x_lparam(lparam);
            let my = get_y_lparam(lparam);
            let (main_hit, sub_hit) = hit_test_menu(hwnd, mx, my);

            let parent_data = get_parent_data(hwnd);
            if let Some(pd) = parent_data {
                if sub_hit >= 0 {
                    // 子菜单项点击 → 执行操作
                    let expanded = pd.menu.expanded_item;
                    let dim_count: usize = 8;
                    if (sub_hit as usize) < dim_count {
                        handle_dimension_click(pd, expanded, sub_hit as usize);
                    } else if expanded == 1 {
                        if sub_hit == dim_count as i32 {
                            pd.sort_ascending = true;
                            log::info!("排序: 递增");
                        } else if sub_hit == dim_count as i32 + 1 {
                            pd.sort_ascending = false;
                            log::info!("排序: 递减");
                        }
                    }
                    hide_menu_window();
                } else if main_hit == 0 {
                    log::info!("[预留] 分类框删除: {}", pd.title);
                    hide_menu_window();
                } else if main_hit >= 1 && main_hit <= 3 {
                    // 排序/分组/查看 → 仅展开子菜单，不关闭
                    if pd.menu.expanded_item != main_hit {
                        pd.menu.expanded_item = main_hit;
                        pd.menu.sub_hovered = -1;
                        resize_menu_for_sub(hwnd);
                    }
                    render_menu(hwnd);
                    let parent_hwnd = get_menu_data(hwnd).parent_hwnd;
                    render_layered_window(parent_hwnd);
                } else {
                    hide_menu_window();
                }
            }
            0
        }

        WM_TIMER => {
            if wparam == MENU_TIMER_COLLAPSE {
                KillTimer(hwnd, MENU_TIMER_COLLAPSE);
                if let Some(pd) = get_parent_data(hwnd) {
                    pd.menu.collapse_timer_id = 0;
                    pd.menu.expanded_item = -1;
                    pd.menu.sub_hovered = -1;
                    resize_menu_for_sub(hwnd);
                    render_menu(hwnd);
                    let parent_hwnd = get_menu_data(hwnd).parent_hwnd;
                    render_layered_window(parent_hwnd);
                }
            }
            0
        }

        WM_ACTIVATE => {
            if wparam == 0 {
                // WA_INACTIVE → 延迟关闭
                PostMessageW(hwnd, WM_USER_SHOW_DESKTOP_MENU + 10, 0, 0);
            }
            0
        }

        x if x == WM_USER_SHOW_DESKTOP_MENU + 10 => {
            hide_menu_window();
            0
        }

        WM_DESTROY => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MenuWindowData;
            if !data_ptr.is_null() {
                let data = Box::from_raw(data_ptr);
                drop(data.d2d_target);
                drop(data.d2d_factory);
                drop(data.dw_factory);
                drop(data.wic_factory);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            MENU_HWND.lock().unwrap().0 = std::ptr::null_mut();
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ===== 辅助：调整菜单窗口尺寸以适应子菜单 =====

fn resize_menu_for_sub(hwnd: HWND) {
    let data = unsafe { get_menu_data(hwnd) };
    let parent_data = unsafe { get_parent_data(hwnd) };
    let (expanded_item, sub_on_left) = parent_data
        .map(|pd| (pd.menu.expanded_item, pd.menu.sub_on_left))
        .unwrap_or((-1, false));

    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };

    let main_w = data.main_w;
    let sub_w = data.sub_w;
    let gap = scale_for_dpi(2, dpi);
    let item_h = data.item_h;

    let main_h = item_h * 4 + data.sep_h + scale_for_dpi(16, dpi);

    let sub_h = if expanded_item >= 1 && expanded_item <= 3 {
        let has_order = expanded_item == 1;
        let dim_count = 8;
        let sub_count = if has_order { dim_count + 2 } else { dim_count };
        item_h * sub_count
            + scale_for_dpi(8, dpi)
            + if has_order { scale_for_dpi(24, dpi) } else { 0 }
    } else {
        0
    };

    let total_w = if expanded_item >= 1 {
        if sub_on_left {
            sub_w + gap + main_w
        } else {
            main_w + gap + sub_w
        }
    } else {
        main_w
    };
    let total_h = main_h.max(sub_h);

    let mut wr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut wr) };

    let new_x = if expanded_item >= 1 && sub_on_left {
        wr.right - total_w
    } else {
        wr.left
    };

    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            new_x,
            wr.top,
            total_w,
            total_h,
            SWP_NOZORDER,
        );
    }
}

// ===== 辅助：处理维度项点击 =====

fn handle_dimension_click(pd: &mut GridWindowData, menu_idx: i32, dim_idx: usize) {
    let sort_modes = [
        SortMode::Name,
        SortMode::Type,
        SortMode::ModifiedTime,
        SortMode::Author,
        SortMode::CreatedTime,
        SortMode::Size,
        SortMode::Tag,
        SortMode::Title,
    ];
    let group_modes = [
        GroupBy::Name,
        GroupBy::Type,
        GroupBy::ModifiedTime,
        GroupBy::Author,
        GroupBy::CreatedTime,
        GroupBy::Size,
        GroupBy::Tag,
        GroupBy::Title,
    ];
    let view_modes = [
        ViewMode::ExtraLarge,
        ViewMode::Large,
        ViewMode::Medium,
        ViewMode::Small,
        ViewMode::List,
        ViewMode::Detail,
        ViewMode::Tiles,
        ViewMode::Content,
    ];

    match menu_idx {
        1 if dim_idx < sort_modes.len() => {
            pd.sort_mode = sort_modes[dim_idx].clone();
            log::info!("排序方式切换为: {:?}", pd.sort_mode);
        }
        2 if dim_idx < group_modes.len() => {
            pd.group_by = group_modes[dim_idx].clone();
            log::info!("分组方式切换为: {:?}", pd.group_by);
        }
        3 if dim_idx < view_modes.len() => {
            pd.view_mode = view_modes[dim_idx].clone();
            log::info!("查看模式切换为: {:?}", pd.view_mode);
        }
        _ => {}
    }
}
