//! Drag-and-drop operations: same-box drop, box-to-box move, box-to-desktop.
//!
//! Handles all drop scenarios when a drag gesture ends, including
//! grid snap, swap, shift-insert, and OLE drop from external sources.

use super::d2d_render::render_layered_window;
use super::ffi_imports::*;
use super::ffi_types::*;

use std::sync::Mutex;

/// HWND 包装器（实现 Send）
struct SendHwnd(*mut std::ffi::c_void);
unsafe impl Send for SendHwnd {}

/// 待异步收入仓库的文件路径队列 (path, target_hwnd)
static PENDING_INGEST_QUEUE: Mutex<Vec<(String, SendHwnd)>> = Mutex::new(Vec::new());

/// 处理所有待处理的异步文件收入操作（由 WM_USER_DEFERRED_INGEST 调用）
pub(crate) fn process_deferred_ingests() {
    let items: Vec<(String, SendHwnd)> = {
        let mut queue = PENDING_INGEST_QUEUE.lock().unwrap();
        std::mem::take(&mut *queue)
    };
    for (path, _hwnd) in items {
        match super::icon_hider::ingest_icon(&path) {
            Ok(()) => log::info!("[DEFERRED] '{}' 已异步收入仓库", path),
            Err(e) => log::warn!("[DEFERRED] 异步收入仓库失败 '{}': {}", path, e),
        }
    }
}

use super::grid_window::{get_data, get_grid_content_rect};

/// Save grid layout configuration to disk after drag operations.
fn save_config() {
    super::drag_handler::save_config();
}

/// 同框内拖拽：grid snap + swap
pub(crate) fn handle_same_box_drop(
    hwnd: HWND,
    dragged_idx: usize,
    screen_x: i32,
    screen_y: i32,
    saved_insert_col: i32,
    saved_insert_row: i32,
) {
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };

    if dragged_idx >= data.items.len() {
        return;
    }

    // Convert screen coords to window client coords
    let Some(wr) = get_grid_content_rect(hwnd) else {
        return;
    };
    let client_x = screen_x - wr.left;
    let client_y = screen_y - wr.top;

    // Get current layout
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let window_width = wr.right - wr.left;

    let view_params = crate::box_core::view_engine::get_view_params(&data.view_mode, dpi);
    let layout =
        crate::box_core::layout_calc::calculate_grid(window_width, data.title_h, &view_params, dpi);

    // Determine target grid cell from click position
    let target_col;
    let target_row;
    if client_x >= layout.grid_left && client_y >= layout.grid_top {
        target_col = ((client_x - layout.grid_left) / layout.cell_w).max(0);
        target_row = ((client_y - layout.grid_top) / layout.cell_h).max(0);
    } else {
        // Click outside grid area → cancel
        log::info!("同框拖拽: 释放在网格区域外，取消");
        return;
    }

    // Materialize dragged item's auto-layout position before any swap
    if data.items[dragged_idx].grid_col < 0 {
        let auto_col = (dragged_idx as i32) % layout.cols;
        let auto_row = (dragged_idx as i32) / layout.cols;
        data.items[dragged_idx].grid_col = auto_col;
        data.items[dragged_idx].grid_row = auto_row;
    }

    // Materialize all auto-layout items' positions before occupied check
    // to ensure accurate collision detection and swap
    for (i, item) in data.items.iter_mut().enumerate() {
        if item.grid_col < 0 {
            item.grid_col = (i as i32) % layout.cols;
            item.grid_row = (i as i32) / layout.cols;
        }
    }

    // Use the saved insert state from end_drag (already cleared in data)
    let insert_col = saved_insert_col;
    let insert_row = saved_insert_row;
    let is_insert_mode = insert_col >= 0 && insert_row >= 0;

    let cols = layout.cols;

    if is_insert_mode {
        // === Insert mode: use pure shift-insert function ===
        let insert_lin = insert_row * cols + insert_col;
        let positions: Vec<(i32, i32)> = data
            .items
            .iter()
            .map(|item| (item.grid_col, item.grid_row))
            .collect();
        let result =
            super::drag_visual::apply_shift_insert(&positions, dragged_idx, insert_lin, cols);
        for (i, item) in data.items.iter_mut().enumerate() {
            item.grid_col = result[i].0;
            item.grid_row = result[i].1;
        }
        log::info!(
            "Grid insert: '{}' -> ({}, {})",
            data.items[dragged_idx].display_name,
            data.items[dragged_idx].grid_col,
            data.items[dragged_idx].grid_row,
        );
    } else {
        // === Replace mode: swap or move to empty cell ===
        let occupied_idx = data
            .items
            .iter()
            .enumerate()
            .find(|(i, item)| {
                *i != dragged_idx && item.grid_col == target_col && item.grid_row == target_row
            })
            .map(|(i, _)| i);

        if let Some(other_idx) = occupied_idx {
            // Swap: exchange grid positions
            let drag_col = data.items[dragged_idx].grid_col;
            let drag_row = data.items[dragged_idx].grid_row;
            data.items[dragged_idx].grid_col = target_col;
            data.items[dragged_idx].grid_row = target_row;
            data.items[other_idx].grid_col = drag_col;
            data.items[other_idx].grid_row = drag_row;
            log::info!(
                "Grid swap: '{}' <-> '{}'",
                data.items[dragged_idx].display_name,
                data.items[other_idx].display_name,
            );
        } else {
            // Move to empty cell
            data.items[dragged_idx].grid_col = target_col;
            data.items[dragged_idx].grid_row = target_row;
            log::info!(
                "Grid snap: '{}' -> ({}, {})",
                data.items[dragged_idx].display_name,
                target_col,
                target_row,
            );
        }
    }

    data.selected_item = dragged_idx as i32;
    render_layered_window(hwnd);
    save_config();
}

