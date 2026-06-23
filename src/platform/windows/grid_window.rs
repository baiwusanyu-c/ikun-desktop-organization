//! 格子窗口 - 核心组件（Direct2D/DirectWrite 渲染）
//!
//! 每个收纳格子是一个独立的 WS_POPUP 分层窗口。
//! 使用 Direct2D + DirectWrite 渲染到 DIB，通过 UpdateLayeredWindow 提交。
//!
//! 核心优势（相比旧 GDI 方案）：
//! - DirectWrite 原生 per-pixel alpha，文字边缘平滑无锯齿
//! - Direct2D DrawBitmap 线性插值缩放，图标质量高
//! - FillRoundedRectangle 硬件加速圆角矩形
//! - GPU 加速渲染，resize 更流畅
//!
//! 拖动标题栏 = WM_NCHITTEST 返回 HTCAPTION，Windows 原生移动。
//! 边缘缩放 = WM_NCHITTEST 返回 HTLEFT/HTRIGHT/...，Windows 原生缩放。

use anyhow::Result;
use std::sync::Mutex;

// ===== Windows crate D2D/DW/WIC imports =====
use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1Factory};
use windows::Win32::Graphics::DirectWrite::IDWriteFactory;
use windows::Win32::Graphics::Imaging::IWICImagingFactory;

// ===== 业务层导入 =====
use crate::box_core::layout_calc::{calculate_grid, hit_test as layout_hit_test};
use crate::box_core::view_engine::get_view_params;
use crate::config::app_config::{GroupBy, SortMode, ViewMode};

// ===== Import shared types and functions from new modules =====
use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
// ===== 公共数据结构 =====

/// 格子内的文件项
pub struct GridFileItem {
    /// 文件路径（.lnk 或实际路径，用于 ShellExecuteW 打开）
    pub file_path: String,
    /// 显示名称
    pub display_name: String,
    /// 系统图标句柄（调用者负责提供，格子窗口负责释放）
    pub icon_handle: HICON,
}

unsafe impl Send for GridFileItem {}

/// 下拉菜单状态
#[derive(Debug, Clone)]
pub(crate) struct MenuState {
    /// 菜单是否打开
    pub(crate) open: bool,
    /// 当前 hover 的主菜单项索引（0=删除, 1=排序, 2=分组, 3=查看, -1=无）
    pub(crate) hovered_item: i32,
    /// 当前展开的子菜单项索引（-1=无展开）
    pub(crate) expanded_item: i32,
    /// 子菜单中当前 hover 的项索引（-1=无）
    /// 对于排序：0-7=维度, 8=分隔线, 9=递增, 10=递减
    /// 对于分组/查看：0-7 或 0-7
    pub(crate) sub_hovered: i32,
    /// 子菜单是否显示在左侧（true=左侧, false=右侧）
    /// 根据窗口在屏幕上的位置自动决定
    pub(crate) sub_on_left: bool,
    /// 子菜单收起延时定时器 ID（0=无定时器）
    pub(crate) collapse_timer_id: usize,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            open: false,
            hovered_item: -1,
            expanded_item: -1,
            sub_hovered: -1,
            sub_on_left: false,
            collapse_timer_id: 0,
        }
    }
}

/// 格子窗口创建参数
pub struct GridCreateParams {
    pub id: usize,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub items: Vec<GridFileItem>,
    /// 视图模式（决定图标大小和布局，默认 Medium）
    pub view_mode: ViewMode,
    /// 是否锁定（锁定后不可拖拽移动/边缘缩放）
    pub locked: bool,
    /// 排序方式
    pub sort_mode: SortMode,
    /// 是否升序
    pub sort_ascending: bool,
    /// 分组方式
    pub group_by: GroupBy,
    /// 配置持久化 ID（对应 BoxLayoutData.id，删除时按此 ID 精确匹配）
    pub config_id: uuid::Uuid,
}

