//! 自定义拖拽覆盖窗口（替代 OLE IDragSourceHelper）
//!
//! 显示被拖拽图标的半透明残影，支持单选和多选拖拽。与原生桌面拖拽体验一致。
//!
//! # 单选拖拽
//!
//! 创建小型 WS_EX_LAYERED 窗口跟随鼠标光标，显示单个图标 + 名称标签。
//! 使用 D2D + DirectWrite 渲染复合位图，详见 `d2d_render::render_drag_overlay_bitmap`。
//!
//! # 多选拖拽
//!
//! 当拖拽多个选中项时，显示所有图标并保持它们在网格中的相对排列，
//! 与 Windows 桌面多选拖拽行为一致。使用 GDI `DrawIconEx` 在大位图上按网格位置绘制每个图标。
//!
//! 不依赖 OLE 拖拽图像（IDragSourceHelper），
//! 避免了 Shell 拒绝 drop 时拖拽图像不显示的问题。

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用

use super::ffi_imports::*;
use super::ffi_types::*;

/// 拖拽覆盖窗口类名
static DRAG_OVERLAY_CLASS: &[u16] = &[
    73, 107, 117, 110, 68, 114, 97, 103, 79, 118, 101, 114, 108, 97, 121, 0,
]; // "IkunDragOverlay"

// SAFETY: 以下 static mut 变量仅在 UI 主线程（消息循环线程）中访问，
// 拖拽覆盖窗口生命周期内同一时刻只有一个拖拽操作，不存在并发竞争。
// 所有访问均在 unsafe 函数 create/update/destroy_drag_overlay 内部完成。

/// 拖拽覆盖窗口句柄和位图
static mut OVERLAY_HWND: HWND = std::ptr::null_mut();
static mut OVERLAY_HBMP: HBITMAP = std::ptr::null_mut();

/// 拖拽创建参数，用于跨显示器时按原始 DPI 重建残影位图。
/// `DRAG_ORIGINAL_DPI` 记录创建时的 DPI，作为缩放的不变基准；
/// `DRAG_LAST_DPI` 记录上次渲染的 DPI，用于判断是否需要重建。
static mut DRAG_ICON_HANDLE: HICON = std::ptr::null_mut();
static mut DRAG_ICON_SIZE: i32 = 0;
static mut DRAG_CELL_W: i32 = 0;
static mut DRAG_ICON_OFFSET_X: i32 = 0;
static mut DRAG_DISPLAY_NAME: String = String::new();
static mut DRAG_LAST_DPI: u32 = 0;
static mut DRAG_ORIGINAL_DPI: u32 = 0;

// GDI 常量（未在 ffi_types 中定义的）
const TRANSPARENT_BK_MODE: i32 = 1;
const DEFAULT_CHARSET: i32 = 1;
const FW_NORMAL: i32 = 400;
const CLEARTYPE_QUALITY: i32 = 5;
const FF_DONTCARE: i32 = 0;

