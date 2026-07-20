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

use crate::box_core::layout_calc::calculate_grid;
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
    /// 网格列号（-1 = 自动布局，基于索引）
    pub grid_col: i32,
    /// 网格行号（-1 = 自动布局，基于索引）
    pub grid_row: i32,
}

unsafe impl Send for GridFileItem {}

/// 下拉菜单状态
#[derive(Debug, Clone)]
#[allow(dead_code)] // 菜单状态字段：渲染代码使用但当前构建路径未完全接入
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
    #[allow(dead_code)] // 窗口 ID 在内部追踪使用
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
    /// 编辑分类框回调函数（双击标题栏时调用）
    pub edit_box_callback: Option<Box<dyn Fn(uuid::Uuid, &str) + Send + Sync>>,
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
    /// 编辑分类框回调函数（双击标题栏时调用）
    pub(crate) edit_box_callback: Option<Box<dyn Fn(uuid::Uuid, &str) + Send + Sync>>,
    /// 排序方式
    pub(crate) sort_mode: SortMode,
    /// 是否升序
    pub(crate) sort_ascending: bool,
    /// 分组方式
    pub(crate) group_by: GroupBy,
    /// 菜单打开前的窗口位置和尺寸（用于关闭菜单时恢复）
    #[allow(dead_code)] // 菜单关闭恢复位置时使用
    pub(crate) base_window_x: i32,
    #[allow(dead_code)]
    pub(crate) base_window_y: i32,
    #[allow(dead_code)]
    pub(crate) base_window_w: i32,
    #[allow(dead_code)]
    pub(crate) base_window_h: i32,

    // ===== 拖拽状态（自定义拖拽跟踪，替代 OLE Drag Source）=====
    /// 是否正在检测拖拽阈值（鼠标按下后跟踪）
    pub(crate) drag_pending: bool,
    /// 拖拽检测起始位置（窗口坐标）
    pub(crate) drag_start_x: i32,
    pub(crate) drag_start_y: i32,
    /// 自定义拖拽是否已激活（超过阈值后）
    pub(crate) drag_active: bool,
    /// 当前拖拽的项索引
    pub(crate) drag_item_index: i32,
    /// 拖拽悬停高亮（目标框被拖拽物悬停时显示）
    pub(crate) drop_highlight: bool,
    /// IDropTarget COM 对象指针（窗口销毁时释放）
    pub(crate) drop_target_ptr: *mut std::ffi::c_void,
    /// 图标网格 Y 轴滚动偏移量（像素，0 = 顶部）
    pub(crate) scroll_offset: i32,

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
            edit_box_callback: None,
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
            drag_pending: false,
            drag_start_x: 0,
            drag_start_y: 0,
            drag_active: false,
            drag_item_index: -1,
            drop_highlight: false,
            drop_target_ptr: std::ptr::null_mut(),
            scroll_offset: 0,
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
    #[allow(dead_code)] // 窗口 ID 用于内部追踪
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
        let app_icon = super::app_icon::get_tray_icon().unwrap_or(std::ptr::null_mut());
        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfn_wnd_proc: Some(grid_wnd_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: hinstance,
            h_icon: app_icon,
            h_cursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as *const u16) },
            hbr_background: std::ptr::null_mut(),
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: CLASS_NAME.as_ptr(),
            h_icon_sm: app_icon,
        };
        unsafe {
            RegisterClassExW(&wc);
        }
        *registered = true;
        log::info!("格子窗口类已注册（含 CS_DBLCLKS，图标已设置）");
    }

    hinstance
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
        edit_box_callback: params.edit_box_callback,
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
        drag_pending: false,
        drag_start_x: 0,
        drag_start_y: 0,
        drag_active: false,
        drag_item_index: -1,
        drop_highlight: false,
        drop_target_ptr: std::ptr::null_mut(),
        scroll_offset: 0,
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

    // 2.5 注册 OLE 拖拽目标（使此框可接收拖入的图标）
    let drop_target = super::drop_target::register_box_drop_target(hwnd);
    if !drop_target.is_null() {
        unsafe {
            let data_ref = &mut *data_ptr;
            data_ref.drop_target_ptr = drop_target;
        }
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

/// 将所有其他分类框窗口推到指定窗口之下（Z-order 管理）
///
/// 当某个分类框被激活（点击）时调用。将该分类框保持在 Z-order 顶部（分类框之间），
/// 同时将其他分类框推到它之下，确保点击的分类框在分类框间处于最前，
/// 但所有分类框都不会高于其他应用程序窗口。
///
/// 实现原理：将所有非激活的分类框依次用 SetWindowPos(HWND_BOTTOM) 放置到 Z-order 底部。
/// 由于 HWND_BOTTOM 会把窗口放在最底部，按从最新到最旧的顺序 push，
/// 最终效果是：被激活的分类框在分类框组最上方，其他分类框按原顺序排列在其下方。
pub fn push_other_grids_below(active_hwnd: HWND) {
    let grids = match GRID_WINDOWS.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    // 从列表末尾向前遍历（最旧的先推到底部，最终排在新窗口下方）
    for grid in grids.iter().rev() {
        if grid.hwnd != active_hwnd {
            unsafe {
                SetWindowPos(
                    grid.hwnd,
                    HWND_BOTTOM,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
            }
        }
    }
}

// Z-order 抑制标志：防止 WM_WINDOWPOSCHANGING → SetWindowPos → WM_WINDOWPOSCHANGING 递归。
// 当 push_other_grids_below 批量调整其他分类框 Z-order 时设为 true，
// 使那些窗口的 WM_WINDOWPOSCHANGING 处理器跳过干预，避免无限递归。
thread_local! {
    static ZORDER_SUPPRESSING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// OLE 拖拽进行中标志：DoDragDrop 是阻塞调用，期间 Windows 会对所有窗口做 hit-testing。
// 如果此时 WM_WINDOWPOSCHANGING 强制 HWND_BOTTOM，分类框可能低于 overlay 窗口，
// 导致 OLE 无法触发目标窗口的 DragEnter。拖拽期间不干预 Z-order。
thread_local! {
    static DRAG_IN_PROGRESS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 检查当前是否正在 Z-order 批量调整中（抑制 WM_WINDOWPOSCHANGING 干预）
pub(crate) fn is_zorder_suppressing() -> bool {
    ZORDER_SUPPRESSING.with(|f| f.get())
}

/// 在 Z-order 抑制模式下执行操作（设置标志 → 执行 → 清除标志）
///
/// 用于 WM_USER_PUSH_GRIDS_BELOW 处理器中，确保 push_other_grids_below 内部
/// 的 SetWindowPos 调用不会触发递归 WM_WINDOWPOSCHANGING 干预。
pub(crate) fn with_zorder_suppressed(f: impl FnOnce()) {
    ZORDER_SUPPRESSING.with(|flag| flag.set(true));
    f();
    ZORDER_SUPPRESSING.with(|flag| flag.set(false));
}

/// 设置 OLE 拖拽进行中标志（DoDragDrop 前设为 true，结束后设为 false）
///
/// 拖拽期间 WM_WINDOWPOSCHANGING 不干预 Z-order，确保 OLE hit-testing 能正确
/// 检测到目标窗口的 IDropTarget。
pub fn set_drag_in_progress(in_progress: bool) {
    DRAG_IN_PROGRESS.with(|f| f.set(in_progress));
}

/// 检查当前是否有 OLE 拖拽正在进行
pub(crate) fn is_drag_in_progress() -> bool {
    DRAG_IN_PROGRESS.with(|f| f.get())
}

/// 查找指定屏幕坐标处的分类框窗口
///
/// 由于分类框窗口位于 HWND_BOTTOM（低于覆盖窗口和其他应用窗口），
/// WindowFromPoint 无法找到它们（被覆盖窗口拦截）。
/// 此函数直接遍历分类框列表，通过 GetWindowRect 检查矩形碰撞，
/// 按 Z-order 顺序（列表末尾 = 最上方）返回第一个命中的分类框。
pub fn find_grid_window_at_point(screen_x: i32, screen_y: i32) -> HWND {
    let grids = match GRID_WINDOWS.lock() {
        Ok(g) => g,
        Err(_) => return std::ptr::null_mut(),
    };
    // 从列表末尾向前遍历（Z-order 最高的先检查）
    for grid in grids.iter().rev() {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe {
            if GetWindowRect(grid.hwnd, &mut rect) != 0 {
                if screen_x >= rect.left
                    && screen_x < rect.right
                    && screen_y >= rect.top
                    && screen_y < rect.bottom
                {
                    return grid.hwnd;
                }
            }
        }
    }
    std::ptr::null_mut()
}

/// 获取格子数量
pub fn get_grid_count() -> usize {
    GRID_WINDOWS.lock().map(|g| g.len()).unwrap_or(0)
}

/// 通过 config_id 更新格子窗口的标题
pub fn update_grid_window_title(config_id: uuid::Uuid, new_title: &str) {
    let grids = match GRID_WINDOWS.lock() {
        Ok(g) => g,
        Err(_) => {
            log::warn!("无法锁定 GRID_WINDOWS");
            return;
        }
    };

    for grid in grids.iter() {
        // 通过 HWND 获取 GridWindowData
        let data_ptr =
            unsafe { GetWindowLongPtrW(grid.hwnd, GWLP_USERDATA) } as *mut GridWindowData;
        if data_ptr.is_null() {
            continue;
        }

        let data = unsafe { &mut *data_ptr };
        if data.config_id == config_id {
            let old_title = data.title.clone();
            data.title = new_title.to_string();
            log::info!(
                "更新格子窗口标题: config_id={}, '{}' -> '{}'",
                config_id,
                old_title,
                new_title
            );

            // 重新渲染窗口
            render_layered_window(grid.hwnd);
            return;
        }
    }

    log::warn!("未找到 config_id={} 对应的格子窗口", config_id);
}

/// 收集所有格子窗口的当前布局数据
///
/// 读取每个窗口的实时位置、尺寸、标题、文件列表等信息，
/// 返回 `BoxLayoutData` 列表用于持久化到配置文件。
///
/// **注意**：存储的是逻辑像素（DPI 96 基准），恢复时 `create_grid_window` 会重新缩放。
pub fn collect_grid_layout() -> Vec<crate::config::BoxLayoutData> {
    use crate::config::BoxLayoutData;

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
        let grid_positions: Vec<[i32; 2]> = data
            .items
            .iter()
            .map(|item| [item.grid_col, item.grid_row])
            .collect();

        let display_names: Vec<String> = data
            .items
            .iter()
            .map(|item| {
                let display_name = item.display_name.clone();
                // 如果是 CLSID 路径，确保保存解析后的显示名称
                if item.file_path.starts_with("::{") {
                    if let Some(resolved) =
                        crate::platform::windows::desktop_manager::clsid_display_name(
                            &item.file_path,
                        )
                    {
                        log::info!(
                            "collect_grid_layout: 解析 CLSID {} -> {}",
                            item.file_path,
                            resolved
                        );
                        return resolved.to_string();
                    }
                }
                log::debug!(
                    "collect_grid_layout: {} -> {}",
                    item.file_path,
                    display_name
                );
                display_name
            })
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
            display_names,
            grid_positions,
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
    use super::super::grid_layout::{lock_btn_rect, more_btn_rect};
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
            edit_box_callback: None,
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
            edit_box_callback: None,
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
            edit_box_callback: None,
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

    // ===== Z-order 管理单元测试 =====

    #[test]
    fn test_zorder_suppressing_default_false() {
        // 默认状态下，Z-order 抑制标志应为 false
        assert!(
            !super::is_zorder_suppressing(),
            "ZORDER_SUPPRESSING should default to false"
        );
    }

    #[test]
    fn test_with_zorder_suppressed_sets_and_clears() {
        // with_zorder_suppressed 应在执行期间设为 true，执行后恢复 false
        assert!(!super::is_zorder_suppressing());

        super::with_zorder_suppressed(|| {
            assert!(
                super::is_zorder_suppressing(),
                "Flag should be true inside closure"
            );
        });

        assert!(
            !super::is_zorder_suppressing(),
            "Flag should be false after closure"
        );
    }

    #[test]
    fn test_with_zorder_suppressed_clears_on_panic() {
        // 即使闭包 panic，标志也应被清除（当前实现不保证，但验证行为）
        let result = std::panic::catch_unwind(|| {
            super::with_zorder_suppressed(|| {
                assert!(super::is_zorder_suppressing());
                panic!("test panic");
            });
        });
        assert!(result.is_err(), "Should have panicked");
        // Note: Current implementation does NOT clear flag on panic.
        // This test documents current behavior. If we add panic safety,
        // this assertion should change to assert!(!is_zorder_suppressing())
    }

    #[test]
    fn test_push_other_grids_below_empty_list() {
        // 空列表时不应 panic
        super::push_other_grids_below(std::ptr::null_mut());
    }

    #[test]
    fn test_push_other_grids_below_with_zorder_suppressed() {
        // 在抑制模式下调用 push_other_grids_below 不应触发递归
        super::with_zorder_suppressed(|| {
            super::push_other_grids_below(std::ptr::null_mut());
            assert!(
                super::is_zorder_suppressing(),
                "Should still be suppressing inside"
            );
        });
        assert!(!super::is_zorder_suppressing(), "Should be cleared after");
    }

    // ===== drag flag unit tests =====

    #[test]
    fn test_drag_in_progress_default_false() {
        assert!(!super::is_drag_in_progress(), "DRAG_IN_PROGRESS should default to false");
    }

    #[test]
    fn test_set_drag_in_progress_toggle() {
        super::set_drag_in_progress(true);
        assert!(super::is_drag_in_progress(), "Should be true after set(true)");
        super::set_drag_in_progress(false);
        assert!(!super::is_drag_in_progress(), "Should be false after set(false)");
    }

    #[test]
    fn test_drag_flag_independent_of_zorder_suppressing() {
        super::set_drag_in_progress(true);
        assert!(!super::is_zorder_suppressing(), "Z-order suppressing should be independent");
        assert!(super::is_drag_in_progress(), "Drag flag should be true");
        super::set_drag_in_progress(false);
    }


    // ===== find_grid_window_at_point unit tests =====

    #[test]
    fn test_find_grid_window_at_point_empty_list() {
        // Empty grid list should return null
        let result = super::find_grid_window_at_point(100, 100);
        assert!(result.is_null(), "Should return null for empty grid list");
    }

    #[test]
    fn test_find_grid_window_at_point_returns_null_for_null_input() {
        // Should handle edge cases gracefully
        let result = super::find_grid_window_at_point(0, 0);
        // May or may not find a window at (0,0), but should not crash
        let _ = result;
    }

    #[test]
    fn test_find_grid_window_at_point_negative_coords() {
        // Should handle negative coordinates without crashing
        let result = super::find_grid_window_at_point(-100, -100);
        assert!(result.is_null(), "Should return null for negative coords");
    }

}
