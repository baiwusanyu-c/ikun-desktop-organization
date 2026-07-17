//! 格子窗口消息处理（wnd_proc）
//!
//! 包含 `grid_wnd_proc` 窗口过程，处理所有 Win32 消息：
//! - WM_NCHITTEST：标题栏拖拽 + 边缘缩放 + 锁定按钮 + hover_edge 同步 + 编辑模式锁定
//! - WM_SETCURSOR：边缘缩放手柄 + 锁定/更多按钮手型 + 编辑模式 I 型光标
//! - WM_SIZE / WM_DPICHANGED：布局重算 + 重绘
//! - WM_MOUSEMOVE：锁定按钮 hover + 编辑模式清除/保存 hover + 图标 hover + tooltip
//! - WM_MOUSELEAVE：重置所有 hover 状态
//! - WM_LBUTTONDOWN：编辑模式清除/保存按钮 + 锁定按钮切换 + 图标选中 + "更多"菜单
//! - WM_LBUTTONDBLCLK：标题栏双击进入名称编辑 + 双击打开文件
//! - WM_CHAR / WM_KEYDOWN：默认处理（编辑已由 GPUI 编辑对话框实现）
//! - WM_IME_COMPOSITION：默认处理
//! - WM_RBUTTONUP：Shell 上下文菜单
//! - WM_DESTROY：释放图标句柄 + D2D/DW/WIC 资源

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use crate::box_core::layout_calc::calculate_grid;
use crate::box_core::view_engine::get_view_params;
use super::grid_layout::{grid_hit_test, lock_btn_rect, more_btn_rect, recalculate_layout, reflow_items};
use super::grid_window::{get_data, GridWindowData};
use super::menu_window::show_menu_window;
use super::shell_menu::{show_desktop_context_menu, show_shell_context_menu};