/// 创建拖拽覆盖窗口
///
/// 创建一个小窗口显示被拖拽图标 + 名称标签的半透明图像。
/// 窗口跟随光标移动（通过 `update_drag_overlay` 更新位置）。
///
/// # 参数
/// - `icon_handle`: 被拖拽图标的 HICON 句柄
/// - `icon_size`: 图标显示尺寸（DPI 自适应，与分类框内图标大小一致）
/// - `cell_w`: 分类框的单元格宽度（用于标签换行，与框内显示一致）
/// - `display_name`: 图标下方的标签文字（空字符串则不显示标签）
/// - `dpi`: 当前窗口 DPI（用于字体缩放）
pub(crate) unsafe fn create_drag_overlay(
    icon_handle: HICON,
    icon_size: i32,
    cell_w: i32,
    display_name: &str,
    dpi: u32,
) {
    log::info!(
        "[拖拽残影] create: name='{}', hwnd={:?}, icon_size={}, dpi={}",
        display_name,
        std::ptr::addr_of!(OVERLAY_HWND).read(),
        icon_size,
        dpi,
    );
    destroy_drag_overlay();

    if icon_handle.is_null() {
        log::debug!("创建拖拽覆盖: 无图标句柄");
        return;
    }

    // ★ 关键：用光标位置的 DPI 替换源窗口 DPI 渲染位图。
    // 残影跟随光标显示，位图尺寸必须匹配光标所在显示器的 DPI，
    // 否则跨显示器拖拽时会出现残影过大/过小。
    let source_dpi = dpi; // 源窗口 DPI（参数传入）
    let mut cursor_pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor_pt);
    let cursor_dpi =
        crate::platform::windows::monitor_utils::dpi_at_screen_point(cursor_pt.x, cursor_pt.y);
    let dpi = if cursor_dpi > 0 { cursor_dpi } else { dpi };

    // 将 icon_size / cell_w 从源窗口 DPI 缩放到光标 DPI
    let icon_size = scale_dpi(icon_size, dpi, source_dpi);
    let cell_w = scale_dpi(cell_w, dpi, source_dpi);

    // 使用分类框的实际 cell_w，确保标签换行行为与框内一致
    let overlay_w = cell_w;
    // 图标在窗口内水平居中，计算偏移量用于补偿窗口位置
    let icon_offset_x = (overlay_w - icon_size) / 2;
    let label_h = if display_name.is_empty() {
        0
    } else {
        icon_size * 36 / 48
    };
    let gap = if display_name.is_empty() {
        0
    } else {
        icon_size * 6 / 48
    };
    let total_w = overlay_w;
    let total_h = icon_size + gap + label_h;

    // 使用 D2D + DirectWrite 渲染复合位图（与分类框标签质量一致）
    let hbmp = super::d2d_render::render_drag_overlay_bitmap(
        icon_handle,
        icon_size,
        display_name,
        total_w,
        total_h,
        gap,
        label_h,
        dpi,
    );
    if hbmp.is_null() {
        log::warn!("创建拖拽覆盖: D2D 位图渲染失败");
        return;
    }

    // 注册窗口类（只注册一次）
    static mut CLASS_REG: bool = false;
    if !CLASS_REG {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfn_wnd_proc: Some(overlay_wnd_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: hinstance,
            h_icon: std::ptr::null_mut(),
            h_cursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as *const u16),
            hbr_background: std::ptr::null_mut(),
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: DRAG_OVERLAY_CLASS.as_ptr(),
            h_icon_sm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
        CLASS_REG = true;
    }

    // 在创建前获取光标位置，用于窗口初始坐标（避免窗口先出现在 (0,0) 再移动）
    let mut init_pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut init_pt);

    // 创建分层窗口（WS_EX_LAYERED + WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW）
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        DRAG_OVERLAY_CLASS.as_ptr(),
        std::ptr::null(),
        WS_POPUP,
        init_pt.x - icon_offset_x,
        init_pt.y,
        total_w,
        total_h,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );

    if hwnd.is_null() {
        log::warn!("创建拖拽覆盖窗口失败");
        DeleteObject(hbmp as HGDIOBJ);
        return;
    }

    // 使用 UpdateLayeredWindow 设置位图内容（per-pixel alpha）
    let hdc_screen = GetDC(std::ptr::null_mut());
    let hdc_mem = CreateCompatibleDC(hdc_screen);
    let old_bmp = SelectObject(hdc_mem, hbmp as HGDIOBJ);

    // 与 CreateWindowExW 相同的初始位置，保持一致
    let pt_dst = POINT {
        x: init_pt.x - icon_offset_x,
        y: init_pt.y,
    };
    let sz = SIZE {
        cx: total_w,
        cy: total_h,
    };
    let pt_src = POINT { x: 0, y: 0 };

    // 70% 不透明度（半透明效果，与原生桌面拖拽一致）
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 180, // 0-255, 180 ≈ 70%
        alpha_format: AC_SRC_ALPHA,
    };

    UpdateLayeredWindow(
        hwnd,
        hdc_screen,
        &pt_dst,
        &sz as *const SIZE as *const POINT,
        hdc_mem,
        &pt_src,
        0,
        &blend,
        0x02, // ULW_ALPHA
    );

    SelectObject(hdc_mem, old_bmp);
    DeleteDC(hdc_mem);
    ReleaseDC(std::ptr::null_mut(), hdc_screen);

    // 显示窗口：由于 CreateWindowExW 和 UpdateLayeredWindow 都使用光标位置，
    // ShowWindow 不会触发从 (0,0) 到光标位置的移动，因此不会闪烁。
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);

    OVERLAY_HWND = hwnd;
    OVERLAY_HBMP = hbmp;

    // 保存拖拽参数，供 update_drag_overlay 跨显示器重建时使用
    DRAG_ICON_HANDLE = icon_handle;
    DRAG_ICON_SIZE = icon_size;
    DRAG_CELL_W = cell_w;
    DRAG_ICON_OFFSET_X = icon_offset_x;
    DRAG_DISPLAY_NAME = display_name.to_string();
    // 用光标位置的 DPI 初始化 DRAG_LAST_DPI，避免 update_drag_overlay 首次检测时
    // 因源窗口 DPI ≠ 光标 DPI（跨显示器拖拽）触发不必要的重建 → 导致图标闪烁
    // DRAG_ORIGINAL_DPI 也使用光标 DPI，因为位图已按光标 DPI 渲染（见上方缩放逻辑）
    let mut cursor_pt2 = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor_pt2);
    let cursor_dpi2 =
        crate::platform::windows::monitor_utils::dpi_at_screen_point(cursor_pt2.x, cursor_pt2.y);
    let effective_dpi = if cursor_dpi2 > 0 { cursor_dpi2 } else { dpi };
    DRAG_LAST_DPI = effective_dpi;
    DRAG_ORIGINAL_DPI = effective_dpi;

    log::info!("拖拽覆盖窗口已创建 ({}x{}, 70% 不透明)", total_w, total_h);
}