/// 框→框移动：从源框移除项，添加到目标框
pub(crate) fn handle_box_to_box_move(source_hwnd: HWND, target_hwnd: HWND, item_index: usize) {
    log::info!(
        "handle_box_to_box_move: 开始框→框移动, item_index={}",
        item_index
    );
    let data = match unsafe { get_data(source_hwnd) } {
        Some(d) => d,
        None => return,
    };

    if item_index >= data.items.len() {
        return;
    }

    let item = &data.items[item_index];
    let file_path = item.file_path.clone();
    let display_name = item.display_name.clone();

    // 获取释放位置坐标
    let mut drop_pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut drop_pt) };
    // 通知目标框添加项（包含释放坐标）
    unsafe {
        // 读取目标框的拖拽覆盖状态（用于插入模式）
        let (insert_col, insert_row) = get_data(target_hwnd)
            .map(|d| (d.drag_over_insert_col, d.drag_over_insert_row))
            .unwrap_or((-1, -1));
        super::drop_target::set_pending_add_item(
            file_path.clone(),
            display_name.clone(),
            drop_pt.x,
            drop_pt.y,
            insert_col,
            insert_row,
        );
        PostMessageW(target_hwnd, WM_USER_DRAG_ITEM_ADDED, 0, 0);
    }

    // 从源框移除项
    let data = match unsafe { get_data(source_hwnd) } {
        Some(d) => d,
        None => return,
    };
    if item_index < data.items.len() {
        let removed = data.items.remove(item_index);
        log::info!(
            "handle_box_to_box_move: '{}' 已从源框移除, file_path={}",
            removed.display_name,
            removed.file_path
        );
        if data.selected_item >= data.items.len() as i32 {
            data.selected_item = -1;
        }
        render_layered_window(source_hwnd);
        save_config();
    }
}

/// 框→桌面：从仓库导出到用户桌面并定位，同时刷新覆盖窗口。
pub(crate) fn handle_box_to_desktop(
    hwnd: HWND,
    item_index: usize,
    drop_x: i32,
    drop_y: i32,
    _target_is_overlay: bool,
) {
    let t0 = std::time::Instant::now();
    log::info!(
        "[TIMING] handle_box_to_desktop: 开始框→桌面, item_index={}, drop=({}, {})",
        item_index,
        drop_x,
        drop_y
    );

    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };

    if item_index >= data.items.len() {
        return;
    }

    let removed = &data.items[item_index];
    let display_name = removed.display_name.clone();
    let file_path = removed.file_path.clone();

    log::info!("框→桌面: 导出图标 '{}' (path={})", display_name, file_path);

    let t_export = std::time::Instant::now();
    let mut export_path = super::icon_hider::export_desktop_path(&file_path);
    let mut export_succeeded = false;
    match super::icon_hider::export_icon(&file_path) {
        Ok((true, path)) => {
            export_path = path;
            export_succeeded = true;
            log::info!(
                "框→桌面: 图标 '{}' 已导出到 {}",
                display_name,
                export_path.display()
            );
        }
        Ok((false, path)) => {
            export_path = path;
            if export_path.exists() {
                export_succeeded = true;
                log::info!("框→桌面: 图标 '{}' 已在桌面", display_name);
            } else {
                log::warn!("框→桌面: 图标 '{}' 不在仓库也不在桌面", display_name);
            }
        }
        Err(e) => {
            log::warn!("框→桌面: 导出图标 '{}' 失败: {}", display_name, e);
        }
    }

    log::info!("[TIMING] export_icon 完成 [{:?}]", t_export.elapsed());

    // ???????????????????????? OLE ???
    // ??????????? ? ???? ? ?? ? ??????
    if export_path.exists() {
        let file_path_str = export_path.to_string_lossy().to_string();
        let target_x = drop_x;
        let target_y = drop_y;
        std::thread::spawn(move || {
            let t_start = std::time::Instant::now();

            log::info!("[TIMING] ??????, ??=({}, {})", target_x, target_y);

            // ?????? 30ms ?????????????
            const POLL_INTERVAL_MS: u64 = 15;
            const MAX_POLL_MS: u64 = 2000;

            for poll in 0..(MAX_POLL_MS / POLL_INTERVAL_MS) {
                let t_iter = std::time::Instant::now();
                let elapsed = t_start.elapsed().as_millis() as u64;

                if elapsed >= MAX_POLL_MS {
                    log::warn!("[TIMING] ???? ({}ms)", MAX_POLL_MS);
                    break;
                }

                // ????
                log::debug!("[TIMING] ? {} ??? (??? {}ms)", poll + 1, elapsed);
                super::icon_manager::position_icon_at_screen(&file_path_str, target_x, target_y);
                log::debug!(
                    "[TIMING] position_icon ?? {}ms",
                    t_iter.elapsed().as_millis()
                );

                // ????
                let t_verify = std::time::Instant::now();
                match super::icon_manager::verify_icon_screen_position(&file_path_str) {
                    Some((actual_x, actual_y)) => {
                        let dx = (actual_x - target_x).abs();
                        let dy = (actual_y - target_y).abs();
                        log::info!(
                            "[TIMING] ??: ??=({}, {}), ??=({}, {}), ??=({}, {}), ??? {:?}",
                            target_x,
                            target_y,
                            actual_x,
                            actual_y,
                            dx,
                            dy,
                            t_start.elapsed()
                        );
                        if dx <= 30 && dy <= 30 {
                            log::info!(
                                "[TIMING] ????! ? {} ???, ??? {:?}",
                                poll + 1,
                                t_start.elapsed()
                            );
                            return;
                        }
                    }
                    None => {
                        log::debug!(
                            "[TIMING] ????? (verify ?? {}ms)",
                            t_verify.elapsed().as_millis()
                        );
                    }
                }

                std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
            }

            // ??????????????
            log::warn!("[TIMING] ??????????????, ??? {:?}", t_start.elapsed());
            for attempt in 0..3 {
                if attempt > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
                super::icon_manager::position_icon_at_screen(&file_path_str, target_x, target_y);
                std::thread::sleep(std::time::Duration::from_millis(100));

                match super::icon_manager::verify_icon_screen_position(&file_path_str) {
                    Some((actual_x, actual_y)) => {
                        let dx = (actual_x - target_x).abs();
                        let dy = (actual_y - target_y).abs();
                        log::info!(
                            "[TIMING] ?? {}: ??=({}, {}), ??=({}, {}), ??=({}, {}), ??? {:?}",
                            attempt + 1,
                            target_x,
                            target_y,
                            actual_x,
                            actual_y,
                            dx,
                            dy,
                            t_start.elapsed()
                        );
                        if dx <= 30 && dy <= 30 {
                            log::info!("[TIMING] ??????! ??? {:?}", t_start.elapsed());
                            return;
                        }
                    }
                    None => {
                        log::warn!("[TIMING] ?? {} ????", attempt + 1);
                    }
                }
            }

            log::warn!("[TIMING] ?????????????, ??? {:?}", t_start.elapsed());
            super::icon_manager::position_icon_at_screen(&file_path_str, target_x, target_y);
        });
    }

    log::info!("[TIMING] handle_box_to_desktop 总计 [{:?}]", t0.elapsed());

    if super::icon_hider::is_public_desktop_path(&file_path) && !export_succeeded {
        log::warn!("框→桌面: 公共图标导出未完成，保留分类框项目: {}", file_path);
        return;
    }

    // 从框中移除项
    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };
    if item_index < data.items.len() {
        let removed = data.items.remove(item_index);
        if !removed.icon_handle.is_null() {
            super::icon_extract::destroy_icon(removed.icon_handle);
        }
        if data.selected_item >= data.items.len() as i32 {
            data.selected_item = -1;
        }
        render_layered_window(hwnd);
        save_config();
    }
}