/// 格子窗口内部数据（存储在 GWLP_USERDATA）
pub(crate) struct GridWindowData {
    pub(crate) title: String,
    pub(crate) items: Vec<GridFileItem>,
    #[allow(dead_code)]
    pub(crate) grid_id: usize,
    /// 配置持久化 ID（对应 BoxLayoutData.id，删除时按此 ID 精确匹配）
    pub(crate) config_id: uuid::Uuid,
    pub(crate) title_h: i32,
    /// 单元格宽度
    pub(crate) cell_w: i32,
    /// 单元格高度
    pub(crate) cell_h: i32,
    /// 列数
    pub(crate) cols: i32,
    /// 图标尺寸
    pub(crate) icon_size: i32,
    /// 当前选中项索引（-1 = 无选中）
    pub(crate) selected_item: i32,
    /// 当前 hover 项索引（-1 = 无 hover）
    pub(crate) hovered_item: i32,
    /// Tooltip 窗口句柄
    pub(crate) tooltip_hwnd: HWND,
    /// 视图模式（决定图标大小、布局参数）
    pub(crate) view_mode: ViewMode,
    /// 是否锁定（锁定后不可拖拽移动/边缘缩放）
    pub(crate) locked: bool,
    /// 锁定按钮是否被鼠标悬停
    pub(crate) lock_hovered: bool,
    /// 当前鼠标悬停的边缘（0=无，1=左，2=右，3=上，4=下）
    pub(crate) hover_edge: i32,
    /// "更多"按钮是否被鼠标悬停
    pub(crate) more_hovered: bool,
    /// 菜单状态
    pub(crate) menu: MenuState,
    /// 排序方式
    pub(crate) sort_mode: SortMode,
    /// 是否升序
    pub(crate) sort_ascending: bool,
    /// 分组方式
    pub(crate) group_by: GroupBy,
    /// 菜单打开前的窗口位置和尺寸（用于关闭菜单时恢复）
    pub(crate) base_window_x: i32,
    pub(crate) base_window_y: i32,
    pub(crate) base_window_w: i32,
    pub(crate) base_window_h: i32,

    // ===== D2D/DirectWrite 设备缓存（窗口级） =====
    pub(crate) d2d_factory: Option<ID2D1Factory>,
    pub(crate) dw_factory: Option<IDWriteFactory>,
    pub(crate) wic_factory: Option<IWICImagingFactory>,
    pub(crate) d2d_target: Option<ID2D1DCRenderTarget>,
}

unsafe impl Send for GridWindowData {}

#[cfg(test)]
impl GridWindowData {
    /// 创建测试用的默认实例（所有字段为初始值）
    pub(crate) fn test_default() -> Self {
        Self {
            title: "测试分类框".to_string(),
            items: Vec::new(),
            grid_id: 0,
            config_id: uuid::Uuid::new_v4(),
            title_h: 40,
            cell_w: 80,
            cell_h: 100,
            cols: 4,
            icon_size: 48,
            selected_item: -1,
            hovered_item: -1,
            tooltip_hwnd: std::ptr::null_mut(),
            view_mode: ViewMode::Medium,
            locked: false,
            lock_hovered: false,
            hover_edge: 0,
            more_hovered: false,
            menu: MenuState::default(),
            sort_mode: SortMode::Name,
            sort_ascending: true,
            group_by: GroupBy::None,
            base_window_x: 100,
            base_window_y: 100,
            base_window_w: 400,
            base_window_h: 600,
            d2d_factory: None,
            dw_factory: None,
            wic_factory: None,
            d2d_target: None,
        }
    }
}

/// 格子窗口句柄包装
pub struct GridWindow {
    pub hwnd: HWND,
    pub id: usize,
}

unsafe impl Send for GridWindow {}

// 全局格子列表
static GRID_WINDOWS: Mutex<Vec<GridWindow>> = Mutex::new(Vec::new());
static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

static CLASS_NAME: &[u16] = &[
    68, 101, 115, 107, 116, 111, 112, 79, 114, 103, 71, 114, 105, 100, 0,
]; // "DesktopOrgGrid"