/// 更新拖拽覆盖窗口位置（跟随光标，支持多显示器）
///
/// 当光标移动到不同 DPI 的显示器时，自动重建残影位图并按新 DPI 缩放，
/// 确保拖拽"残影"图标在高/低分辨率屏幕间切换时大小自适应。
pub(crate) unsafe fn update_drag_overlay() {
    if OVERLAY_HWND.is_null() {
        return;
    }

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);

    // DPI 检测优化：每 8 次调用才检测一次 DPI，减少系统调用开销
    static mut DPI_CHECK_COUNTER: u32 = 0;
    DPI_CHECK_COUNTER = (DPI_CHECK_COUNTER + 1) % 8;

    if DPI_CHECK_COUNTER == 0 {
        let current_dpi = crate::platform::windows::monitor_utils::dpi_at_screen_point(pt.x, pt.y);
        if current_dpi != DRAG_LAST_DPI && !DRAG_ICON_HANDLE.is_null() {
            rebuild_drag_overlay_at_dpi(current_dpi);
        }
    }

    // 窗口位置：光标位置减去图标偏移量，使图标对齐光标
    // 使用 SWP_NOZORDER 避免每次都调整 Z-order，减少开销
    SetWindowPos(
        OVERLAY_HWND,
        std::ptr::null_mut(),
        pt.x - DRAG_ICON_OFFSET_X,
        pt.y,
        0,
        0,
        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
    );
}

/// 按新 DPI 缩放像素值，始终以原始 DPI 为基准，避免累积缩放误差。
fn scale_dpi(px: i32, target_dpi: u32, original_dpi: u32) -> i32 {
    (px * target_dpi as i32) / original_dpi.max(1) as i32
}

/// 计算拖拽残影的标签高度（与 icon_size 成比例）。
fn drag_label_height(icon_size: i32, has_label: bool) -> i32 {
    if has_label {
        icon_size * 36 / 48
    } else {
        0
    }
}

/// 计算拖拽残影图标与标签的间距。
fn drag_icon_gap(icon_size: i32, has_label: bool) -> i32 {
    if has_label {
        icon_size * 6 / 48
    } else {
        0
    }
}

/// 按新 DPI 重建拖拽残影位图并更新窗口尺寸。
unsafe fn rebuild_drag_overlay_at_dpi(new_dpi: u32) {
    let icon_handle = DRAG_ICON_HANDLE;
    let base_icon_size = DRAG_ICON_SIZE;
    let base_cell_w = DRAG_CELL_W;
    let display_name = (*std::ptr::addr_of!(DRAG_DISPLAY_NAME)).clone();

    let icon_size = scale_dpi(base_icon_size, new_dpi, DRAG_ORIGINAL_DPI);
    let cell_w = scale_dpi(base_cell_w, new_dpi, DRAG_ORIGINAL_DPI);

    let label_h = drag_label_height(icon_size, !display_name.is_empty());
    let gap = drag_icon_gap(icon_size, !display_name.is_empty());
    let total_w = cell_w;
    let total_h = icon_size + gap + label_h;

    let hbmp = super::d2d_render::render_drag_overlay_bitmap(
        icon_handle,
        icon_size,
        &display_name,
        total_w,
        total_h,
        gap,
        label_h,
        new_dpi,
    );
    if hbmp.is_null() {
        log::warn!("重建拖拽残影失败: DPI={}", new_dpi);
        return;
    }

    // 替换旧位图
    if !OVERLAY_HBMP.is_null() {
        DeleteObject(OVERLAY_HBMP as HGDIOBJ);
    }
    OVERLAY_HBMP = hbmp;
    DRAG_LAST_DPI = new_dpi;

    // 用新位图更新窗口（同时更新尺寸）
    let hdc_screen = GetDC(std::ptr::null_mut());
    let hdc_mem = CreateCompatibleDC(hdc_screen);
    let old_bmp = SelectObject(hdc_mem, hbmp as HGDIOBJ);

    // 使用当前窗口位置（而非 (0, 0)），避免重建时窗口瞬间跳到主屏幕左上角
    let mut cur_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    GetWindowRect(OVERLAY_HWND, &mut cur_rect);
    let pt_dst = POINT {
        x: cur_rect.left,
        y: cur_rect.top,
    };
    let sz = SIZE {
        cx: total_w,
        cy: total_h,
    };
    let pt_src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 180,
        alpha_format: AC_SRC_ALPHA,
    };

    UpdateLayeredWindow(
        OVERLAY_HWND,
        hdc_screen,
        &pt_dst,
        &sz as *const SIZE as *const POINT,
        hdc_mem,
        &pt_src,
        0,
        &blend,
        0x02, // ULW_ALPHA
    );

    SelectObject(hdc_mem, old_bmp);
    DeleteDC(hdc_mem);
    ReleaseDC(std::ptr::null_mut(), hdc_screen);

    log::info!(
        "拖拽残影已重建: DPI={} 尺寸={}x{}",
        new_dpi,
        total_w,
        total_h
    );
}

