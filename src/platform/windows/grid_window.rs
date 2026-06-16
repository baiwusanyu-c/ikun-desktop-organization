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
use crate::config::app_config::ViewMode;

// ===== Import shared types and functions from new modules =====
use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::shell_menu::{show_desktop_context_menu, show_shell_context_menu};

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
}

/// 格子窗口内部数据（存储在 GWLP_USERDATA）
pub(crate) struct GridWindowData {
    pub(crate) title: String,
    pub(crate) items: Vec<GridFileItem>,
    #[allow(dead_code)]
    pub(crate) grid_id: usize,
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

    // ===== D2D/DirectWrite 设备缓存（窗口级） =====
    pub(crate) d2d_factory: Option<ID2D1Factory>,
    pub(crate) dw_factory: Option<IDWriteFactory>,
    pub(crate) wic_factory: Option<IWICImagingFactory>,
    pub(crate) d2d_target: Option<ID2D1DCRenderTarget>,
}

unsafe impl Send for GridWindowData {}

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
        title_h,
        cell_w,
        cell_h,
        cols,
        icon_size,
        selected_item: -1,
        hovered_item: -1,
        tooltip_hwnd: std::ptr::null_mut(),
        view_mode: params.view_mode,
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
fn recalculate_layout(data: &mut GridWindowData, width: i32, _height: i32, dpi: u32) {
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
fn grid_hit_test(
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
            id: Uuid::new_v4(), // TODO: 创建时分配固定 ID，此处暂用新生成
            name: data.title.clone(),
            position: [x, y],
            size: [w, h],
            locked: false, // TODO: 从窗口数据读取锁定状态
            sort_mode: crate::config::SortMode::Name,
            sort_ascending: true,
            view_mode: data.view_mode.clone(),
            group_by: crate::config::GroupBy::None,
            icon_names,
            style_overrides: None,
        });
    }

    log::info!("收集布局数据: {} 个分类框", layouts.len());
    layouts
}

// ===== 窗口过程 =====

/// 从 HWND 获取 GridWindowData 引用
unsafe fn get_data<'a>(hwnd: HWND) -> Option<&'a mut GridWindowData> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