fn ensure_class_registered() -> HINSTANCE {
    let mut registered = CLASS_REGISTERED.lock().unwrap();
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

    if !*registered {
        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfn_wnd_proc: Some(grid_wnd_proc),
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
        unsafe {
            RegisterClassExW(&wc);
        }
        *registered = true;
        log::info!("格子窗口类已注册（含 CS_DBLCLKS）");
    }

    hinstance
}

/// 获取主屏幕分辨率（物理像素）
pub fn get_screen_resolution() -> (i32, i32) {
    let w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    (w, h)
}

/// 获取当前鼠标所在显示器的工作区域（物理像素虚拟桌面坐标）
///
/// 返回值不做 DPI 转换，直接返回 Windows API 的原始物理像素坐标。
/// 用于 Win32 `SetWindowPos` 定位窗口到指定显示器。
pub fn get_cursor_monitor_work_area_phys() -> (i32, i32, i32, i32) {
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) == 0 {
            return fallback_work_area();
        }

        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        if hmon.is_null() {
            return fallback_work_area();
        }

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

        if GetMonitorInfoW(hmon, &mut mi) != 0 {
            let w = &mi.rcWork;
            (w.left, w.top, w.right - w.left, w.bottom - w.top)
        } else {
            fallback_work_area()
        }
    }
}

/// 通过 Win32 `SetWindowPos` 将指定标题的窗口移动到目标物理像素位置
///
/// 用于修正 egui viewport 在多屏 + DPI 缩放下的定位错误。
pub fn reposition_window_by_title(title: &str, target_x: i32, target_y: i32) -> bool {
    unsafe {
        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let hwnd = FindWindowW(std::ptr::null(), title_wide.as_ptr());
        if hwnd.is_null() {
            log::debug!("reposition: 未找到窗口 '{}'", title);
            return false;
        }

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        GetWindowRect(hwnd, &mut rect);
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;

        // SWP_NOSIZE=1, SWP_NOZORDER=4, SWP_NOACTIVATE=16
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            target_x,
            target_y,
            w,
            h,
            1 | 4 | 16,
        );
        log::info!(
            "reposition: '{}' → ({}, {}) 尺寸 {}x{}",
            title,
            target_x,
            target_y,
            w,
            h
        );
        true
    }
}

/// 回退：主屏幕全屏范围
fn fallback_work_area() -> (i32, i32, i32, i32) {
    let (w, h) = get_screen_resolution();
    (0, 0, w, h)
}

fn create_tooltip_window(parent: HWND) -> HWND {
    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_TAB_CLASSES,
    };
    unsafe { InitCommonControlsEx(&icc) };

    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let tooltip_style = TTS_ALWAYSTIP | TTS_NOPREFIX;
    unsafe {
        CreateWindowExW_tooltip(
            0,
            TOOLTIPS_CLASSW.as_ptr(),
            std::ptr::null(),
            tooltip_style,
            -0x8000, // CW_USEDEFAULT
            -0x8000,
            -0x8000,
            -0x8000,
            parent,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    }
}