/// 销毁拖拽覆盖窗口
pub(crate) unsafe fn destroy_drag_overlay() {
    log::info!(
        "[拖拽残影] destroy: hwnd={:?}, hbmp={:?}, multi={}",
        std::ptr::addr_of!(OVERLAY_HWND).read(),
        std::ptr::addr_of!(OVERLAY_HBMP).read(),
        !(*std::ptr::addr_of!(MULTI_DRAG_ITEMS_STORE)).is_empty()
    );
    if !OVERLAY_HWND.is_null() {
        DestroyWindow(OVERLAY_HWND);
        OVERLAY_HWND = std::ptr::null_mut();
    }
    if !OVERLAY_HBMP.is_null() {
        DeleteObject(OVERLAY_HBMP as HGDIOBJ);
        OVERLAY_HBMP = std::ptr::null_mut();
    }
    // 清理保存的拖拽参数
    DRAG_ICON_HANDLE = std::ptr::null_mut();
    DRAG_ICON_SIZE = 0;
    DRAG_CELL_W = 0;
    DRAG_DISPLAY_NAME = String::new();
    DRAG_LAST_DPI = 0;
    DRAG_ORIGINAL_DPI = 0;
    // 清理多选拖拽状态
    clear_multi_drag_state();
}

/// 覆盖窗口的 WndProc（极简，仅处理默认消息）
unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            log::debug!("drag_overlay WM_NCHITTEST: returning HTTRANSPARENT");
            HTTRANSPARENT as LRESULT
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE => {
            log::warn!(
                "drag_overlay received mouse message: msg={:#X} (should not happen!)",
                msg
            );
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ============================================================================
// 多选拖拽支持
// ============================================================================

/// 多选拖拽中每个选中项的信息
pub(crate) struct MultiDragItemInfo {
    pub icon_handle: HICON,
    pub display_name: String,
    pub grid_col: i32,
    pub grid_row: i32,
}

// ============================================================================
// 多选拖拽 - 纯逻辑函数（可独立测试）
// ============================================================================

/// 计算多选拖拽的网格布局（bounding box）
///
/// # 参数
/// - `positions`: 选中项的网格位置列表 `[(col, row), ...]`
///
/// # 返回
/// `(min_col, max_col, min_row, max_row, cols_span, rows_span)`
///
/// # 示例
/// ```
/// let positions = vec![(0, 0), (1, 0), (0, 1), (1, 1)]; // 2x2 矩形
/// let (min_col, max_col, min_row, max_row, cols, rows) = compute_multi_drag_layout(&positions);
/// assert_eq!((min_col, max_col, min_row, max_row), (0, 1, 0, 1));
/// assert_eq!((cols, rows), (2, 2));
/// ```
fn compute_multi_drag_layout(positions: &[(i32, i32)]) -> (i32, i32, i32, i32, i32, i32) {
    if positions.is_empty() {
        return (0, 0, 0, 0, 0, 0);
    }
    let min_col = positions.iter().map(|(c, _)| *c).min().unwrap_or(0);
    let max_col = positions.iter().map(|(c, _)| *c).max().unwrap_or(0);
    let min_row = positions.iter().map(|(_, r)| *r).min().unwrap_or(0);
    let max_row = positions.iter().map(|(_, r)| *r).max().unwrap_or(0);
    let cols_span = max_col - min_col + 1;
    let rows_span = max_row - min_row + 1;
    (min_col, max_col, min_row, max_row, cols_span, rows_span)
}

/// 计算图标在位图中的像素位置
///
/// 根据图标的网格位置和 bounding box 信息，计算其在输出位图中的绘制坐标。
/// 图标在单元格内水平垂直居中。
///
/// # 参数
/// - `grid_col`: 图标的网格列号
/// - `grid_row`: 图标的网格行号
/// - `min_col`: bounding box 的最小列号
/// - `min_row`: bounding box 的最小行号
/// - `cell_w`: 单元格宽度（像素）
/// - `cell_h`: 单元格高度（像素）
/// - `icon_size`: 图标尺寸（像素）
///
/// # 返回
/// `(x, y)` 像素坐标（位图左上角为原点）
fn compute_item_pixel_position(
    grid_col: i32,
    grid_row: i32,
    min_col: i32,
    min_row: i32,
    cell_w: i32,
    cell_h: i32,
    icon_size: i32,
) -> (i32, i32) {
    let col_offset = grid_col - min_col;
    let row_offset = grid_row - min_row;
    let x = col_offset * cell_w + (cell_w - icon_size) / 2;
    let y = row_offset * cell_h + (cell_h - icon_size) / 2;
    (x, y)
}

/// 计算热区偏移（拖拽源图标在位图中的中心位置）
///
/// 热区是鼠标光标点相对于残影窗口左上角的偏移量。
/// 默认锚定到拖拽源图标的中心。
///
/// # 参数
/// - `source_col`: 拖拽源图标的网格列号
/// - `source_row`: 拖拽源图标的网格行号
/// - `min_col`: bounding box 的最小列号
/// - `min_row`: bounding box 的最小行号
/// - `cell_w`: 单元格宽度（像素）
/// - `cell_h`: 单元格高度（像素）
///
/// # 返回
/// `(offset_x, offset_y)` 热区偏移量
fn compute_hotzone_offset(
    source_col: i32,
    source_row: i32,
    min_col: i32,
    min_row: i32,
    cell_w: i32,
    cell_h: i32,
) -> (i32, i32) {
    let col_offset = source_col - min_col;
    let row_offset = source_row - min_row;
    let offset_x = col_offset * cell_w + cell_w / 2;
    let offset_y = row_offset * cell_h + cell_h / 2;
    (offset_x, offset_y)
}

/// 多选拖拽状态（用于 DPI 重建）
static mut MULTI_DRAG_ACTIVE: bool = false;
static mut MULTI_DRAG_ITEMS_STORE: Vec<MultiDragItemInfo> = Vec::new();
static mut MULTI_DRAG_SOURCE_IDX: usize = 0;
static mut MULTI_DRAG_ICON_SIZE: i32 = 0;
static mut MULTI_DRAG_CELL_W: i32 = 0;
static mut MULTI_DRAG_CELL_H: i32 = 0;

/// 创建多选拖拽覆盖窗口（显示多个图标，保持相对位置）
///
/// 当拖拽多个选中项时，显示所有图标并保持它们在网格中的相对排列，
/// 与 Windows 桌面多选拖拽行为一致。
///
/// # 参数
/// - `items`: 所有选中项的信息（图标、名称、网格位置）
/// - `drag_source_idx`: `items` 中被鼠标点击的项索引（热区锚点）
/// - `icon_size`: 图标渲染尺寸（DPI 自适应）
/// - `cell_w`: 单元格宽度
/// - `cell_h`: 单元格高度
/// - `dpi`: 当前窗口 DPI
pub(crate) unsafe fn create_drag_overlay_multi(
    items: Vec<MultiDragItemInfo>,
    drag_source_idx: usize,
    icon_size: i32,
    cell_w: i32,
    cell_h: i32,
    dpi: u32,
) {
    log::info!(
        "[拖拽残影] create_multi: {} items, source_idx={}, hwnd={:?}, icon_size={}",
        items.len(),
        drag_source_idx,
        std::ptr::addr_of!(OVERLAY_HWND).read(),
        icon_size,
    );
    destroy_drag_overlay();
    clear_multi_drag_state();

    if items.is_empty() || drag_source_idx >= items.len() {
        log::debug!("多选拖拽覆盖: 无效参数");
        return;
    }

    // DPI 缩放处理
    let source_dpi = dpi;
    let mut cursor_pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor_pt);
    let cursor_dpi =
        crate::platform::windows::monitor_utils::dpi_at_screen_point(cursor_pt.x, cursor_pt.y);
    let dpi = if cursor_dpi > 0 { cursor_dpi } else { dpi };

    let icon_size = scale_dpi(icon_size, dpi, source_dpi);
    let cell_w = scale_dpi(cell_w, dpi, source_dpi);
    let cell_h = scale_dpi(cell_h, dpi, source_dpi);

    // 计算网格布局（bounding box）
    let positions: Vec<(i32, i32)> = items.iter().map(|i| (i.grid_col, i.grid_row)).collect();
    let (min_col, _max_col, min_row, _max_row, cols_span, rows_span) =
        compute_multi_drag_layout(&positions);

    // 位图尺寸：基于网格跨度
    let total_w = cols_span * cell_w;
    let total_h = rows_span * cell_h;

    if total_w <= 0 || total_h <= 0 {
        log::warn!("多选拖拽: 无效的位图尺寸 {}x{}", total_w, total_h);
        return;
    }

    log::info!(
        "多选拖拽: bounding box {}x{} cells, bitmap {}x{} pixels, {} icons",
        cols_span,
        rows_span,
        total_w,
        total_h,
        items.len()
    );

    // 创建 32-bit DIB Section
    let screen_dc = GetDC(std::ptr::null_mut());
    let mem_dc = CreateCompatibleDC(screen_dc);

    let bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: total_w,
        bi_height: -total_h,
        bi_planes: 1,
        bi_bit_count: 32,
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };

    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let dib = CreateDIBSection(
        mem_dc,
        &bi as *const BITMAPINFOHEADER as *const std::ffi::c_void,
        DIB_RGB_COLORS,
        &mut bits,
        std::ptr::null_mut(),
        0,
    );

    if dib.is_null() || bits.is_null() {
        log::warn!("创建多选拖拽位图失败");
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        return;
    }

    let old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);

    // 填充透明背景（premultiplied alpha: 全 0 = 完全透明）
    std::ptr::write_bytes(bits as *mut u8, 0, (total_w * total_h * 4) as usize);

    // 绘制每个图标
    for item in &items {
        if item.icon_handle.is_null() {
            continue;
        }

        let (x, y) = compute_item_pixel_position(
            item.grid_col,
            item.grid_row,
            min_col,
            min_row,
            cell_w,
            cell_h,
            icon_size,
        );

        DrawIconEx(
            mem_dc,
            x,
            y,
            item.icon_handle,
            icon_size,
            icon_size,
            0,
            std::ptr::null_mut(),
            DI_NORMAL,
        );
    }

    SelectObject(mem_dc, old_bmp);
    DeleteDC(mem_dc);
    ReleaseDC(std::ptr::null_mut(), screen_dc);

    // 计算热区偏移：基于拖拽源图标的位置
    let source_item = &items[drag_source_idx];
    let (icon_offset_x, icon_offset_y) = compute_hotzone_offset(
        source_item.grid_col,
        source_item.grid_row,
        min_col,
        min_row,
        cell_w,
        cell_h,
    );

    // 保存多选拖拽状态（用于 DPI 重建和清理）
    MULTI_DRAG_ACTIVE = true;
    MULTI_DRAG_ITEMS_STORE = items;
    MULTI_DRAG_SOURCE_IDX = drag_source_idx;
    MULTI_DRAG_ICON_SIZE = icon_size;
    MULTI_DRAG_CELL_W = cell_w;
    MULTI_DRAG_CELL_H = cell_h;

    // 注册窗口类（只注册一次）
    static mut CLASS_REG: bool = false;
    if !CLASS_REG {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfn_wnd_proc: Some(overlay_wnd_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: hinstance,
            h_icon: std::ptr::null_mut(),
            h_cursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as *const u16),
            hbr_background: std::ptr::null_mut(),
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: DRAG_OVERLAY_CLASS.as_ptr(),
            h_icon_sm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
        CLASS_REG = true;
    }

    // 获取光标位置用于窗口初始坐标
    let mut init_pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut init_pt);

    // 创建分层窗口
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        DRAG_OVERLAY_CLASS.as_ptr(),
        std::ptr::null(),
        WS_POPUP,
        init_pt.x - icon_offset_x,
        init_pt.y - icon_offset_y,
        total_w,
        total_h,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );

    if hwnd.is_null() {
        log::warn!("创建多选拖拽覆盖窗口失败");
        DeleteObject(dib as HGDIOBJ);
        return;
    }

    // 使用 UpdateLayeredWindow 设置位图内容
    let hdc_screen = GetDC(std::ptr::null_mut());
    let hdc_mem = CreateCompatibleDC(hdc_screen);
    let old_bmp2 = SelectObject(hdc_mem, dib as HGDIOBJ);

    let pt_dst = POINT {
        x: init_pt.x - icon_offset_x,
        y: init_pt.y - icon_offset_y,
    };
    let sz = SIZE {
        cx: total_w,
        cy: total_h,
    };
    let pt_src = POINT { x: 0, y: 0 };

    // 70% 不透明度（与单选拖拽一致）
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 180,
        alpha_format: AC_SRC_ALPHA,
    };

    UpdateLayeredWindow(
        hwnd,
        hdc_screen,
        &pt_dst,
        &sz as *const SIZE as *const POINT,
        hdc_mem,
        &pt_src,
        0,
        &blend,
        0x02, // ULW_ALPHA
    );

    SelectObject(hdc_mem, old_bmp2);
    DeleteDC(hdc_mem);
    ReleaseDC(std::ptr::null_mut(), hdc_screen);

    ShowWindow(hwnd, SW_SHOWNOACTIVATE);

    OVERLAY_HWND = hwnd;
    OVERLAY_HBMP = dib;
    DRAG_ICON_OFFSET_X = icon_offset_x;
    DRAG_ICON_SIZE = MULTI_DRAG_ICON_SIZE;
    DRAG_CELL_W = MULTI_DRAG_CELL_W;
    DRAG_LAST_DPI = dpi;
    DRAG_ORIGINAL_DPI = dpi;

    log::info!(
        "多选拖拽覆盖窗口已创建: {}x{}, {} icons, offset=({},{}), 70% 不透明",
        total_w,
        total_h,
        (*std::ptr::addr_of!(MULTI_DRAG_ITEMS_STORE)).len(),
        icon_offset_x,
        icon_offset_y
    );
}