unsafe extern "system" fn grid_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // ===== WM_PAINT（分层窗口使用 UpdateLayeredWindow，此处仅验证绘制）=====
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let _hdc = BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }

        // ===== Hit Test：标题栏拖拽 + 边缘缩放 =====
        WM_NCHITTEST => {
            let screen_x = get_x_lparam(lparam);
            let screen_y = get_y_lparam(lparam);

            let mut wr = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe { GetWindowRect(hwnd, &mut wr) };
            let dx = screen_x - wr.left;
            let dy = screen_y - wr.top;
            let win_w = wr.right - wr.left;
            let win_h = wr.bottom - wr.top;

            let title_h = get_data(hwnd).map(|d| d.title_h).unwrap_or(40);
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };
            let edge = scale_for_dpi(8, dpi);

            let on_left = dx >= 0 && dx < edge;
            let on_right = dx > win_w - edge && dx <= win_w;
            let on_bottom = dy > win_h - edge && dy <= win_h;

            // 主体区域边缘检测
            if dy >= title_h {
                if on_left && on_bottom {
                    return HTBOTTOMLEFT as LRESULT;
                }
                if on_right && on_bottom {
                    return HTBOTTOMRIGHT as LRESULT;
                }
                if on_left {
                    return HTLEFT as LRESULT;
                }
                if on_right {
                    return HTRIGHT as LRESULT;
                }
                if on_bottom {
                    return HTBOTTOM as LRESULT;
                }
            }

            // 标题栏区域
            if dy >= 0 && dy < title_h {
                let h_edge = scale_for_dpi(8, dpi);
                let on_h_left = dx >= 0 && dx < h_edge;
                let on_h_right = dx > win_w - h_edge && dx <= win_w;
                let on_h_top = dy >= 0 && dy < h_edge;

                if on_h_top && on_h_left {
                    return HTTOPLEFT as LRESULT;
                }
                if on_h_top && on_h_right {
                    return HTTOPRIGHT as LRESULT;
                }
                if on_h_top {
                    return HTTOP as LRESULT;
                }
                if on_h_left {
                    return HTLEFT as LRESULT;
                }
                if on_h_right {
                    return HTRIGHT as LRESULT;
                }
                return HTCAPTION as LRESULT;
            }

            HTCLIENT as LRESULT
        }

        // ===== 光标设置 =====
        WM_SETCURSOR => {
            let hit_test = (lparam & 0xFFFF) as i16 as i32;
            let cursor_id = match hit_test {
                HTLEFT | HTRIGHT => IDC_SIZEWE_ID,
                HTTOP | HTBOTTOM => IDC_SIZENS_ID,
                HTTOPLEFT | HTBOTTOMRIGHT => IDC_SIZENWSE_ID,
                HTTOPRIGHT | HTBOTTOMLEFT => IDC_SIZENESW_ID,
                _ => {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
            };
            let cursor = LoadCursorW(std::ptr::null_mut(), cursor_id as *const u16);
            SetCursor(cursor);
            1
        }

        // ===== 窗口缩放 → 重新计算布局并渲染 =====
        WM_SIZE => {
            if wparam != 0 {
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let mut wr = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetWindowRect(hwnd, &mut wr) };
                let win_w = wr.right - wr.left;
                let win_h = wr.bottom - wr.top;

                if let Some(data) = get_data(hwnd) {
                    recalculate_layout(data, win_w, win_h, dpi);
                }
            }
            render_layered_window(hwnd);
            0
        }

        // ===== DPI 变化（跨显示器移动）=====
        WM_DPICHANGED => {
            let new_dpi = (wparam & 0xFFFF) as u32;
            log::info!("DPI 变化: {} → 重新计算布局", new_dpi);

            let suggested_rect = lparam as *const RECT;
            let new_w = unsafe { (*suggested_rect).right - (*suggested_rect).left };
            let new_h = unsafe { (*suggested_rect).bottom - (*suggested_rect).top };

            if let Some(data) = get_data(hwnd) {
                recalculate_layout(data, new_w, new_h, new_dpi);
            }

            unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    (*suggested_rect).left,
                    (*suggested_rect).top,
                    new_w,
                    new_h,
                    SWP_NOZORDER,
                );
            }

            render_layered_window(hwnd);
            0
        }

        // ===== 最小/最大窗口尺寸 =====
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };
            let title_h = get_data(hwnd)
                .map(|d| d.title_h)
                .unwrap_or(scale_for_dpi(40, dpi));
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.pt_min_track_size = POINT {
                x: scale_for_dpi(200, dpi),
                y: title_h + scale_for_dpi(100, dpi),
            };
            mmi.pt_max_track_size = POINT {
                x: scale_for_dpi(3000, dpi),
                y: scale_for_dpi(3000, dpi),
            };
            0
        }

        // ===== 鼠标移动：hover 效果 + tooltip =====
        WM_MOUSEMOVE => {
            let mx = get_x_lparam(lparam);
            let my = get_y_lparam(lparam);

            let mut need_rerender = false;
            let mut show_tooltip = false;
            let mut tooltip_text = String::new();
            let mut tooltip_x: i32 = 0;
            let mut tooltip_y: i32 = 0;

            if let Some(data) = get_data(hwnd) {
                let mut wr = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetWindowRect(hwnd, &mut wr) };
                let w = wr.right - wr.left;
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let hit = grid_hit_test(data, w, mx, my, dpi);

                // 更新 hover 状态
                if hit != data.hovered_item {
                    data.hovered_item = hit;
                    need_rerender = true;
                }

                // 仅在分类框有选中项时显示 tooltip
                if hit >= 0
                    && data.selected_item >= 0
                    && (hit as usize) < data.items.len()
                    && !data.tooltip_hwnd.is_null()
                {
                    tooltip_text = data.items[hit as usize].display_name.clone();
                    show_tooltip = true;
                    // tooltip 位置：鼠标下方偏移（屏幕坐标）
                    let mut pt = POINT { x: 0, y: 0 };
                    unsafe { GetCursorPos(&mut pt) };
                    tooltip_x = pt.x + 16;
                    tooltip_y = pt.y + 16;
                }
            }

            if need_rerender {
                render_layered_window(hwnd);
            }

            // 更新 tooltip（在 render 之后，避免 data 借用冲突）
            if let Some(data) = get_data(hwnd) {
                let tt = data.tooltip_hwnd;
                if !tt.is_null() {
                    if show_tooltip {
                        let text_wide: Vec<u16> = tooltip_text.encode_utf16().collect();
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
                            lpszText: text_wide.as_ptr(),
                            lParam: 0,
                            lpReserved: std::ptr::null_mut(),
                        };
                        unsafe {
                            SendMessageW(
                                tt,
                                TTM_UPDATETIPTEXTW,
                                0,
                                &mut ti as *mut TOOLINFOW as LPARAM,
                            );
                            SendMessageW(
                                tt,
                                TTM_TRACKPOSITION,
                                0,
                                ((tooltip_x as u32 as LPARAM) & 0xFFFF)
                                    | (((tooltip_y as u32 as LPARAM) & 0xFFFF) << 16),
                            );
                            SendMessageW(
                                tt,
                                TTM_TRACKACTIVATE,
                                1,
                                &mut ti as *mut TOOLINFOW as LPARAM,
                            );
                        }
                    } else {
                        // 隐藏 tooltip
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
                            lpszText: std::ptr::null(),
                            lParam: 0,
                            lpReserved: std::ptr::null_mut(),
                        };
                        unsafe {
                            SendMessageW(
                                tt,
                                TTM_TRACKACTIVATE,
                                0,
                                &mut ti as *mut TOOLINFOW as LPARAM,
                            );
                        }
                    }
                }
            }
            0
        }

        // ===== 鼠标左键：选中项 =====
        WM_LBUTTONDOWN => {
            let click_x = get_x_lparam(lparam);
            let click_y = get_y_lparam(lparam);
            let mut need_rerender = false;

            if let Some(data) = get_data(hwnd) {
                let mut wr = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetWindowRect(hwnd, &mut wr) };
                let w = wr.right - wr.left;
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let hit = grid_hit_test(data, w, click_x, click_y, dpi);
                if hit != data.selected_item {
                    data.selected_item = hit;
                    need_rerender = true;
                }
            }

            if need_rerender {
                render_layered_window(hwnd);
            }
            0
        }

        // ===== 鼠标左键双击：打开文件 =====
        WM_LBUTTONDBLCLK => {
            let click_x = get_x_lparam(lparam);
            let click_y = get_y_lparam(lparam);

            if let Some(data) = get_data(hwnd) {
                let mut wr = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetWindowRect(hwnd, &mut wr) };
                let w = wr.right - wr.left;
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let idx = grid_hit_test(data, w, click_x, click_y, dpi);

                if idx >= 0 && (idx as usize) < data.items.len() {
                    let item = &data.items[idx as usize];
                    let op: Vec<u16> = "open\0".encode_utf16().collect();
                    let path: Vec<u16> = item
                        .file_path
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect();
                    ShellExecuteW(
                        hwnd,
                        op.as_ptr(),
                        path.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        SW_SHOW,
                    );
                    log::info!("双击打开: {}", item.display_name);
                }
            }
            0
        }

        // ===== 鼠标右键：弹出上下文菜单 =====
        WM_RBUTTONUP => {
            let click_x = get_x_lparam(lparam);
            let click_y = get_y_lparam(lparam);

            let mut file_path: Option<String> = None;
            if let Some(data) = get_data(hwnd) {
                let mut wr = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetWindowRect(hwnd, &mut wr) };
                let w = wr.right - wr.left;
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let hit = grid_hit_test(data, w, click_x, click_y, dpi);
                if hit >= 0 {
                    data.selected_item = hit;
                    if (hit as usize) < data.items.len() {
                        file_path = Some(data.items[hit as usize].file_path.clone());
                    }
                    render_layered_window(hwnd);
                }
            }

            if let Some(ref path) = file_path {
                show_shell_context_menu(hwnd, path);
            } else {
                show_desktop_context_menu(hwnd);
            }
            0
        }

        // ===== 窗口销毁 =====
        WM_DESTROY => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let data = Box::from_raw(data_ptr);

                // 释放图标句柄
                for item in &data.items {
                    if !item.icon_handle.is_null() {
                        super::icon_extract::destroy_icon(item.icon_handle);
                    }
                }

                // D2D/DW/WIC 资源通过 Drop 自动释放（windows crate COM 包装器）
                drop(data.d2d_target);
                drop(data.d2d_factory);
                drop(data.dw_factory);
                drop(data.wic_factory);

                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                log::info!("格子窗口 D2D/DW 资源已释放");
            }
            0
        }

        // ===== 默认处理 =====
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