/// 创建一个格子窗口
pub fn create_grid_window(params: GridCreateParams) -> Result<HWND> {
    let hinstance = ensure_class_registered();

    // 先用初始 DPI 创建窗口，创建后查询实际 DPI 并重新计算尺寸
    let initial_dpi: u32 = 96;
    let initial_w = scale_for_dpi(params.width, initial_dpi);
    let initial_h = scale_for_dpi(params.height, initial_dpi);

    // 1. 创建主窗口（WS_EX_LAYERED 用于 UpdateLayeredWindow 逐像素 alpha）
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_POPUP | WS_VISIBLE | WS_THICKFRAME,
            params.x,
            params.y,
            initial_w,
            initial_h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    };

    if hwnd.is_null() {
        anyhow::bail!("CreateWindowExW 创建格子窗口失败");
    }

    // 查询实际 DPI 并重新计算尺寸
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        log::warn!("GetDpiForWindow 返回 0，使用默认 DPI 96");
    }
    let dpi = if dpi > 0 { dpi } else { 96 };

    let scaled_w = scale_for_dpi(params.width, dpi);
    let scaled_h = scale_for_dpi(params.height, dpi);
    let scaled_x = scale_for_dpi(params.x, dpi);
    let scaled_y = scale_for_dpi(params.y, dpi);
    let title_h = scale_for_dpi(40, dpi);

    // 通过 view_engine 获取视图参数（参数化，支持 8 种视图模式）
    let view_params = get_view_params(&params.view_mode, dpi);
    let icon_size = view_params.icon_size;

    // 通过 layout_calc 计算网格布局（自动换行，窄窗口 clamp 保护）
    let grid_layout = calculate_grid(scaled_w, title_h, &view_params, dpi);
    let cols = grid_layout.cols;
    let cell_w = grid_layout.cell_w;
    let cell_h = grid_layout.cell_h;

    // 如果 DPI 不是 96，调整窗口位置和尺寸（逻辑像素 → 物理像素）
    if dpi != initial_dpi {
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                scaled_x,
                scaled_y,
                scaled_w,
                scaled_h,
                SWP_NOZORDER,
            );
        }
        log::info!(
            "DPI 自适应: {} -> 位置 ({},{}), 尺寸 {}x{}, title_h={}, icon_size={}, cols={}",
            dpi,
            scaled_x,
            scaled_y,
            scaled_w,
            scaled_h,
            title_h,
            icon_size,
            cols
        );
    }

    // 2. 存储窗口数据（含 D2D/DW 设备缓存）
    let data = Box::new(GridWindowData {
        title: params.title,
        items: params.items,
        grid_id: params.id,
        config_id: params.config_id,
        title_h,
        cell_w,
        cell_h,
        cols,
        icon_size,
        selected_item: -1,
        hovered_item: -1,
        tooltip_hwnd: std::ptr::null_mut(),
        view_mode: params.view_mode,
        locked: params.locked,
        lock_hovered: false,
        hover_edge: 0,
        more_hovered: false,
        menu: MenuState::default(),
        sort_mode: params.sort_mode,
        sort_ascending: params.sort_ascending,
        group_by: params.group_by,
        base_window_x: params.x,
        base_window_y: params.y,
        base_window_w: scaled_w,
        base_window_h: scaled_h,
        d2d_factory: None,
        dw_factory: None,
        wic_factory: None,
        d2d_target: None,
    });
    let data_ptr = Box::into_raw(data);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
    }

    unsafe {
        ShowWindow(hwnd, SW_SHOW);
    }

    // 创建 tooltip 窗口（仅在有选中项时显示）
    let tooltip_hwnd = create_tooltip_window(hwnd);
    if !tooltip_hwnd.is_null() {
        // 设置最大宽度（防止 tooltip 过宽）
        unsafe {
            SendMessageW(tooltip_hwnd, TTM_SETMAXTIPWIDTH, 0, 400);
        }
        // 注册 tooltip 工具（tracking 模式）
        let empty_text: Vec<u16> = "\0".encode_utf16().collect();
        let mut ti = TOOLINFOW {
            cbSize: std::mem::size_of::<TOOLINFOW>() as u32,
            uFlags: TTF_SUBCLASS | TTF_IDISHWND,
            hwnd,
            uId: 0,
            rect: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            hinst: std::ptr::null_mut(),
            lpszText: empty_text.as_ptr(),
            lParam: 0,
            lpReserved: std::ptr::null_mut(),
        };
        unsafe {
            SendMessageW(
                tooltip_hwnd,
                TTM_ADDTOOLW,
                0,
                &mut ti as *mut TOOLINFOW as LPARAM,
            );
        }
    }

    // 更新 tooltip_hwnd 到 data
    unsafe {
        let data_ref = &mut *data_ptr;
        data_ref.tooltip_hwnd = tooltip_hwnd;
    }

    // 3. 首次渲染（使用 Direct2D + UpdateLayeredWindow）
    render_layered_window(hwnd);

    // 4. 注册到全局列表
    if let Ok(mut grids) = GRID_WINDOWS.lock() {
        grids.push(GridWindow {
            hwnd,
            id: params.id,
        });
    }

    log::info!(
        "格子窗口已创建: id={} items={} cols={} dpi={}",
        params.id,
        unsafe { &*data_ptr }.items.len(),
        cols,
        dpi
    );
    Ok(hwnd)
}