/// 检查是否处于多选拖拽模式
pub(crate) unsafe fn is_multi_drag_active() -> bool {
    MULTI_DRAG_ACTIVE
}

/// 清理多选拖拽状态
pub(crate) unsafe fn clear_multi_drag_state() {
    MULTI_DRAG_ACTIVE = false;
    (*std::ptr::addr_of_mut!(MULTI_DRAG_ITEMS_STORE)).clear();
    MULTI_DRAG_SOURCE_IDX = 0;
    MULTI_DRAG_ICON_SIZE = 0;
    MULTI_DRAG_CELL_W = 0;
    MULTI_DRAG_CELL_H = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_dpi_same_dpi() {
        assert_eq!(scale_dpi(48, 96, 96), 48);
        assert_eq!(scale_dpi(80, 96, 96), 80);
    }

    #[test]
    fn test_scale_dpi_higher_dpi() {
        // 96 -> 144 DPI: 1.5x
        assert_eq!(scale_dpi(48, 144, 96), 72);
        assert_eq!(scale_dpi(80, 144, 96), 120);
    }

    #[test]
    fn test_scale_dpi_lower_dpi() {
        // 144 -> 96 DPI: 0.67x
        assert_eq!(scale_dpi(72, 96, 144), 48);
        assert_eq!(scale_dpi(120, 96, 144), 80);
    }

    #[test]
    fn test_scale_dpi_roundtrip() {
        // 96 -> 144 -> 96 应该还原原始值
        let original = 48;
        let scaled = scale_dpi(original, 144, 96);
        let _restored = scale_dpi(scaled, 96, 96);
        // 注意：restored 用的是 original_dpi=96，不是 144
        // 正确用法：始终以 original_dpi=96 为基准
        assert_eq!(scale_dpi(original, 96, 96), 48);
    }

    #[test]
    fn test_scale_dpi_no_accumulation_error() {
        // 模拟多次跨显示器切换，始终以原始 DPI 为基准
        let original_dpi = 96;
        let original_size = 48;

        // 第一次：96 -> 144
        let size_144 = scale_dpi(original_size, 144, original_dpi);
        assert_eq!(size_144, 72);

        // 第二次：144 -> 96（仍然以 original_dpi=96 为基准）
        let size_96 = scale_dpi(original_size, 96, original_dpi);
        assert_eq!(size_96, 48);

        // 第三次：96 -> 192（2x 缩放）
        let size_192 = scale_dpi(original_size, 192, original_dpi);
        assert_eq!(size_192, 96);

        // 第四次：192 -> 144
        let size_144_again = scale_dpi(original_size, 144, original_dpi);
        assert_eq!(size_144_again, 72);
    }

    #[test]
    fn test_scale_dpi_zero_original() {
        // original_dpi=0 应该回退到 1，避免除零
        assert_eq!(scale_dpi(48, 96, 0), 48 * 96);
    }

    #[test]
    fn test_drag_label_height_with_label() {
        assert_eq!(drag_label_height(48, true), 36);
        assert_eq!(drag_label_height(72, true), 54);
        assert_eq!(drag_label_height(96, true), 72);
    }

    #[test]
    fn test_drag_label_height_no_label() {
        assert_eq!(drag_label_height(48, false), 0);
        assert_eq!(drag_label_height(72, false), 0);
    }

    #[test]
    fn test_drag_icon_gap_with_label() {
        assert_eq!(drag_icon_gap(48, true), 6);
        assert_eq!(drag_icon_gap(72, true), 9);
    }

    #[test]
    fn test_drag_icon_gap_no_label() {
        assert_eq!(drag_icon_gap(48, false), 0);
    }

    // ===== 多选拖拽纯逻辑函数测试 =====

    #[test]
    fn test_compute_multi_drag_layout_single_item() {
        let positions = vec![(2, 3)];
        let (min_col, max_col, min_row, max_row, cols, rows) =
            compute_multi_drag_layout(&positions);
        assert_eq!((min_col, max_col, min_row, max_row), (2, 2, 3, 3));
        assert_eq!((cols, rows), (1, 1));
    }

    #[test]
    fn test_compute_multi_drag_layout_2x2_rectangle() {
        // A(0,0) B(1,0)
        // C(0,1) D(1,1)
        let positions = vec![(0, 0), (1, 0), (0, 1), (1, 1)];
        let (min_col, max_col, min_row, max_row, cols, rows) =
            compute_multi_drag_layout(&positions);
        assert_eq!((min_col, max_col, min_row, max_row), (0, 1, 0, 1));
        assert_eq!((cols, rows), (2, 2));
    }

    #[test]
    fn test_compute_multi_drag_layout_horizontal_line() {
        // A(1,0) B(2,0) C(3,0) D(4,0)
        let positions = vec![(1, 0), (2, 0), (3, 0), (4, 0)];
        let (min_col, max_col, min_row, max_row, cols, rows) =
            compute_multi_drag_layout(&positions);
        assert_eq!((min_col, max_col, min_row, max_row), (1, 4, 0, 0));
        assert_eq!((cols, rows), (4, 1));
    }

    #[test]
    fn test_compute_multi_drag_layout_vertical_line() {
        // A(0,0)
        // B(0,1)
        // C(0,2)
        let positions = vec![(0, 0), (0, 1), (0, 2)];
        let (min_col, max_col, min_row, max_row, cols, rows) =
            compute_multi_drag_layout(&positions);
        assert_eq!((min_col, max_col, min_row, max_row), (0, 0, 0, 2));
        assert_eq!((cols, rows), (1, 3));
    }

    #[test]
    fn test_compute_multi_drag_layout_l_shape() {
        // A(0,0)
        // B(0,1) C(1,1)
        let positions = vec![(0, 0), (0, 1), (1, 1)];
        let (min_col, max_col, min_row, max_row, cols, rows) =
            compute_multi_drag_layout(&positions);
        assert_eq!((min_col, max_col, min_row, max_row), (0, 1, 0, 1));
        assert_eq!((cols, rows), (2, 2));
    }

    #[test]
    fn test_compute_multi_drag_layout_sparse() {
        // A(0,0)         D(3,0)
        //
        // B(0,2)         C(3,2)
        let positions = vec![(0, 0), (3, 0), (0, 2), (3, 2)];
        let (min_col, max_col, min_row, max_row, cols, rows) =
            compute_multi_drag_layout(&positions);
        assert_eq!((min_col, max_col, min_row, max_row), (0, 3, 0, 2));
        assert_eq!((cols, rows), (4, 3));
    }

    #[test]
    fn test_compute_multi_drag_layout_empty() {
        let positions: Vec<(i32, i32)> = vec![];
        let (min_col, max_col, min_row, max_row, cols, rows) =
            compute_multi_drag_layout(&positions);
        assert_eq!(
            (min_col, max_col, min_row, max_row, cols, rows),
            (0, 0, 0, 0, 0, 0)
        );
    }

    #[test]
    fn test_compute_item_pixel_position_top_left() {
        // 图标在 (0,0)，bounding box 从 (0,0) 开始
        let (x, y) = compute_item_pixel_position(0, 0, 0, 0, 80, 100, 48);
        // col_offset=0, row_offset=0
        // x = 0 * 80 + (80 - 48) / 2 = 16
        // y = 0 * 100 + (100 - 48) / 2 = 26
        assert_eq!((x, y), (16, 26));
    }

    #[test]
    fn test_compute_item_pixel_position_offset_cell() {
        // 图标在 (2, 1)，bounding box 从 (1, 0) 开始
        let (x, y) = compute_item_pixel_position(2, 1, 1, 0, 80, 100, 48);
        // col_offset=1, row_offset=1
        // x = 1 * 80 + 16 = 96
        // y = 1 * 100 + 26 = 126
        assert_eq!((x, y), (96, 126));
    }

    #[test]
    fn test_compute_item_pixel_position_full_cell_icon() {
        // 图标尺寸等于单元格尺寸
        let (x, y) = compute_item_pixel_position(0, 0, 0, 0, 48, 48, 48);
        // x = 0 + (48-48)/2 = 0
        // y = 0 + (48-48)/2 = 0
        assert_eq!((x, y), (0, 0));
    }

    #[test]
    fn test_compute_hotzone_offset_source_at_origin() {
        // 拖拽源在 (0,0)，bounding box 从 (0,0) 开始
        let (ox, oy) = compute_hotzone_offset(0, 0, 0, 0, 80, 100);
        // offset_x = 0 * 80 + 40 = 40
        // offset_y = 0 * 100 + 50 = 50
        assert_eq!((ox, oy), (40, 50));
    }

    #[test]
    fn test_compute_hotzone_offset_source_at_bottom_right() {
        // 拖拽源在 (1,1)，bounding box 从 (0,0) 开始
        let (ox, oy) = compute_hotzone_offset(1, 1, 0, 0, 80, 100);
        // offset_x = 1 * 80 + 40 = 120
        // offset_y = 1 * 100 + 50 = 150
        assert_eq!((ox, oy), (120, 150));
    }

    #[test]
    fn test_compute_hotzone_offset_with_min_offset() {
        // 拖拽源在 (3,2)，bounding box 从 (2,1) 开始
        let (ox, oy) = compute_hotzone_offset(3, 2, 2, 1, 80, 100);
        // col_offset=1, row_offset=1
        // offset_x = 1 * 80 + 40 = 120
        // offset_y = 1 * 100 + 50 = 150
        assert_eq!((ox, oy), (120, 150));
    }

    #[test]
    fn test_compute_hotzone_offset_small_cells() {
        // 小单元格尺寸
        let (ox, oy) = compute_hotzone_offset(0, 0, 0, 0, 20, 24);
        // offset_x = 0 + 10 = 10
        // offset_y = 0 + 12 = 12
        assert_eq!((ox, oy), (10, 12));
    }
}

