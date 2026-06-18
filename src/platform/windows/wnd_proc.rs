//! 格子窗口消息处理（wnd_proc）
//!
//! 包含 `grid_wnd_proc` 窗口过程，处理所有 Win32 消息：
//! - WM_NCHITTEST：标题栏拖拽 + 边缘缩放 + 锁定按钮 + hover_edge 同步
//! - WM_SETCURSOR：边缘缩放手柄 + 锁定按钮手型光标
//! - WM_SIZE / WM_DPICHANGED：布局重算 + 重绘
//! - WM_MOUSEMOVE：锁定按钮 hover + 图标 hover + tooltip + TrackMouseEvent
//! - WM_MOUSELEAVE：重置所有 hover 状态
//! - WM_LBUTTONDOWN：锁定按钮切换 + 图标选中 + "更多"菜单
//! - WM_LBUTTONDBLCLK：双击打开文件
//! - WM_RBUTTONUP：Shell 上下文菜单
//! - WM_DESTROY：释放图标句柄 + D2D/DW/WIC 资源

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::{
    get_data, grid_hit_test, lock_btn_rect, more_btn_rect, recalculate_layout, GridWindowData,
};
use super::menu_window::show_menu_window;
use super::shell_menu::{show_desktop_context_menu, show_shell_context_menu};

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
                    // 检查是否在锁定按钮或"更多"按钮上 → 显示手型光标
                    if get_data(hwnd)
                        .map(|d| d.lock_hovered || d.more_hovered)
                        .unwrap_or(false)
                    {
                        IDC_HAND_ID
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

        // ===== 鼠标移动：hover 效果 + tooltip + 锁定按钮 + 边缘指示 =====
        WM_MOUSEMOVE => {
            let mx = get_x_lparam(lparam);
            let my = get_y_lparam(lparam);

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

                // 锁定按钮 hover 检测
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

        // ===== 鼠标左键：锁定按钮 + "更多"按钮 + 图标选中 =====
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
                // shell_menu.rs 会在用户点击「创建分类框」时设置 PENDING_CREATE_BOX 标志，
                // app.rs 的 update 循环轮询该标志并打开 egui viewport 对话框。
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

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