/// 根据当前窗口尺寸和 DPI 重新计算网格布局参数
///
/// 使用 view_engine 获取视图参数，layout_calc 计算网格布局。
/// 窗口拖拽缩放时自动调用，保证图标始终可见并自动换行。
pub(crate) fn recalculate_layout(data: &mut GridWindowData, width: i32, _height: i32, dpi: u32) {
    data.title_h = scale_for_dpi(40, dpi);

    // 通过 view_engine 获取当前视图模式的参数（支持 ExtraLarge/Large/Medium/Small 等）
    let view_params = get_view_params(&data.view_mode, dpi);
    data.icon_size = view_params.icon_size;

    // 通过 layout_calc 计算网格（自动 clamp：窄窗口时 cell_w 缩小到 body_w，保证图标不超出可视区域）
    let grid_layout = calculate_grid(width, data.title_h, &view_params, dpi);
    data.cols = grid_layout.cols;
    data.cell_w = grid_layout.cell_w;
    data.cell_h = grid_layout.cell_h;
}

/// 命中测试：根据鼠标位置获取网格项索引
///
/// 委托给 `layout_calc::hit_test`，保证与渲染使用完全相同的布局计算公式，
/// 消除 hover/选中/双击/右键的目标偏移问题。
pub(crate) fn grid_hit_test(
    data: &GridWindowData,
    window_width: i32,
    click_x: i32,
    click_y: i32,
    dpi: u32,
) -> i32 {
    let view_params = get_view_params(&data.view_mode, dpi);
    let layout = calculate_grid(window_width, data.title_h, &view_params, dpi);

    layout_hit_test(&layout, data.items.len(), click_x, click_y)
        .map(|i| i as i32)
        .unwrap_or(-1)
}

/// 计算锁定按钮在窗口坐标中的矩形区域
///
/// 位于标题栏右侧（"更多"按钮左边），用于渲染和命中测试。
/// 返回 (x, y, btn_size) 其中 btn_size 为按钮边长。
pub(super) fn lock_btn_rect(win_w: i32, title_h: i32, dpi: u32) -> (i32, i32, i32) {
    let btn_size = scale_for_dpi(26, dpi);
    // 锁定按钮在"更多"按钮左侧
    let (_, _, more_size) = more_btn_rect(win_w, title_h, dpi);
    let btn_x = win_w - more_size - btn_size - scale_for_dpi(10, dpi);
    let btn_y = (title_h - btn_size) / 2;
    (btn_x, btn_y, btn_size)
}

/// 计算"更多"按钮在窗口坐标中的矩形区域
///
/// 位于锁定按钮右侧，用于渲染和命中测试。
/// 返回 (x, y, btn_size) 其中 btn_size 为按钮边长。
pub(super) fn more_btn_rect(win_w: i32, title_h: i32, dpi: u32) -> (i32, i32, i32) {
    let btn_size = scale_for_dpi(26, dpi);
    // "更多"按钮在最右侧
    let btn_x = win_w - btn_size - scale_for_dpi(6, dpi);
    let btn_y = (title_h - btn_size) / 2;
    (btn_x, btn_y, btn_size)
}

/// 菜单行高（DPI 缩放后）
pub(super) fn menu_row_h(dpi: u32) -> i32 {
    scale_for_dpi(36, dpi)
}

/// 主菜单宽度（DPI 缩放后）
pub(super) fn menu_width(dpi: u32) -> i32 {
    scale_for_dpi(160, dpi)
}