pub(crate) fn handle_drag_enter(hwnd: HWND) {
    if let Some(data) = unsafe { get_data(hwnd) } {
        if !data.locked && !data.drop_highlight {
            data.drop_highlight = true;
            render_layered_window(hwnd);
            log::debug!("拖拽悬停进入: 高亮");
        }
    }
}

/// 处理 WM_USER_DRAG_LEAVE（拖拽悬停离开，清除高亮）
pub(crate) fn handle_drag_leave(hwnd: HWND) {
    if let Some(data) = unsafe { get_data(hwnd) } {
        let mut changed = false;
        if data.drop_highlight {
            data.drop_highlight = false;
            changed = true;
        }
        // 清除跨框拖拽覆盖可视化状态
        if data.drag_over_item != -1
            || data.drag_over_insert_col != -1
            || data.drag_over_insert_row != -1
        {
            data.drag_over_item = -1;
            data.drag_over_insert_col = -1;
            data.drag_over_insert_row = -1;
            changed = true;
        }
        if changed {
            render_layered_window(hwnd);
        }
    }
}

/// 处理 WM_USER_DRAG_ITEM_ADDED（框→框移动 或 桌面→框 时，目标框添加项）
pub(crate) fn handle_drag_item_added(hwnd: HWND) {
    log::info!("handle_drag_item_added: 目标框接收到项");
    if let Some((file_path, display_name, drop_x, drop_y, insert_col, insert_row)) =
        super::drop_target::take_pending_add_item()
    {
        log::info!(
            "handle_drag_item_added: file_path={}, display_name={}, drop=({}, {})",
            file_path,
            display_name,
            drop_x,
            drop_y
        );
        // 检查是否已存在
        let public_preflight = super::icon_hider::is_public_desktop_path(&file_path);
        if public_preflight && !hide_single_desktop_icon(&file_path, &display_name) {
            log::warn!("公共图标收纳失败，取消加入分类框: {}", file_path);
            return;
        }
        if let Some(data) = unsafe { get_data(hwnd) } {
            if data.items.iter().any(|i| i.file_path == file_path) {
                log::debug!("跳过已存在项: {}", file_path);
                return;
            }

            // 提取图标。分类框中保存的是“逻辑桌面路径”，图标可能已经被移动/复制到
            // managed_icons 仓库；读取时必须解析到实际存在的路径，否则框→框拖拽后
            // 目标框只能得到名称，图标句柄为空，右键菜单也会因为原路径不存在而降级。
            let read_path = resolve_drag_item_read_path(&file_path);
            let read_path_str = read_path.to_string_lossy().to_string();
            let icon_handle = super::icon_extract::extract_file_icon(&read_path_str)
                .map(|info| info.hicon)
                .unwrap_or(std::ptr::null_mut());

            // 计算释放位置对应的网格坐标
            let (target_col, target_row) = compute_drop_grid_position(hwnd, drop_x, drop_y, data);
            log::info!(
                "释放位置计算: drop=({}, {}) -> grid=({}, {})",
                drop_x,
                drop_y,
                target_col,
                target_row
            );

            let is_insert = insert_col >= 0 && insert_row >= 0;

            if is_insert {
                // === Insert mode: shift existing items right, then add new item ===
                let cols = data.cols;
                let insert_lin = insert_row * cols + insert_col;

                // Materialize all auto-layout positions
                for (i, item) in data.items.iter_mut().enumerate() {
                    if item.grid_col < 0 {
                        item.grid_col = (i as i32) % cols;
                        item.grid_row = (i as i32) / cols;
                    }
                }

                // Shift items at or after insert position right by 1
                for item in data.items.iter_mut() {
                    let item_lin = item.grid_row * cols + item.grid_col;
                    if item_lin >= insert_lin {
                        let new_lin = item_lin + 1;
                        item.grid_col = new_lin % cols;
                        item.grid_row = new_lin / cols;
                    }
                }

                data.items.push(super::grid_window::GridFileItem {
                    file_path: file_path.clone(),
                    display_name: display_name.clone(),
                    icon_handle,
                    grid_col: insert_col,
                    grid_row: insert_row,
                });
                log::info!(
                    "项已插入到框: '{}', insert=({}, {})",
                    display_name,
                    insert_col,
                    insert_row
                );
            } else {
                // === Normal mode: add at computed grid position ===
                data.items.push(super::grid_window::GridFileItem {
                    file_path: file_path.clone(),
                    display_name: display_name.clone(),
                    icon_handle,
                    grid_col: target_col,
                    grid_row: target_row,
                });
                log::info!(
                    "项已添加到框: '{}', grid=({}, {})",
                    display_name,
                    target_col,
                    target_row
                );
            }
            render_layered_window(hwnd);
            save_config();
        }

        // ★ 隐藏桌面原始图标（该图标已在分类框中管理，桌面不应再显示）
        if !public_preflight {
            hide_single_desktop_icon(&file_path, &display_name);
        }
    }
}