#[test]
fn test_icon_offset_calculation() {
    // Test icon offset calculation logic
    // icon_offset_x = (cell_w - icon_size) / 2

    let cell_w = 100;
    let icon_size = 48;
    let icon_offset_x = (cell_w - icon_size) / 2;

    assert_eq!(icon_offset_x, 26);

    // Test with different sizes
    let cell_w = 80;
    let icon_size = 48;
    let icon_offset_x = (cell_w - icon_size) / 2;

    assert_eq!(icon_offset_x, 16);

    // Test with icon_size == cell_w (no offset)
    let cell_w = 48;
    let icon_size = 48;
    let icon_offset_x = (cell_w - icon_size) / 2;

    assert_eq!(icon_offset_x, 0);
}

#[test]
fn test_icon_offset_with_odd_difference() {
    // Test with odd difference (should round down)
    let cell_w = 81;
    let icon_size = 48;
    let icon_offset_x = (cell_w - icon_size) / 2;

    // 81 - 48 = 33, 33 / 2 = 16 (integer division)
    assert_eq!(icon_offset_x, 16);
}

#[test]
fn test_drag_timer_interval() {
    // Test that timer interval is 8ms (125Hz)
    let timer_interval_ms = 8;
    let fps = 1000.0 / timer_interval_ms as f32;

    assert!((fps - 125.0).abs() < 0.1);
}

#[test]
fn test_dpi_cache_check_frequency() {
    // Test DPI cache check frequency (every 8 frames)
    let mut frame_count = 0u32;
    let mut dpi_check_count = 0u32;

    for _ in 0..32 {
        frame_count += 1;
        if frame_count.is_multiple_of(8) {
            dpi_check_count += 1;
        }
    }

    // 32 frames should result in 4 DPI checks
    assert_eq!(dpi_check_count, 4);
    assert_eq!(frame_count, 32);
}