/// 子菜单宽度（DPI 缩放后）
pub(super) fn submenu_width(dpi: u32) -> i32 {
    scale_for_dpi(160, dpi)
}

/// 分隔线高度
pub(super) fn menu_separator_h(dpi: u32) -> i32 {
    scale_for_dpi(1, dpi)
}

/// 主菜单在窗口坐标中的矩形
/// 返回 (x, y, w, h)
pub(super) fn main_menu_rect(win_w: i32, title_h: i32, dpi: u32) -> (i32, i32, i32, i32) {
    let mw = menu_width(dpi);
    let mx = win_w - mw - scale_for_dpi(6, dpi);
    let my = title_h + scale_for_dpi(2, dpi);
    // 4 项 + 1 分隔线 + padding (top 4 + bottom 12)
    let mh = menu_row_h(dpi) * 4 + menu_separator_h(dpi) + scale_for_dpi(16, dpi);
    (mx, my, mw, mh)
}

/// 子菜单在窗口坐标中的矩形
///
/// `on_left` 为 true 时显示在主菜单左侧，否则显示在右侧。
/// 返回 (x, y, w, h)
pub(super) fn sub_menu_rect(
    win_w: i32,
    title_h: i32,
    dpi: u32,
    sub_item_count: i32,
    on_left: bool,
) -> (i32, i32, i32, i32) {
    let (mx, my, mw, _mh) = main_menu_rect(win_w, title_h, dpi);
    let sw = submenu_width(dpi);
    let sx = if on_left {
        mx - sw - scale_for_dpi(2, dpi)
    } else {
        mx + mw + scale_for_dpi(2, dpi)
    };
    let sy = my;
    // padding top(4) + items + padding bottom(12) 确保圆角不裁剪
    let sh = menu_row_h(dpi) * sub_item_count + scale_for_dpi(16, dpi);
    (sx, sy, sw, sh)
}

/// 菜单需要的额外窗口高度
pub(super) fn menu_extra_height(dpi: u32) -> i32 {
    // 4 主项 + 1 分隔线 + padding + 额外空间
    menu_row_h(dpi) * 4 + menu_separator_h(dpi) + scale_for_dpi(32, dpi)
}

/// 销毁单个格子窗口
///
/// 从全局列表中移除并销毁 Win32 窗口。
/// WM_DESTROY 会自动释放 GridWindowData（图标句柄、D2D/DW 资源）。
pub fn destroy_grid_window(hwnd: HWND) {
    // 从全局列表移除
    if let Ok(mut grids) = GRID_WINDOWS.lock() {
        grids.retain(|g| g.hwnd != hwnd);
    }

    // 销毁窗口（触发 WM_DESTROY → 释放 GridWindowData）
    unsafe {
        DestroyWindow(hwnd);
    }
    log::info!("分类框窗口已销毁");
}

/// 销毁所有格子窗口
pub fn destroy_all_grid_windows() {
    if let Ok(mut grids) = GRID_WINDOWS.lock() {
        for grid in grids.drain(..) {
            unsafe {
                DestroyWindow(grid.hwnd);
            }
        }
    }
    log::info!("所有格子窗口已销毁");
}