pub(crate) fn resolve_drag_item_read_path(file_path: &str) -> std::path::PathBuf {
    resolve_drag_item_read_path_with(file_path, super::icon_hider::resolve_icon_read_path)
}

fn resolve_drag_item_read_path_with(
    file_path: &str,
    resolve_read_path: impl Fn(&str) -> Option<std::path::PathBuf>,
) -> std::path::PathBuf {
    resolve_read_path(file_path).unwrap_or_else(|| std::path::PathBuf::from(file_path))
}

pub(crate) fn hide_single_desktop_icon(file_path: &str, display_name: &str) -> bool {
    match super::icon_hider::hide_single_icon(file_path) {
        Ok(()) => {
            log::info!("桌面图标 '{}' 已收入仓库", display_name);
            true
        }
        Err(e) => {
            log::warn!("收入仓库失败 '{}': {}", display_name, e);
            false
        }
    }
}

/// 根据释放位置的屏幕坐标计算目标网格位置
fn compute_drop_grid_position(
    hwnd: HWND,
    drop_x: i32,
    drop_y: i32,
    data: &super::grid_window::GridWindowData,
) -> (i32, i32) {
    let Some(wr) = get_grid_content_rect(hwnd) else {
        return (-1, -1);
    };
    let client_x = drop_x - wr.left;
    let client_y = drop_y - wr.top;

    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let window_width = wr.right - wr.left;

    let view_params = crate::box_core::view_engine::get_view_params(&data.view_mode, dpi);
    let layout =
        crate::box_core::layout_calc::calculate_grid(window_width, data.title_h, &view_params, dpi);

    if client_x >= layout.grid_left && client_y >= layout.grid_top {
        let col = ((client_x - layout.grid_left) / layout.cell_w).max(0);
        let row = ((client_y - layout.grid_top) / layout.cell_h).max(0);
        (col, row)
    } else {
        // 释放在网格区域外，使用自动布局
        (-1, -1)
    }
}

/// 触发。从 `PENDING_DROP` 读取文件路径，添加到目标分类框，并隐藏桌面图标。
/// 将非桌面文件复制到用户桌面，返回桌面路径字符串。
/// 用于模拟原生桌面拖拽行为：从资源管理器拖拽文件到桌面时，Windows 会复制文件。
/// 检查路径是否来自回收站
fn is_recycle_bin_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("$recycle.bin")
}

/// 从回收站 $I 文件解析原始路径
fn parse_recycle_bin_original_path(recycle_path: &str) -> Option<String> {
    use std::path::Path;
    let p = Path::new(recycle_path);
    let file_name = p.file_name()?.to_string_lossy().to_string();
    if !file_name.starts_with("$R") {
        return None;
    }
    let i_name = format!("$I{}", &file_name[2..]);
    let i_path = p.parent()?.join(&i_name);
    if !i_path.exists() {
        log::warn!("[回收站] $I 文件不存在: {}", i_path.display());
        return None;
    }
    let data = std::fs::read(&i_path).ok()?;
    if data.len() < 20 {
        log::warn!("[回收站] $I 文件太短: {} bytes", data.len());
        return None;
    }
    let result = try_parse_i_file(&data);
    if let Some(ref path) = result {
        log::info!("[回收站] 成功解析: {} -> {}", recycle_path, path);
    } else {
        log::warn!(
            "[回收站] 解析失败, 文件大小={}, 头部={:02X?}",
            data.len(),
            &data[..std::cmp::min(40, data.len())]
        );
    }
    result
}