fn remove_grid_item_if_missing_after_shell_menu(hwnd: HWND, logical_path: &str) {
    if super::icon_hider::icon_exists_for_box(logical_path) {
        return;
    }

    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return;
    };
    let Some(index) = data
        .items
        .iter()
        .position(|item| item.file_path == logical_path)
    else {
        return;
    };

    let removed = data.items.remove(index);
    if !removed.icon_handle.is_null() {
        super::icon_extract::destroy_icon(removed.icon_handle);
    }
    if data.selected_item >= data.items.len() as i32 {
        data.selected_item = -1;
    }
    if data.hovered_item >= data.items.len() as i32 {
        data.hovered_item = -1;
    }
    log::info!(
        "右键菜单操作后图标文件不存在，已从分类框移除: {}",
        removed.display_name
    );
    render_layered_window(hwnd);
    super::drag_handler::save_config();
}
/// 格子窗口消息处理过程
///
/// 处理标题栏拖拽、边缘缩放、锁定按钮、图标选中、双击打开、右键菜单等。
/// 边缘 resize 指示条由 `hover_edge` 驱动，在 WM_NCHITTEST 中与实际 hit test 结果同步更新。
pub(super) unsafe extern "system" fn grid_wnd_proc(
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

        // ===== Hit Test：标题栏拖拽 + 边缘缩放（含锁定状态）=====
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

            let data_opt = get_data(hwnd);
            let title_h = data_opt.as_ref().map(|d| d.title_h).unwrap_or(40);
            let is_locked = data_opt.as_ref().map(|d| d.locked).unwrap_or(false);
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };

            // 锁定按钮区域（标题栏右侧）→ 始终返回 HTCLIENT（可点击，不触发拖拽）
            let (btn_x, btn_y, btn_size) = lock_btn_rect(win_w, title_h, dpi);
            if dy >= btn_y && dy < btn_y + btn_size && dx >= btn_x && dx < btn_x + btn_size {
                if let Some(data) = get_data(hwnd) {
                    if data.hover_edge != 0 {
                        data.hover_edge = 0;
                        render_layered_window(hwnd);
                    }
                }
                return HTCLIENT as LRESULT;
            }

            // "更多"按钮区域 → 始终返回 HTCLIENT
            let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(win_w, title_h, dpi);
            if dy >= mbtn_y && dy < mbtn_y + mbtn_size && dx >= mbtn_x && dx < mbtn_x + mbtn_size {
                if let Some(data) = get_data(hwnd) {
                    if data.hover_edge != 0 {
                        data.hover_edge = 0;
                        render_layered_window(hwnd);
                    }
                }
                return HTCLIENT as LRESULT;
            }

            // 菜单打开时，分类框已锁定，全部返回 HTCLIENT
            if let Some(ref d) = data_opt {
                if d.menu.open {
                    return HTCLIENT as LRESULT;
                }
            }

            // 锁定时禁止拖拽移动和边缘缩放（除锁定按钮外全部返回 HTCLIENT）
            if is_locked {
                if let Some(data) = get_data(hwnd) {
                    if data.hover_edge != 0 {
                        data.hover_edge = 0;
                        render_layered_window(hwnd);
                    }
                }
                return HTCLIENT as LRESULT;
            }

            // 未锁定：正常边缘缩放检测（统一计算 result，最后更新 hover_edge）
            let edge = scale_for_dpi(8, dpi);
            let result: i32;
            let new_edge: i32;

            if dy >= title_h {
                // 主体区域边缘检测
                let on_left = dx >= 0 && dx < edge;
                let on_right = dx > win_w - edge && dx <= win_w;
                let on_bottom = dy > win_h - edge && dy <= win_h;

                if on_left && on_bottom {
                    result = HTBOTTOMLEFT;
                    new_edge = 1;
                } else if on_right && on_bottom {
                    result = HTBOTTOMRIGHT;
                    new_edge = 2;
                } else if on_left {
                    result = HTLEFT;
                    new_edge = 1;
                } else if on_right {
                    result = HTRIGHT;
                    new_edge = 2;
                } else if on_bottom {
                    result = HTBOTTOM;
                    new_edge = 4;
                } else {
                    result = HTCLIENT;
                    new_edge = 0;
                }
            } else if dy >= 0 {
                // 标题栏区域（含顶部边缘缩放）
                let h_edge = scale_for_dpi(8, dpi);
                let on_left = dx >= 0 && dx < h_edge;
                let on_right = dx > win_w - h_edge && dx <= win_w;
                let on_top = dy >= 0 && dy < h_edge;

                if on_top && on_left {
                    result = HTTOPLEFT;
                    new_edge = 1;
                } else if on_top && on_right {
                    result = HTTOPRIGHT;
                    new_edge = 2;
                } else if on_top {
                    result = HTTOP;
                    new_edge = 3;
                } else if on_left {
                    result = HTLEFT;
                    new_edge = 1;
                } else if on_right {
                    result = HTRIGHT;
                    new_edge = 2;
                } else {
                    result = HTCAPTION;
                    new_edge = 0;
                }
            } else {
                result = HTCLIENT;
                new_edge = 0;
            }

            // 更新 hover_edge 并在变化时触发重绘（显示/隐藏边缘指示条）
            if let Some(data) = get_data(hwnd) {
                if data.hover_edge != new_edge {
                    data.hover_edge = new_edge;
                    render_layered_window(hwnd);
                }
            }

            result as LRESULT
        }

        // ===== 光标设置 =====
        WM_SETCURSOR => {
            let hit_test = (lparam & 0xFFFF) as i16 as i32;
            let cursor_id = match hit_test {
                HTLEFT | HTRIGHT => IDC_SIZEWE_ID,
                HTTOP | HTBOTTOM => IDC_SIZENS_ID,
                HTTOPLEFT | HTBOTTOMRIGHT => IDC_SIZENWSE_ID,
                HTTOPRIGHT | HTBOTTOMLEFT => IDC_SIZENESW_ID,
                HTCLIENT => {
                    if let Some(data) = get_data(hwnd) {
                        if data.lock_hovered || data.more_hovered {
                            // 正常模式：按钮上显示手型光标
                            IDC_HAND_ID
                        } else {
                            return DefWindowProcW(hwnd, msg, wparam, lparam);
                        }
                    } else {
                        return DefWindowProcW(hwnd, msg, wparam, lparam);
                    }
                }
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
            // Always recalculate layout on resize (wparam == 0 is SIZE_RESTORED = normal resize)
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

            if win_w > 0 && win_h > 0 {
                if let Some(data) = get_data(hwnd) {
                    // 先重排：把固定位置转回自动布局，保持视觉顺序
                    reflow_items(data);
                    recalculate_layout(data, win_w, win_h, dpi);
                    // Clamp scroll_offset after layout change (content may have reflowed)
                    let view_params = get_view_params(&data.view_mode, dpi);
                    let grid_layout = calculate_grid(win_w, data.title_h, &view_params, dpi);
                    let total_rows = (data.items.len() as i32 + grid_layout.cols - 1) / grid_layout.cols;
                    let total_content_h = total_rows * grid_layout.cell_h;
                    let visible_h = win_h - data.title_h;
                    let max_scroll = (total_content_h - visible_h).max(0);
                    data.scroll_offset = data.scroll_offset.min(max_scroll);
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
                reflow_items(data);
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

        // ===== 鼠标移动：hover 效果 + tooltip + 锁定按钮 + 边缘指示 + 拖拽检测 =====
        WM_MOUSEMOVE => {
            let mx = get_x_lparam(lparam);
            let my = get_y_lparam(lparam);

            // Phase 1: 拖拽阈值检测（超过阈值 → 创建覆盖窗口 + SetCapture）
            if super::drag_handler::check_drag_threshold(hwnd, mx, my) {
                return 0;
            }

            // Phase 2: 拖拽中更新覆盖窗口位置
            if super::drag_handler::update_drag(hwnd) {
                return 0;
            }

            let mut need_rerender = false;
            let mut show_tooltip = false;
            let mut tooltip_text = String::new();
            let mut tooltip_x: i32 = 0;
            let mut tooltip_y: i32 = 0;

            // 注册 WM_MOUSELEAVE 跟踪（每次进入时重新注册）
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut tme);
            }

            if let Some(data) = get_data(hwnd) {
                let mut wr = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe { GetWindowRect(hwnd, &mut wr) };
                let w = wr.right - wr.left;
                let _h = wr.bottom - wr.top;
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };

                // 正常模式：锁定按钮 hover 检测
                let (btn_x, btn_y, btn_size) = lock_btn_rect(w, data.title_h, dpi);
                let over_lock =
                    mx >= btn_x && mx < btn_x + btn_size && my >= btn_y && my < btn_y + btn_size;
                if over_lock != data.lock_hovered {
                    data.lock_hovered = over_lock;
                    need_rerender = true;
                }

                // "更多"按钮 hover 检测
                let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(w, data.title_h, dpi);
                let over_more = mx >= mbtn_x
                    && mx < mbtn_x + mbtn_size
                    && my >= mbtn_y
                    && my < mbtn_y + mbtn_size;
                if over_more != data.more_hovered {
                    data.more_hovered = over_more;
                    need_rerender = true;
                }

                // 边缘 resize hover 已由 WM_NCHITTEST 更新 hover_edge

                // 图标网格 hit test（tooltip + hover 高亮）
                let hit = grid_hit_test(data, w, mx, my, dpi);
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

        // ===== 鼠标左键：编辑模式按钮 + 锁定按钮 + "更多"按钮 + 图标选中 =====
        WM_LBUTTONDOWN => {
            let click_x = get_x_lparam(lparam);
            let click_y = get_y_lparam(lparam);
            let mut need_rerender = false;

            // 获取窗口尺寸和 DPI
            let mut wr = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe { GetWindowRect(hwnd, &mut wr) };
            let win_w = wr.right - wr.left;
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let dpi = if dpi > 0 { dpi } else { 96 };

            // 检查是否点击了锁定按钮
            let lock_clicked = if let Some(data) = get_data(hwnd) {
                let (btn_x, btn_y, btn_size) = lock_btn_rect(win_w, data.title_h, dpi);
                click_x >= btn_x
                    && click_x < btn_x + btn_size
                    && click_y >= btn_y
                    && click_y < btn_y + btn_size
            } else {
                false
            };

            // 检查是否点击了"更多"按钮
            let more_clicked = if let Some(data) = get_data(hwnd) {
                let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(win_w, data.title_h, dpi);
                click_x >= mbtn_x
                    && click_x < mbtn_x + mbtn_size
                    && click_y >= mbtn_y
                    && click_y < mbtn_y + mbtn_size
            } else {
                false
            };

            if lock_clicked {
                // 切换锁定状态
                if let Some(data) = get_data(hwnd) {
                    data.locked = !data.locked;
                    log::info!(
                        "锁定状态切换: {} → {}",
                        if data.locked {
                            "已锁定"
                        } else {
                            "已解锁"
                        },
                        data.title
                    );
                    need_rerender = true;
                }
            } else if more_clicked {
                // 显示自定义分层窗口菜单（阻塞直到用户选择或取消）
                show_menu_window(hwnd);
            } else if let Some(data) = get_data(hwnd) {
                // 普通点击选中网格项
                let hit = grid_hit_test(data, win_w, click_x, click_y, dpi);
                if hit != data.selected_item {
                    data.selected_item = hit;
                    need_rerender = true;
                }
                // 点击图标时开始拖拽检测（锁定状态下不启动）
                if hit >= 0 && !data.locked {
                    data.drag_pending = true;
                    data.drag_start_x = click_x;
                    data.drag_start_y = click_y;
                }
            }

            if need_rerender {
                render_layered_window(hwnd);
            }
            0
        }

        // ===== 鼠标左键双击：标题栏进入名称编辑 / 图标打开文件 =====
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

                // 检查是否双击了标题栏文字区域（非按钮区域）
                if click_y < data.title_h && !data.menu.open {
                    let (btn_x, btn_y, btn_size) = lock_btn_rect(w, data.title_h, dpi);
                    let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(w, data.title_h, dpi);
                    let on_lock = click_x >= btn_x
                        && click_x < btn_x + btn_size
                        && click_y >= btn_y
                        && click_y < btn_y + btn_size;
                    let on_more = click_x >= mbtn_x
                        && click_x < mbtn_x + mbtn_size
                        && click_y >= mbtn_y
                        && click_y < mbtn_y + mbtn_size;

                    if !on_lock && !on_more {
                        // 调用编辑分类框回调函数
                        if let Some(ref callback) = data.edit_box_callback {
                            let config_id = data.config_id;
                            let title = data.title.clone();
                            callback(config_id, &title);
                            log::info!(
                                "触发编辑分类框对话框: config_id={}, title={}",
                                config_id,
                                title
                            );
                        } else {
                            log::warn!("edit_box_callback 未设置，无法打开编辑对话框");
                        }
                        return 0;
                    }
                }

                // 双击图标打开文件
                let idx = grid_hit_test(data, w, click_x, click_y, dpi);

                if idx >= 0 && (idx as usize) < data.items.len() {
                    let item = &data.items[idx as usize];
                    let op: Vec<u16> = "open\0".encode_utf16().collect();
                    let open_path = super::icon_hider::resolve_icon_read_path(&item.file_path)
                        .map(|path| path.to_string_lossy().to_string())
                        .unwrap_or_else(|| item.file_path.clone());
                    let path: Vec<u16> =
                        open_path.encode_utf16().chain(std::iter::once(0)).collect();
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

        // ===== 非客户区双击：标题栏进入名称编辑（未锁定时 HTCAPTION 区域双击）=====
        WM_NCLBUTTONDBLCLK => {
            if wparam as i32 == HTCAPTION {
                // lparam 为屏幕坐标，转换为窗口坐标
                let screen_x = (lparam & 0xFFFF) as i16 as i32;
                let screen_y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

                if let Some(data) = get_data(hwnd) {
                    let mut wr = RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    };
                    unsafe { GetWindowRect(hwnd, &mut wr) };
                    let w = wr.right - wr.left;
                    let click_x = screen_x - wr.left;
                    let click_y = screen_y - wr.top;
                    let dpi = unsafe { GetDpiForWindow(hwnd) };
                    let dpi = if dpi > 0 { dpi } else { 96 };

                    // 排除锁定/更多按钮区域（理论上不会收到 HTCAPTION，双重保险）
                    let (btn_x, btn_y, btn_size) = lock_btn_rect(w, data.title_h, dpi);
                    let (mbtn_x, mbtn_y, mbtn_size) = more_btn_rect(w, data.title_h, dpi);
                    let on_lock = click_x >= btn_x
                        && click_x < btn_x + btn_size
                        && click_y >= btn_y
                        && click_y < btn_y + btn_size;
                    let on_more = click_x >= mbtn_x
                        && click_x < mbtn_x + mbtn_size
                        && click_y >= mbtn_y
                        && click_y < mbtn_y + mbtn_size;

                    if !on_lock && !on_more && !data.menu.open {
                        // 调用编辑分类框回调函数（与 WM_LBUTTONDBLCLK 一致）
                        if let Some(ref callback) = data.edit_box_callback {
                            let config_id = data.config_id;
                            let title = data.title.clone();
                            callback(config_id, &title);
                            log::info!(
                                "WM_NCLBUTTONDBLCLK 触发编辑分类框对话框: config_id={}, title={}",
                                config_id,
                                title
                            );
                        } else {
                            log::warn!("edit_box_callback 未设置，无法打开编辑对话框");
                        }
                        return 0;
                    }
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        // ===== 键盘字符输入：名称编辑模式 =====
        WM_CHAR => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },

        // ===== 键盘按键：名称编辑模式特殊键 =====
        WM_KEYDOWN => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },

        // ===== IME 组合完成：插入中文等 CJK 字符 =====
        WM_IME_COMPOSITION => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },

        // ===== 失去焦点：取消名称编辑 =====
        WM_KILLFOCUS => 0,

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
                remove_grid_item_if_missing_after_shell_menu(hwnd, path);
            } else {
                show_desktop_context_menu(hwnd, false);
                // shell_menu.rs 会在用户点击「创建分类框」时设置 PENDING_CREATE_BOX 标志，
                // app.rs 的 update 循环轮询该标志并打开 egui viewport 对话框。
            }
            0
        }

        // ===== 鼠标滚轮滚动 =====
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) as i16) as i32; // HIWORD(wParam)
            let scroll_amount = 3; // lines per notch
            if let Some(data) = get_data(hwnd) {
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let dpi = if dpi > 0 { dpi } else { 96 };
                let mut wr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                unsafe { GetWindowRect(hwnd, &mut wr) };
                let window_w = wr.right - wr.left;
                let view_params = get_view_params(&data.view_mode, dpi);
                let grid_layout = calculate_grid(window_w, data.title_h, &view_params, dpi);
                let cell_h = grid_layout.cell_h;
                let cols = grid_layout.cols;
                let total_rows = (data.items.len() as i32 + cols - 1) / cols;
                let total_content_h = total_rows * cell_h;
                let window_h = wr.bottom - wr.top;
                let visible_h = window_h - data.title_h;
                let max_scroll = (total_content_h - visible_h).max(0);
                let old_offset = data.scroll_offset;
                data.scroll_offset = (data.scroll_offset - delta * scroll_amount).clamp(0, max_scroll);
                if data.scroll_offset != old_offset {
                    render_layered_window(hwnd);
                }
            }
            0
        }

        // ===== 鼠标离开窗口：重置所有 hover 状态 =====
        WM_MOUSELEAVE => {
            let mut need_rerender = false;
            if let Some(data) = get_data(hwnd) {
                if data.hovered_item != -1 {
                    data.hovered_item = -1;
                    need_rerender = true;
                }
                if data.lock_hovered {
                    data.lock_hovered = false;
                    need_rerender = true;
                }
                if data.more_hovered {
                    data.more_hovered = false;
                    need_rerender = true;
                }
                if data.hover_edge != 0 {
                    data.hover_edge = 0;
                    need_rerender = true;
                }
            }
            if need_rerender {
                render_layered_window(hwnd);
            }
            0
        }

        // ===== 窗口销毁 =====
        WM_DESTROY => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
            if !data_ptr.is_null() {
                let data = Box::from_raw(data_ptr);

                // 注销 OLE 拖拽目标
                if !data.drop_target_ptr.is_null() {
                    super::drop_target::unregister_box_drop_target(hwnd, data.drop_target_ptr);
                }

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
        // ===== 延迟打开菜单（由 PostMessage 触发，用于菜单互斥切换）=====
        WM_USER_OPEN_MENU => {
            show_menu_window(hwnd);
            0
        }

        // ===== 延迟删除分类框（由菜单 PostMessage 触发，在消息循环正常调度时处理）=====
        WM_USER_DELETE_BOX => {
            // 先读取 config_id 和 title（窗口销毁前）
            let (config_id, title) = get_data(hwnd)
                .map(|d| (d.config_id, d.title.clone()))
                .unwrap_or((uuid::Uuid::nil(), String::new()));
            log::info!("删除分类框: '{}' (id={})", title, config_id);

            // 销毁窗口（从全局列表移除 + DestroyWindow）
            super::grid_window::destroy_grid_window(hwnd);

            // 按 UUID 精确删除配置中的一条记录
            match crate::config::AppConfig::load() {
                Ok(mut config) => {
                    let before = config.layout.boxes.len();
                    config.layout.boxes.retain(|b| b.id != config_id);
                    let after = config.layout.boxes.len();
                    if let Err(e) = config.save() {
                        log::warn!("删除分类框后保存配置失败: {}", e);
                    } else {
                        log::info!("已删除分类框 '{}'，配置: {} → {} 个", title, before, after);
                    }
                }
                Err(e) => {
                    log::warn!("加载配置失败，跳过配置更新: {}", e);
                }
            }

            // 刷新桌面覆盖窗口：删除分类框后，其包含的图标变为“非管理”状态，
            // 需要重新收集并显示在覆盖窗口上（纵向优先布局）
            // 还原仓库中不再被任何剩余分类框管理的图标到桌面
            super::icon_hider::restore_unmanaged_vault_icons();

            super::desktop_overlay::request_desktop_overlay_refresh();

            // 修复同模板分层窗口 Z-order 异常（DestroyWindow 副作用）
            super::grid_window::rerender_all_grid_windows();

            0
        }

        // ===== 鼠标左键释放：完成自定义拖拽 / 取消拖拽检测 =====
        WM_LBUTTONUP => {
            // Phase 3: 完成拖拽（检测目标 + 执行移动/移除）
            if super::drag_handler::end_drag(hwnd) {
                return 0;
            }
            // 非拖拽状态：取消拖拽检测
            if let Some(data) = get_data(hwnd) {
                data.drag_pending = false;
            }
            0
        }

        // ===== 拖拽悬停进入：显示高亮 =====
        WM_USER_DRAG_ENTER => {
            super::drag_handler::handle_drag_enter(hwnd);
            0
        }

        // ===== 拖拽悬停离开：清除高亮 =====
        WM_USER_DRAG_LEAVE => {
            super::drag_handler::handle_drag_leave(hwnd);
            0
        }

        // ===== 框→框移动：目标框添加项（由源框 PostMessage 触发）=====
        WM_USER_DRAG_ITEM_ADDED => {
            super::drag_handler::handle_drag_item_added(hwnd);
            0
        }

        // ===== OLE 拖拽放置：桌面→框 或 框→框（由 IDropTarget PostMessage 触发）=====
        WM_USER_DRAG_DROP => {
            super::drag_handler::handle_ole_drop(hwnd);
            0
        }

        // ===== 源框通知：项已被拖出（OLE Drop Target 触发，保留兼容）=====
        WM_USER_DRAG_ITEM_REMOVED => {
            // 自定义拖拽模式下，框→框移动在 end_drag 中直接处理
            // 此处理器保留给 OLE Drop Target 的外部拖拽场景
            if let Some(removal) = super::drop_target::take_pending_source_removal() {
                if let Some(data) = get_data(hwnd) {
                    let idx = removal.item_index;
                    if idx < data.items.len() {
                        let removed = data.items.remove(idx);
                        log::info!("OLE 拖拽移除: '{}'", removed.display_name);
                        if data.selected_item >= data.items.len() as i32 {
                            data.selected_item = -1;
                        }
                        render_layered_window(hwnd);
                    }
                }
            }
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