/// 刷新所有格子窗口（修复 DestroyWindow 导致的分层窗口异常）
///
/// DestroyWindow 会导致同线程其他分层窗口 Z-order 和可见性异常。
/// 修复顺序：先 UpdateLayeredWindow 刷新内容 → 再 SetWindowPos 恢复 Z-order。
pub fn rerender_all_grid_windows() {
    if let Ok(grids) = GRID_WINDOWS.lock() {
        for grid in grids.iter() {
            // 1. 先重绘内容（UpdateLayeredWindow 刷新分层窗口位图）
            render_layered_window(grid.hwnd);
            // 2. 恢复 Z-order + 确保可见（不抢焦点）
            unsafe {
                SetWindowPos(
                    grid.hwnd,
                    std::ptr::null_mut(), // HWND_TOP
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
                ShowWindow(grid.hwnd, SW_SHOWNA);
            }
        }
    }
}

/// 获取格子数量
pub fn get_grid_count() -> usize {
    GRID_WINDOWS.lock().map(|g| g.len()).unwrap_or(0)
}

/// 收集所有格子窗口的当前布局数据
///
/// 读取每个窗口的实时位置、尺寸、标题、文件列表等信息，
/// 返回 `BoxLayoutData` 列表用于持久化到配置文件。
///
/// **注意**：存储的是逻辑像素（DPI 96 基准），恢复时 `create_grid_window` 会重新缩放。
pub fn collect_grid_layout() -> Vec<crate::config::BoxLayoutData> {
    use crate::config::BoxLayoutData;
    use uuid::Uuid;

    let grids = match GRID_WINDOWS.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };

    let mut layouts = Vec::new();

    for grid in grids.iter() {
        // 获取窗口 DPI
        let dpi = unsafe { GetDpiForWindow(grid.hwnd) };
        let dpi = if dpi > 0 { dpi } else { 96 };

        // 读取窗口当前位置和尺寸（物理像素）
        let mut wr = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetWindowRect(grid.hwnd, &mut wr) };

        // 物理像素 → 逻辑像素（DPI 96 基准），避免恢复时双重缩放
        let x = (wr.left as f32 * 96.0) / dpi as f32;
        let y = (wr.top as f32 * 96.0) / dpi as f32;
        let w = ((wr.right - wr.left) as f32 * 96.0) / dpi as f32;
        let h = ((wr.bottom - wr.top) as f32 * 96.0) / dpi as f32;

        // 读取窗口内部数据（标题、文件列表、视图模式）
        let data_ptr =
            unsafe { GetWindowLongPtrW(grid.hwnd, GWLP_USERDATA) } as *mut GridWindowData;
        if data_ptr.is_null() {
            continue;
        }

        let data = unsafe { &*data_ptr };
        let icon_names: Vec<String> = data
            .items
            .iter()
            .map(|item| item.file_path.clone())
            .collect();

        layouts.push(BoxLayoutData {
            id: data.config_id,
            name: data.title.clone(),
            position: [x, y],
            size: [w, h],
            locked: data.locked,
            sort_mode: data.sort_mode.clone(),
            sort_ascending: data.sort_ascending,
            view_mode: data.view_mode.clone(),
            group_by: data.group_by.clone(),
            icon_names,
            style_overrides: None,
        });
    }

    log::info!("收集布局数据: {} 个分类框", layouts.len());
    layouts
}

// ===== 窗口过程（已拆分到 wnd_proc.rs）=====
use super::wnd_proc::grid_wnd_proc;