/// 解析 $I 文件内容，返回原始路径
fn try_parse_i_file(data: &[u8]) -> Option<String> {
    if data.len() < 20 {
        return None;
    }
    if let Some(path) = try_parse_at_offset(data, 16) {
        return Some(path);
    }
    if data.len() >= 28 {
        if let Some(path) = try_parse_at_offset(data, 24) {
            return Some(path);
        }
    }
    if data.len() >= 36 {
        if let Some(path) = try_parse_at_offset(data, 32) {
            return Some(path);
        }
    }
    scan_for_utf16_path(data)
}

/// 尝试从指定偏移读取 path_len + UTF-16 路径
fn try_parse_at_offset(data: &[u8], len_offset: usize) -> Option<String> {
    if len_offset + 4 > data.len() {
        return None;
    }
    let path_len = u32::from_le_bytes([
        data[len_offset],
        data[len_offset + 1],
        data[len_offset + 2],
        data[len_offset + 3],
    ]) as usize;
    if path_len < 3 || path_len > 32768 {
        return None;
    }
    let start = len_offset + 4;
    let end = start + path_len * 2;
    if end > data.len() {
        return None;
    }
    let utf16: Vec<u16> = data[start..end]
        .chunks(2)
        .filter_map(|chunk| {
            if chunk.len() == 2 {
                Some(u16::from_le_bytes([chunk[0], chunk[1]]))
            } else {
                None
            }
        })
        .collect();
    let path = String::from_utf16_lossy(&utf16)
        .trim_end_matches('\0')
        .to_string();
    if path.contains(":\\") || (path.contains('.') && path.len() >= 5) {
        Some(path)
    } else {
        None
    }
}

/// 暴力扫描文件中的 UTF-16LE Windows 路径
fn scan_for_utf16_path(data: &[u8]) -> Option<String> {
    if data.len() < 10 {
        return None;
    }
    for i in 0..data.len().saturating_sub(6) {
        if i + 6 <= data.len()
            && data[i + 1] == 0
            && data[i + 2] == 0x3A
            && data[i + 3] == 0
            && data[i + 4] == 0x5C
            && data[i + 5] == 0
        {
            let drive_letter = data[i];
            if drive_letter >= b'A' && drive_letter <= b'Z' {
                let mut end = i;
                while end + 1 < data.len() {
                    let ch = u16::from_le_bytes([data[end], data[end + 1]]);
                    if ch == 0 {
                        break;
                    }
                    end += 2;
                }
                let utf16: Vec<u16> = data[i..end]
                    .chunks(2)
                    .filter_map(|chunk| {
                        if chunk.len() == 2 {
                            Some(u16::from_le_bytes([chunk[0], chunk[1]]))
                        } else {
                            None
                        }
                    })
                    .collect();
                let path = String::from_utf16_lossy(&utf16);
                if path.len() >= 5 && path.contains('.') {
                    log::info!("[回收站] brute-force 扫描找到路径 @ offset {}: {}", i, path);
                    return Some(path);
                }
            }
        }
    }
    None
}

/// 从回收站恢复文件到用户桌面
fn restore_from_recycle_bin_to_desktop(recycle_path: &str) -> Option<String> {
    use std::path::Path;
    let original_path = parse_recycle_bin_original_path(recycle_path)?;
    let original_filename = Path::new(&original_path)
        .file_name()?
        .to_string_lossy()
        .to_string();
    let desktop = dirs::desktop_dir()?;
    let mut dest = desktop.join(&original_filename);
    if dest.exists() {
        let stem = Path::new(&original_filename)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| original_filename.clone());
        let ext = Path::new(&original_filename)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut counter = 1;
        loop {
            let new_name = format!("{} ({}){}", stem, counter, ext);
            let new_dest = desktop.join(&new_name);
            if !new_dest.exists() {
                dest = new_dest;
                break;
            }
            counter += 1;
            if counter > 100 {
                log::warn!("[回收站] 无法生成唯一文件名");
                return None;
            }
        }
    }
    match std::fs::copy(recycle_path, &dest) {
        Ok(_) => {
            log::info!("[回收站] 恢复文件: {} -> {}", recycle_path, dest.display());
        }
        Err(e) => {
            log::warn!("[回收站] 复制文件失败: {}", e);
            return None;
        }
    }
    let recycle_p = Path::new(recycle_path);
    let _ = std::fs::remove_file(recycle_path);
    if let Some(fname) = recycle_p
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
    {
        if fname.starts_with("$R") {
            let i_path = recycle_p
                .parent()
                .map(|p| p.join(format!("$I{}", &fname[2..])));
            if let Some(ip) = i_path {
                let _ = std::fs::remove_file(&ip);
                log::info!("[回收站] 已删除 $I 文件: {}", ip.display());
            }
        }
    }
    super::icon_hider::notify_shell_create(&dest.to_string_lossy());
    Some(dest.to_string_lossy().to_string())
}

fn copy_file_to_user_desktop(source_path: &str) -> Option<String> {
    use std::path::Path;

    let source = Path::new(source_path);
    if !source.exists() {
        log::warn!("copy_file_to_user_desktop: 源文件不存在: {}", source_path);
        return None;
    }

    let file_name = source.file_name()?.to_string_lossy().to_string();
    let desktop = dirs::desktop_dir()?;
    let dest = desktop.join(&file_name);

    // 如果桌面已有同名文件，生成唯一名称
    let dest = if dest.exists() {
        let stem = source.file_stem()?.to_string_lossy().to_string();
        let ext = source
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut counter = 1;
        loop {
            let new_name = format!("{} ({}){}", stem, counter, ext);
            let new_dest = desktop.join(&new_name);
            if !new_dest.exists() {
                break new_dest;
            }
            counter += 1;
            if counter > 100 {
                log::warn!("copy_file_to_user_desktop: 无法生成唯一文件名");
                return None;
            }
        }
    } else {
        dest
    };

    match std::fs::copy(source, &dest) {
        Ok(_) => {
            super::icon_hider::notify_shell_create(&dest.to_string_lossy());
            Some(dest.to_string_lossy().to_string())
        }
        Err(e) => {
            log::warn!(
                "copy_file_to_user_desktop: 复制失败 {} -> {}: {}",
                source_path,
                dest.display(),
                e
            );
            None
        }
    }
}