/// 从 HWND 获取 GridWindowData 引用
pub(crate) unsafe fn get_data<'a>(hwnd: HWND) -> Option<&'a mut GridWindowData> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== lock_btn_rect 计算验证 =====

    #[test]
    fn test_lock_btn_rect_at_96dpi() {
        // DPI 96: scale_for_dpi(x, 96) = x
        // more 按钮在最右: x = 400 - 26 - 6 = 368
        // lock 按钮在 more 左侧: x = 400 - 26 - 26 - 10 = 338... let me compute
        // more_btn_rect(400, 40, 96) = (368, 7, 26)
        // lock_x = 400 - 26 - 26 - 10 = 338
        let win_w = 400;
        let title_h = 40;
        let dpi = 96;
        let (btn_x, btn_y, btn_size) = lock_btn_rect(win_w, title_h, dpi);
        assert_eq!(btn_size, 26);
        // lock 在 more 左侧
        let (more_x, _, more_size) = more_btn_rect(win_w, title_h, dpi);
        assert_eq!(more_x, 368); // more 在最右
        assert!(btn_x < more_x, "锁定按钮应在更多按钮左侧");
        assert_eq!(btn_x, win_w - more_size - btn_size - 10);
        assert_eq!(btn_y, 7);
    }

    #[test]
    fn test_lock_btn_rect_at_144dpi() {
        let win_w = 600;
        let title_h = 60;
        let dpi = 144;
        let (btn_x, btn_y, btn_size) = lock_btn_rect(win_w, title_h, dpi);
        assert_eq!(btn_size, 39);
        let (more_x, _, more_size) = more_btn_rect(win_w, title_h, dpi);
        assert!(btn_x < more_x, "锁定按钮应在更多按钮左侧");
        assert_eq!(btn_y, 10);
    }

    #[test]
    fn test_lock_btn_rect_at_192dpi() {
        let win_w = 800;
        let title_h = 80;
        let dpi = 192;
        let (btn_x, btn_y, btn_size) = lock_btn_rect(win_w, title_h, dpi);
        assert_eq!(btn_size, 52);
        let (more_x, _, more_size) = more_btn_rect(win_w, title_h, dpi);
        assert!(btn_x < more_x, "锁定按钮应在更多按钮左侧");
        assert_eq!(btn_y, 14);
    }

    #[test]
    fn test_lock_btn_rect_narrow_window() {
        // 窄窗口：按钮不应超出窗口右边界
        let win_w = 100;
        let title_h = 40;
        let dpi = 96;
        let (btn_x, _btn_y, btn_size) = lock_btn_rect(win_w, title_h, dpi);
        let (more_x, _more_y, more_size) = more_btn_rect(win_w, title_h, dpi);

        assert!(more_x + more_size <= win_w, "更多按钮不应超出窗口右边界");
        assert!(btn_x >= 0, "锁定按钮 x 坐标不应为负数");
        assert!(btn_x < more_x, "锁定按钮应在更多按钮左侧");
    }

    // ===== GridCreateParams locked 字段 =====

    #[test]
    fn test_grid_create_params_locked_field() {
        let params = GridCreateParams {
            id: 0,
            title: "测试".to_string(),
            x: 0,
            y: 0,
            width: 400,
            height: 600,
            items: Vec::new(),
            view_mode: crate::config::app_config::ViewMode::Medium,
            locked: true,
            sort_mode: crate::config::app_config::SortMode::Name,
            sort_ascending: true,
            group_by: crate::config::app_config::GroupBy::None,
            config_id: uuid::Uuid::new_v4(),
        };
        assert!(params.locked);

        let params2 = GridCreateParams {
            id: 1,
            title: "测试2".to_string(),
            x: 0,
            y: 0,
            width: 400,
            height: 600,
            items: Vec::new(),
            view_mode: crate::config::app_config::ViewMode::Medium,
            locked: false,
            sort_mode: crate::config::app_config::SortMode::Name,
            sort_ascending: true,
            group_by: crate::config::app_config::GroupBy::None,
            config_id: uuid::Uuid::new_v4(),
        };
        assert!(!params2.locked);
    }

    #[test]
    fn test_grid_create_params_config_id_field() {
        let id = uuid::Uuid::new_v4();
        let params = GridCreateParams {
            id: 0,
            title: "测试".to_string(),
            x: 0,
            y: 0,
            width: 400,
            height: 600,
            items: Vec::new(),
            view_mode: crate::config::app_config::ViewMode::Medium,
            locked: false,
            sort_mode: crate::config::app_config::SortMode::Name,
            sort_ascending: true,
            group_by: crate::config::app_config::GroupBy::None,
            config_id: id,
        };
        assert_eq!(params.config_id, id);
        // 每个窗口应有独立的 UUID
        let params2 = GridCreateParams {
            config_id: uuid::Uuid::new_v4(),
            ..params
        };
        assert_ne!(params.config_id, params2.config_id);
    }

    #[test]
    fn test_grid_window_data_config_id_field() {
        let id = uuid::Uuid::new_v4();
        let data = GridWindowData::test_default();
        // test_default 生成的 config_id 应为有效 UUID
        assert_ne!(data.config_id, uuid::Uuid::nil());
        // config_id 应为 v4 UUID
        assert_eq!(data.config_id.get_version_num(), 4);
        let _ = id; // 验证外部 UUID 可用
    }
}