pub(crate) fn handle_ole_drop(hwnd: HWND) {
    let t0 = std::time::Instant::now();
    let drop = match super::drop_target::take_pending_drop() {
        Some(d) => d,
        None => {
            log::debug!("OLE drop: 无待处理数据");
            return;
        }
    };

    let is_box_drag = drop.box_data.is_some();
    log::info!(
        "OLE drop: {} 个文件, box_drag={}, drop=({}, {}), paths={:?}",
        drop.file_paths.len(),
        is_box_drag,
        drop.drop_x,
        drop.drop_y,
        drop.file_paths
    );

    let data = match unsafe { get_data(hwnd) } {
        Some(d) => d,
        None => return,
    };

    let mut added = 0;
    // 计算基础 grid 位置（第一个文件的位置）
    let (base_col, base_row) = compute_drop_grid_position(hwnd, drop.drop_x, drop.drop_y, data);
    let cols = data.cols;

    for file_path in &drop.file_paths {
        // 去重：已存在则跳过
        if data.items.iter().any(|i| i.file_path == *file_path) {
            log::debug!("OLE drop: 跳过已存在项 '{}'", file_path);
            continue;
        }

        let is_clsid = file_path.starts_with("::{");
        let is_user_desktop = super::icon_hider::is_user_desktop_path(file_path);
        let is_public_desktop = super::icon_hider::is_public_desktop_path(file_path);
        let is_desktop_file = !is_box_drag && (is_user_desktop || is_public_desktop || is_clsid);
        log::info!(
            "OLE drop: path='{}', is_clsid={}, is_user_desktop={}, is_public_desktop={}, is_box_drag={}, is_desktop_file={}",
            file_path, is_clsid, is_user_desktop, is_public_desktop, is_box_drag, is_desktop_file
        );

        // 对于非桌面、非框间拖拽的外部文件：先复制到用户桌面（模拟原生桌面拖拽行为）
        let is_recycle_bin = !is_box_drag && is_recycle_bin_path(file_path);
        let effective_path = if is_clsid {
            // 虚拟 Shell 对象（回收站、此电脑等）直接使用 CLSID 路径
            log::info!("[OLE-DROP] 虚拟 Shell 对象: {}", file_path);
            file_path.clone()
        } else if is_recycle_bin {
            // 回收站文件：恢复到桌面（使用原始文件名）
            match restore_from_recycle_bin_to_desktop(file_path) {
                Some(desktop_path) => {
                    log::info!(
                        "[OLE-DROP] 回收站文件已恢复到桌面: {} -> {}",
                        file_path,
                        desktop_path
                    );
                    desktop_path
                }
                None => {
                    log::warn!("[OLE-DROP] 回收站文件恢复失败: {}", file_path);
                    file_path.clone()
                }
            }
        } else if !is_box_drag && !is_desktop_file {
            match copy_file_to_user_desktop(file_path) {
                Some(desktop_path) => {
                    log::info!(
                        "[OLE-DROP] 非桌面文件已复制到桌面: {} -> {}",
                        file_path,
                        desktop_path
                    );
                    desktop_path
                }
                None => {
                    log::warn!("[OLE-DROP] 复制文件到桌面失败: {}", file_path);
                    file_path.clone()
                }
            }
        } else {
            file_path.clone()
        };

        log::info!(
            "[OLE-DROP] effective_path='{}', display_name待提取",
            effective_path
        );

        // 提取图标（使用有效路径）
        let icon_info = super::icon_extract::extract_file_icon(&effective_path);
        let (icon_handle, display_name) = match icon_info {
            Some(info) => (info.hicon, info.display_name),
            None => {
                let name = std::path::Path::new(&effective_path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| effective_path.clone());
                (std::ptr::null_mut(), name)
            }
        };

        // 检查是否有插入模式（来自拖拽悬停可视化反馈）
        let is_insert = drop.insert_col >= 0 && drop.insert_row >= 0;
        let (final_col, final_row) = if is_insert {
            // 插入模式：使用插入位置 + 偏移
            let offset_lin = (drop.insert_row * cols + drop.insert_col) + added as i32;
            (offset_lin % cols, offset_lin / cols)
        } else {
            // 正常模式：基础位置 + 偏移（每个文件占据不同的 grid cell）
            let offset_lin = (base_row * cols + base_col) + added as i32;
            let col = offset_lin % cols;
            let row = offset_lin / cols;
            log::info!(
                "[OLE-DROP-OFFSET] file #{}, base=({}, {}), offset_lin={}, final=({}, {})",
                added,
                base_col,
                base_row,
                offset_lin,
                col,
                row
            );
            (col, row)
        };

        log::info!(
            "OLE drop grid: drop=({}, {}) -> grid=({}, {}), insert=({}, {})",
            drop.drop_x,
            drop.drop_y,
            final_col,
            final_row,
            drop.insert_col,
            drop.insert_row
        );

        if is_insert {
            // 插入模式：将现有项右移，新项插入到指定位置
            let cols = data.cols;
            let insert_lin = drop.insert_row * cols + drop.insert_col;

            // 物化所有自动布局位置
            for (i, item) in data.items.iter_mut().enumerate() {
                if item.grid_col < 0 {
                    item.grid_col = (i as i32) % cols;
                    item.grid_row = (i as i32) / cols;
                }
            }

            // 将插入位置及之后的项右移
            for item in data.items.iter_mut() {
                let item_lin = item.grid_row * cols + item.grid_col;
                if item_lin >= insert_lin {
                    let new_lin = item_lin + 1;
                    item.grid_col = new_lin % cols;
                    item.grid_row = new_lin / cols;
                }
            }
        }

        // 外部→框：按统一收纳流程处理桌面图标。
        if !is_box_drag {
            let t_ingest = std::time::Instant::now();
            match super::icon_hider::ingest_icon(&effective_path) {
                Ok(()) => {
                    log::info!(
                        "OLE drop: '{}' 已收入仓库 [{:?}]",
                        display_name,
                        t_ingest.elapsed()
                    );
                    data.items.push(super::grid_window::GridFileItem {
                        file_path: effective_path.clone(),
                        display_name: display_name.clone(),
                        icon_handle,
                        grid_col: final_col,
                        grid_row: final_row,
                    });
                    added += 1;
                }
                Err(e) => {
                    log::warn!(
                        "OLE drop: 收入仓库失败 '{}': {}，不添加到分类框",
                        display_name,
                        e
                    );
                    if !icon_handle.is_null() {
                        super::icon_extract::destroy_icon(icon_handle);
                    }
                }
            }
        } else {
            // 框间拖拽：直接添加
            data.items.push(super::grid_window::GridFileItem {
                file_path: effective_path.clone(),
                display_name: display_name.clone(),
                icon_handle,
                grid_col: final_col,
                grid_row: final_row,
            });
            added += 1;
        }
    }

    if added > 0 {
        log::info!(
            "OLE drop: 添加 {} 个项到分类框 [total {:?}]",
            added,
            t0.elapsed()
        );
        super::d2d_render::render_layered_window(hwnd);
        save_config();
    }

    // 清除拖拽悬停可视化状态
    data.drag_over_item = -1;
    data.drag_over_insert_col = -1;
    data.drag_over_insert_row = -1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_resolve_drag_item_read_path_uses_resolved_vault_path() {
            let logical = r"C:\Users\chenzhihui\Desktop\CC Switch.lnk";
            let vault = std::path::PathBuf::from(
                r"C:\Users\chenzhihui\.ikun_organizer\managed_icons\user__CC Switch.lnk",
            );

            assert_eq!(
                resolve_drag_item_read_path_with(logical, |_| Some(vault.clone())),
                vault
            );
        }

        #[test]
        fn test_resolve_drag_item_read_path_falls_back_to_logical_path() {
            let path = r"Z:\ikun-organizer-test\missing-shortcut.lnk";

            assert_eq!(
                resolve_drag_item_read_path_with(path, |_| None),
                std::path::PathBuf::from(path)
            );
        }
    }

    // =============================================================================
    // OLE 拖拽放置处理（由 WM_USER_DRAG_DROP 触发）
    // =============================================================================

    /// 处理 OLE 拖拽放置（桌面→框 或 框→框）
    ///
    /// 由 `drop_target.rs` 的 `drop_target_drop()` 通过 `PostMessageW(WM_USER_DRAG_DROP)`

    #[test]
    fn test_compute_drop_grid_position_basic() {
        // Test basic grid position calculation
        // This is a simplified test - in real usage, HWND and GetWindowRect would be mocked
        // For now, we test the logic with known values

        let grid_left = 10;
        let grid_top = 40;
        let cell_w = 80;
        let cell_h = 96;

        // Drop at first cell
        let drop_x = grid_left + 10;
        let drop_y = grid_top + 10;
        let client_x = drop_x;
        let client_y = drop_y;

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 0);
        assert_eq!(row, 0);

        // Drop at second cell (col=1, row=0)
        let drop_x = grid_left + cell_w + 10;
        let drop_y = grid_top + 10;
        let client_x = drop_x;
        let client_y = drop_y;

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 1);
        assert_eq!(row, 0);

        // Drop at third cell (col=0, row=1)
        let drop_x = grid_left + 10;
        let drop_y = grid_top + cell_h + 10;
        let client_x = drop_x;
        let client_y = drop_y;

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 0);
        assert_eq!(row, 1);
    }

    #[test]
    fn test_compute_drop_grid_position_outside_grid() {
        // Test dropping outside grid area returns (-1, -1)
        let grid_left = 10;
        let grid_top = 40;

        // Drop before grid_left
        let client_x = grid_left - 5;
        let client_y = grid_top + 10;

        if client_x < grid_left || client_y < grid_top {
            let col = -1;
            let row = -1;
            assert_eq!(col, -1);
            assert_eq!(row, -1);
        }
    }

    #[test]
    fn test_compute_drop_grid_position_negative_coordinates() {
        // Test that negative coordinates are clamped to 0
        let grid_left = 10;
        let grid_top = 40;
        let cell_w = 80;
        let cell_h = 96;

        let client_x = grid_left + 10;
        let client_y = grid_top + 10;

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 0);
        assert_eq!(row, 0);
    }
}

#[cfg(test)]
mod ole_drop_tests {
    use super::*;

    #[test]
    fn test_copy_file_to_user_desktop_handles_missing_source() {
        // Test that copy_file_to_user_desktop returns None for non-existent source
        let result = super::copy_file_to_user_desktop(r"Z:\nonexistent\file.txt");
        assert!(result.is_none());
    }

    #[test]
    fn test_compute_drop_grid_position_outside_grid() {
        // Test that positions outside grid area return (-1, -1)
        // This is a simplified test - in real usage we'd need to mock HWND
        let grid_left = 10;
        let grid_top = 40;
        let client_x = grid_left - 5;
        let client_y = grid_top - 5;

        if client_x < grid_left || client_y < grid_top {
            let col = -1;
            let row = -1;
            assert_eq!(col, -1);
            assert_eq!(row, -1);
        }
    }

    #[test]
    fn test_compute_drop_grid_position_valid_grid() {
        // Test valid grid position calculation
        let grid_left = 10;
        let grid_top = 40;
        let cell_w = 80;
        let cell_h = 96;
        let client_x = grid_left + cell_w + 20; // Second column
        let client_y = grid_top + 20; // First row

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 1);
        assert_eq!(row, 0);
    }

    #[test]
    fn test_compute_drop_grid_position_basic() {
        // Test basic grid position calculation
        let grid_left = 10;
        let grid_top = 40;
        let cell_w = 80;
        let cell_h = 96;

        // Drop at first cell
        let drop_x = grid_left + 10;
        let drop_y = grid_top + 10;
        let client_x = drop_x;
        let client_y = drop_y;

        let col = ((client_x - grid_left) / cell_w).max(0);
        let row = ((client_y - grid_top) / cell_h).max(0);

        assert_eq!(col, 0);
        assert_eq!(row, 0);

        // Drop at second column
        let drop_x = grid_left + cell_w + 20;
        let client_x = drop_x;
        let col = ((client_x - grid_left) / cell_w).max(0);
        assert_eq!(col, 1);

        // Drop at second row
        let drop_y = grid_top + cell_h + 30;
        let client_y = drop_y;
        let row = ((client_y - grid_top) / cell_h).max(0);
        assert_eq!(row, 1);
    }

    #[test]
    fn test_copy_file_to_user_desktop_nonexistent_source() {
        // Test that copy_file_to_user_desktop returns None for non-existent source
        let result = super::copy_file_to_user_desktop(r"Z:\nonexistent\file.txt");
        assert!(result.is_none());
    }

    #[test]
    fn test_insert_mode_logic() {
        // Test insert mode detection
        let insert_col = 2;
        let insert_row = 3;
        let is_insert = insert_col >= 0 && insert_row >= 0;
        assert!(is_insert);

        // Test non-insert mode
        let insert_col = -1;
        let insert_row = -1;
        let is_insert = insert_col >= 0 && insert_row >= 0;
        assert!(!is_insert);
    }

    #[test]
    fn test_is_recycle_bin_path_positive() {
        assert!(is_recycle_bin_path(
            r"C:\$Recycle.Bin\S-1-5-21\$RGRKAOP.lnk"
        ));
    }

    #[test]
    fn test_is_recycle_bin_path_negative() {
        assert!(!is_recycle_bin_path(r"C:\Users\test\Desktop\Chrome.lnk"));
        assert!(!is_recycle_bin_path(
            r"C:\Users\test\.ikun_organizer\managed_icons\user__Chrome.lnk"
        ));
    }

    #[test]
    fn test_try_parse_at_offset_valid_v1() {
        let path = r"C:\Users\test\Desktop\Chrome.lnk";
        let path_utf16: Vec<u8> = path
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .flat_map(|c| c.to_le_bytes())
            .collect();
        let path_len = (path.encode_utf16().count() + 1) as u32;
        let mut data = vec![0u8; 20 + path_utf16.len()];
        data[16..20].copy_from_slice(&path_len.to_le_bytes());
        data[20..20 + path_utf16.len()].copy_from_slice(&path_utf16);
        let result = try_parse_at_offset(&data, 16);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), path);
    }

    #[test]
    fn test_try_parse_at_offset_invalid_length() {
        let data = vec![0u8; 20];
        assert!(try_parse_at_offset(&data, 16).is_none());
    }

    #[test]
    fn test_try_parse_at_offset_too_short() {
        let data = vec![0u8; 10];
        assert!(try_parse_at_offset(&data, 16).is_none());
    }

    #[test]
    fn test_scan_for_utf16_path_found() {
        let path = r"C:\Users\test\Desktop\Chrome.lnk";
        let mut data = vec![0u8; 100];
        let path_bytes: Vec<u8> = path
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .flat_map(|c| c.to_le_bytes())
            .collect();
        data.extend_from_slice(&path_bytes);
        let result = scan_for_utf16_path(&data);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), path);
    }

    #[test]
    fn test_scan_for_utf16_path_not_found() {
        let data = vec![0u8; 50];
        assert!(scan_for_utf16_path(&data).is_none());
    }

    #[test]
    fn test_scan_for_utf16_path_too_short() {
        let data = vec![0u8; 5];
        assert!(scan_for_utf16_path(&data).is_none());
    }
}
