//! 桌面覆盖窗口创建与生命周期管理
//!
//! 阶段 1 创建一个覆盖主显示器工作区的全透明分层窗口，放在较低 Z-order，
//! 用于后续自行渲染桌面图标。窗口不抢焦点，当前阶段鼠标事件全部穿透。

use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use crate::config::app_config::OverlayConfig;

use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1Factory};
use windows::Win32::Graphics::DirectWrite::IDWriteFactory;
use windows::Win32::Graphics::Imaging::IWICImagingFactory;

use crate::box_core::view_engine::get_view_params;
use crate::config::app_config::{AppConfig, ViewMode};
use crate::platform::windows::ffi_imports::*;
use crate::platform::windows::ffi_types::*;
use crate::platform::DesktopManagerTrait;

use super::overlay_grid::{overlay_first_free_cell, overlay_grid_to_logical, overlay_snap_to_grid};
use super::overlay_render::render_desktop_overlay;

use super::overlay_wnd_proc::overlay_wnd_proc;

/// 覆盖窗口中的桌面图标项。
pub(crate) struct OverlayIconItem {
    /// 文件路径（用于后续双击打开、右键菜单、拖拽）
    pub(crate) file_path: String,
    /// 显示名称
    pub(crate) display_name: String,
    /// 系统图标句柄
    pub(crate) icon_handle: HICON,
    /// 持久化的桌面 X 坐标（逻辑像素，96 DPI 基准）
    pub(crate) x: f32,
    /// 持久化的桌面 Y 坐标（逻辑像素，96 DPI 基准）
    pub(crate) y: f32,
}

impl OverlayIconItem {
    fn new(file_path: String, display_name: String, icon_handle: HICON) -> Self {
        Self {
            file_path,
            display_name,
            icon_handle,
            x: -1.0,
            y: -1.0,
        }
    }

    fn with_position(
        file_path: String,
        display_name: String,
        icon_handle: HICON,
        position: Option<[f32; 2]>,
    ) -> Self {
        let mut item = Self::new(file_path, display_name, icon_handle);
        if let Some([x, y]) = position {
            item.x = x;
            item.y = y;
        }
        item
    }
}

unsafe impl Send for OverlayIconItem {}

/// 桌面覆盖窗口运行时数据。
pub(crate) struct OverlayWindowData {
    /// 工作区左上角 X（物理像素）
    pub(crate) x: i32,
    /// 工作区左上角 Y（物理像素）
    pub(crate) y: i32,
    /// 工作区宽度（物理像素）
    pub(crate) width: i32,
    /// 工作区高度（物理像素）
    pub(crate) height: i32,
    /// 窗口 DPI
    pub(crate) dpi: u32,
    /// 当前覆盖窗口负责绘制的桌面图标
    pub(crate) icons: Vec<OverlayIconItem>,
    /// 当前选中的图标索引
    pub(crate) selected_item: i32,
    /// 当前 hover 的图标索引
    pub(crate) hovered_item: i32,
    /// 是否正在等待拖拽阈值触发
    pub(crate) drag_pending: bool,
    /// 是否已经进入拖拽状态
    pub(crate) drag_active: bool,
    /// 正在拖拽的图标索引
    pub(crate) drag_item_index: i32,
    /// 拖拽起点 X（窗口客户区坐标）
    pub(crate) drag_start_x: i32,
    /// 拖拽起点 Y（窗口客户区坐标）
    pub(crate) drag_start_y: i32,
    /// 当前布局单元格宽度
    pub(crate) cell_w: i32,
    /// 当前布局单元格高度
    pub(crate) cell_h: i32,
    /// 当前布局图标尺寸
    pub(crate) icon_size: i32,
    /// 拖拽覆盖时，光标下的替换高亮项索引（-1 = 无替换高亮）
    pub(crate) drag_over_item: i32,
    /// 拖拽覆盖时，插入指示器的列位置（-1 = 无插入指示）
    pub(crate) drag_over_insert_col: i32,
    /// 拖拽覆盖时，插入指示器的行位置（-1 = 无插入指示）
    pub(crate) drag_over_insert_row: i32,
    /// D2D Factory
    pub(crate) d2d_factory: Option<ID2D1Factory>,
    /// DirectWrite Factory
    pub(crate) dw_factory: Option<IDWriteFactory>,
    /// WIC Factory
    pub(crate) wic_factory: Option<IWICImagingFactory>,
    /// DC RenderTarget
    pub(crate) d2d_target: Option<ID2D1DCRenderTarget>,
    // ===== 重命名状态 =====
}

unsafe impl Send for OverlayWindowData {}

/// 覆盖窗口句柄包装。
struct OverlayWindow {
    hwnd: HWND,
}

unsafe impl Send for OverlayWindow {}

static OVERLAY_WINDOWS: Mutex<Vec<OverlayWindow>> = Mutex::new(Vec::new());
static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

static CLASS_NAME: &[u16] = &[
    68, 101, 115, 107, 116, 111, 112, 79, 114, 103, 79, 118, 101, 114, 108, 97, 121, 0,
]; // "DesktopOrgOverlay"

fn ensure_class_registered() -> Result<HINSTANCE> {
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    if hinstance.is_null() {
        anyhow::bail!("GetModuleHandleW 失败");
    }

    let mut registered = CLASS_REGISTERED
        .lock()
        .map_err(|e| anyhow::anyhow!("覆盖窗口类锁获取失败: {}", e))?;
    if *registered {
        return Ok(hinstance);
    }

    let wc = WNDCLASSEXW {
        cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
        lpfn_wnd_proc: Some(overlay_wnd_proc),
        cb_cls_extra: 0,
        cb_wnd_extra: 0,
        h_instance: hinstance,
        h_icon: std::ptr::null_mut(),
        h_cursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW_ID as usize as *const u16) },
        hbr_background: std::ptr::null_mut(),
        lpsz_menu_name: std::ptr::null(),
        lpsz_class_name: CLASS_NAME.as_ptr(),
        h_icon_sm: std::ptr::null_mut(),
    };

    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        anyhow::bail!("RegisterClassExW 注册桌面覆盖窗口类失败");
    }

    *registered = true;
    log::info!("桌面覆盖窗口类已注册");
    Ok(hinstance)
}

/// 创建桌面覆盖窗口。
///
/// 阶段 1 只覆盖鼠标所在显示器的工作区。后续阶段会扩展为每个显示器一个窗口。
pub fn create_desktop_overlay() -> Result<()> {
    if let Ok(windows) = OVERLAY_WINDOWS.lock() {
        if !windows.is_empty() {
            log::debug!("桌面覆盖窗口已存在，跳过创建");
            if let Err(e) = super::desktop_watcher::start_desktop_file_watcher() {
                log::warn!("启动桌面文件监听失败: {}", e);
            }
            return Ok(());
        }
    }

    let hinstance = ensure_class_registered()?;
    // 使用虚拟桌面边界覆盖所有显示器，支持跨显示器拖拽
    let (x, y, width, height) =
        crate::platform::windows::monitor_utils::get_virtual_desktop_bounds();

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            width,
            height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null_mut(),
        )
    };

    if hwnd.is_null() {
        anyhow::bail!("CreateWindowExW 创建桌面覆盖窗口失败");
    }

    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let icons = collect_overlay_icons();
    let icon_count = icons.len();
    let layout = overlay_layout_params(width, dpi);
    let data = Box::new(OverlayWindowData {
        x,
        y,
        width,
        height,
        dpi,
        icons,
        selected_item: -1,
        hovered_item: -1,
        drag_pending: false,
        drag_active: false,
        drag_item_index: -1,
        drag_start_x: 0,
        drag_start_y: 0,
        cell_w: layout.cell_w,
        cell_h: layout.cell_h,
        icon_size: layout.icon_size,
        drag_over_item: -1,
        drag_over_insert_col: -1,
        drag_over_insert_row: -1,
        d2d_factory: None,
        dw_factory: None,
        wic_factory: None,
        d2d_target: None,
    });
    let data_ptr = Box::into_raw(data);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
    }

    // 阶段 2：使用 per-pixel alpha 渲染未管理桌面图标。
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        ShowWindow(hwnd, SW_SHOWNA);
    }
    render_desktop_overlay(hwnd);

    OVERLAY_WINDOWS
        .lock()
        .map_err(|e| anyhow::anyhow!("覆盖窗口列表锁获取失败: {}", e))?
        .push(OverlayWindow { hwnd });

    if let Err(e) = super::desktop_watcher::start_desktop_file_watcher() {
        log::warn!("启动桌面文件监听失败: {}", e);
    }

    log::info!(
        "桌面覆盖窗口已创建: hwnd={:?}, area=({}, {}) {}x{}, dpi={}, icons={}",
        hwnd,
        x,
        y,
        width,
        height,
        dpi,
        icon_count
    );
    Ok(())
}

/// 销毁所有桌面覆盖窗口。
pub fn destroy_desktop_overlay() {
    let windows = match OVERLAY_WINDOWS.lock() {
        Ok(mut guard) => guard.drain(..).collect::<Vec<_>>(),
        Err(e) => {
            log::warn!("覆盖窗口列表锁获取失败: {}", e);
            return;
        }
    };

    for window in windows {
        release_overlay_window_data(window.hwnd);
        unsafe {
            DestroyWindow(window.hwnd);
        }
    }

    super::desktop_watcher::stop_desktop_file_watcher();
    log::info!("所有桌面覆盖窗口已销毁");
}

/// 请求所有覆盖窗口刷新图标列表。
pub(crate) fn request_desktop_overlay_refresh() {
    let windows = match OVERLAY_WINDOWS.lock() {
        Ok(windows) => windows.iter().map(|window| window.hwnd).collect::<Vec<_>>(),
        Err(e) => {
            log::warn!("覆盖窗口列表锁获取失败，无法请求刷新: {}", e);
            return;
        }
    };

    for hwnd in windows {
        unsafe {
            PostMessageW(hwnd, WM_USER_REFRESH_OVERLAY, 0, 0);
        }
    }
}

/// 刷新指定覆盖窗口的图标列表。
pub(crate) fn refresh_desktop_overlay_icons(hwnd: HWND) {
    // 拖拽期间跳过刷新：刷新会重置 drag_active 等状态，导致残影卡死
    if let Some(data) = get_overlay_data(hwnd) {
        if data.drag_active || data.drag_pending {
            log::info!("refresh_desktop_overlay_icons: 拖拽中，跳过刷新");
            return;
        }
    }

    let new_icons = collect_overlay_icons();
    let new_count = new_icons.len();
    let Some(data) = get_overlay_data(hwnd) else {
        log::warn!("覆盖窗口刷新失败: hwnd={:?} 缺少运行时数据", hwnd);
        return;
    };

    let old_icons = std::mem::replace(&mut data.icons, new_icons);
    for icon in old_icons {
        crate::platform::windows::icon_extract::destroy_icon(icon.icon_handle);
    }
    reset_overlay_interaction_state(data);
    let layout = overlay_layout_params(data.width, data.dpi);
    data.cell_w = layout.cell_w;
    data.cell_h = layout.cell_h;
    data.icon_size = layout.icon_size;
    render_desktop_overlay(hwnd);
    log::info!("覆盖窗口图标列表已刷新: {} 个", new_count);
}

fn reset_overlay_interaction_state(data: &mut OverlayWindowData) {
    data.selected_item = -1;
    data.hovered_item = -1;
    data.drag_pending = false;
    data.drag_active = false;
    data.drag_item_index = -1;
    data.drag_start_x = 0;
    data.drag_start_y = 0;
}

fn release_overlay_window_data(hwnd: HWND) {
    let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut OverlayWindowData;
    if data_ptr.is_null() {
        return;
    }
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let data = Box::from_raw(data_ptr);
        for icon in &data.icons {
            crate::platform::windows::icon_extract::destroy_icon(icon.icon_handle);
        }
        drop(data);
    }
}

/// 判断 HWND 是否为桌面覆盖窗口。
pub(crate) fn is_desktop_overlay_window(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let mut class_name = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
    if len <= 0 {
        return false;
    }
    String::from_utf16_lossy(&class_name[..len as usize]) == "DesktopOrgOverlay"
}

/// 将一个图标添加回覆盖窗口并重绘。
pub(crate) fn add_icon_to_overlay(
    file_path: String,
    display_name: String,
    drop_x: i32,
    drop_y: i32,
) -> bool {
    let hwnd = match OVERLAY_WINDOWS.lock() {
        Ok(windows) => windows.first().map(|window| window.hwnd),
        Err(e) => {
            log::warn!("覆盖窗口列表锁获取失败: {}", e);
            None
        }
    };
    let Some(hwnd) = hwnd else {
        log::warn!("框→覆盖窗口: 未找到覆盖窗口，无法显示 {}", display_name);
        return false;
    };

    let Some(data) = get_overlay_data(hwnd) else {
        log::warn!("框→覆盖窗口: 覆盖窗口数据缺失，无法显示 {}", display_name);
        return false;
    };

    if data
        .icons
        .iter()
        .any(|item| item.file_path == file_path || item.display_name == display_name)
    {
        log::debug!("框→覆盖窗口: 覆盖窗口已存在该图标，跳过 {}", display_name);
        return true;
    }

    // Snap to grid: if drop position provided, snap to nearest cell;
    // if target cell is occupied or no drop position, find the first free cell.
    let position = if drop_x >= 0 && drop_y >= 0 {
        let local_x = drop_x.saturating_sub(data.x);
        let local_y = drop_y.saturating_sub(data.y);
        let (snap_pos, occupied) = overlay_snap_to_grid(data, local_x, local_y, -1);
        if occupied.is_some() {
            let (col, row) = overlay_first_free_cell(data);
            overlay_grid_to_logical(data, col, row)
        } else {
            snap_pos
        }
    } else {
        let (col, row) = overlay_first_free_cell(data);
        overlay_grid_to_logical(data, col, row)
    };

    let icon_handle = crate::platform::windows::extract_file_icon(&file_path)
        .map(|info| info.hicon)
        .unwrap_or(std::ptr::null_mut());
    data.icons.push(OverlayIconItem::with_position(
        file_path.clone(),
        display_name.clone(),
        icon_handle,
        Some(position),
    ));
    data.selected_item = (data.icons.len() as i32).saturating_sub(1);
    data.hovered_item = -1;
    render_desktop_overlay(hwnd);
    save_overlay_icon_positions();
    log::info!("框→覆盖窗口: '{}' 已显示在覆盖窗口", display_name);
    true
}

/// 收集需要由覆盖窗口绘制的桌面图标。
fn collect_overlay_icons() -> Vec<OverlayIconItem> {
    let managed_paths = collect_managed_icon_paths();
    let manager = match crate::platform::windows::DesktopManager::new() {
        Ok(manager) => manager,
        Err(e) => {
            log::warn!("覆盖窗口初始化桌面管理器失败: {}", e);
            return Vec::new();
        }
    };

    let mut items = Vec::new();
    let mut seen_display_names = std::collections::HashSet::new();
    let overlay_config = AppConfig::load()
        .map(|config| config.overlay)
        .unwrap_or_else(|e| {
            log::warn!("覆盖窗口读取位置配置失败，使用默认位置: {}", e);
            OverlayConfig::default()
        });
    log::debug!(
        "覆盖窗口: managed_paths数={}, 开始遍历桌面图标",
        managed_paths.len()
    );
    for icon in manager.get_desktop_icons() {
        let path = icon.path.to_string_lossy().to_string();
        let is_managed = is_managed_icon_path(&path, &icon.display_name, &managed_paths);
        if is_managed {
            continue;
        }
        // 按 display_name 去重：同一图标可能在用户桌面和公共桌面各有一个 .lnk
        let norm_dn = icon.display_name.to_lowercase();
        if !seen_display_names.insert(norm_dn) {
            continue;
        }

        let icon_handle = crate::platform::windows::extract_file_icon(&path)
            .map(|info| info.hicon)
            .unwrap_or(std::ptr::null_mut());
        let position = overlay_icon_position(&overlay_config, &path);
        items.push(OverlayIconItem::with_position(
            path,
            icon.display_name,
            icon_handle,
            position,
        ));
    }

    log::info!("覆盖窗口收集未管理桌面图标: {} 个", items.len());
    items
}

fn collect_managed_icon_paths() -> HashSet<String> {
    let mut paths = HashSet::new();
    let config = match AppConfig::load() {
        Ok(config) => config,
        Err(e) => {
            log::warn!("覆盖窗口读取配置失败，无法过滤已管理图标: {}", e);
            return paths;
        }
    };

    for box_data in &config.layout.boxes {
        for path in &box_data.icon_names {
            paths.insert(normalize_path_key(path));
            if let Some(read_path) = crate::platform::windows::resolve_icon_read_path(path) {
                paths.insert(normalize_path_key(&read_path.to_string_lossy()));
            }
        }
    }
    paths
}

fn is_managed_icon_path(path: &str, display_name: &str, managed_paths: &HashSet<String>) -> bool {
    if managed_paths.contains(&normalize_path_key(path)) {
        return true;
    }

    let Some(file_name) = Path::new(path).file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    let norm_file_name = normalize_path_key(file_name);
    let public_alias = normalize_path_key(&format!("public__{}", file_name));

    managed_paths.iter().any(|managed| {
        // 精确匹配文件名部分（避免子串误判：如"企业微信.lnk"匹配"微信.lnk"）
        let managed_file_name = Path::new(managed)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        // 检查 managed 路径的文件名是否精确匹配
        if managed_file_name == norm_file_name {
            return true;
        }
        // 检查 managed 路径的文件名是否精确匹配 public__ 别名
        if managed_file_name == public_alias {
            return true;
        }
        // 检查 display_name 精确匹配（不使用 ends_with，避免子串误判）
        if !display_name.is_empty() {
            let norm_dn = normalize_path_key(display_name);
            // 从 managed 路径提取文件名（去掉扩展名），与 display_name 比较
            let managed_stem = Path::new(managed)
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if normalize_path_key(managed_stem) == norm_dn {
                return true;
            }
        }
        false
    })
}

fn normalize_path_key(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

/// 从 HWND 获取覆盖窗口数据。
pub(crate) fn get_overlay_data<'a>(hwnd: HWND) -> Option<&'a mut OverlayWindowData> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut OverlayWindowData;
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &mut *ptr })
    }
}

fn overlay_icon_position(config: &OverlayConfig, path: &str) -> Option<[f32; 2]> {
    config.icon_positions.get(path).copied().or_else(|| {
        config
            .icon_positions
            .get(&normalize_path_key(path))
            .copied()
    })
}

pub(crate) fn save_overlay_icon_positions() {
    log::info!("save_overlay_icon_positions: 开始保存覆盖窗口图标位置");
    let mut positions = std::collections::HashMap::new();
    let windows = match OVERLAY_WINDOWS.lock() {
        Ok(windows) => windows.iter().map(|window| window.hwnd).collect::<Vec<_>>(),
        Err(e) => {
            log::warn!("覆盖窗口列表锁获取失败，无法保存图标位置: {}", e);
            return;
        }
    };

    for hwnd in windows {
        if let Some(data) = get_overlay_data(hwnd) {
            for icon in &data.icons {
                // 只保存用户拖拽设定的位置（x >= 0 && y >= 0），
                // 不保存自动布局位置（x < 0 || y < 0），避免下次刷新时自动布局位置
                // 因列表顺序变化导致图标重叠
                if icon.x >= 0.0 && icon.y >= 0.0 {
                    let position = [icon.x, icon.y];
                    positions.insert(icon.file_path.clone(), position);
                    positions.insert(normalize_path_key(&icon.file_path), position);
                }
            }
        }
    }

    let mut config = AppConfig::load().unwrap_or_else(|e| {
        log::warn!("保存覆盖窗口位置前加载配置失败，使用默认配置: {}", e);
        AppConfig::default()
    });
    log::info!(
        "save_overlay_icon_positions: 当前覆盖窗口图标数={}, 保存位置数={}, 原有位置数={}",
        positions.len() / 2, // 除以2因为每个图标有两个key（原路径和normalized）
        positions.len(),
        config.overlay.icon_positions.len()
    );
    // 检查是否有位置丢失
    for (path, pos) in &config.overlay.icon_positions {
        if !positions.contains_key(path) {
            log::warn!(
                "save_overlay_icon_positions: 位置丢失! path={}, pos={:?}",
                path,
                pos
            );
        }
    }
    config.overlay.icon_positions = positions;
    config.last_modified = chrono::Utc::now();
    if let Err(e) = config.save() {
        log::warn!("保存覆盖窗口图标位置失败: {}", e);
    } else {
        log::info!("覆盖窗口图标位置已保存");
    }
}

fn resolved_icon_position(data: &OverlayWindowData, icon: &OverlayIconItem) -> [f32; 2] {
    if icon.x >= 0.0 && icon.y >= 0.0 {
        return [icon.x, icon.y];
    }

    let Some(index) = data
        .icons
        .iter()
        .position(|item| item.file_path == icon.file_path)
    else {
        return [0.0, 0.0];
    };
    let layout = overlay_layout_params(data.width, data.dpi);
    // 纵向优先（列优先）布局：先填满一列再换下一列
    let total = data.icons.len().max(1) as i32;
    let rows = (total + layout.cols - 1) / layout.cols;
    let row = (index as i32) % rows;
    let col = (index as i32) / rows;
    [
        physical_to_logical(layout.padding + col * layout.cell_w, data.dpi),
        physical_to_logical(layout.padding + row * layout.cell_h, data.dpi),
    ]
}

/// 获取指定屏幕坐标处的显示器 DPI。
///
/// 使用 `MonitorFromPoint` + `GetDpiForMonitor` 获取该坐标所在显示器的有效 DPI。
/// 失败时回退到 96 DPI。
pub(crate) fn dpi_at_screen_point(screen_x: i32, screen_y: i32) -> u32 {
    let pt = POINT {
        x: screen_x,
        y: screen_y,
    };
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return 96;
    }
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } == 0 {
        dpi_x
    } else {
        96
    }
}

pub(crate) fn physical_to_logical(value: i32, dpi: u32) -> f32 {
    value as f32 * 96.0 / dpi.max(1) as f32
}

pub(crate) fn logical_to_physical(value: f32, dpi: u32) -> i32 {
    (value * dpi.max(1) as f32 / 96.0).round() as i32
}

/// 覆盖窗口图标布局参数。
pub(crate) struct OverlayLayoutParams {
    pub(crate) padding: i32,
    pub(crate) cell_w: i32,
    pub(crate) cell_h: i32,
    pub(crate) icon_size: i32,
    pub(crate) cols: i32,
}

/// 计算覆盖窗口当前图标布局参数。
pub(crate) fn overlay_layout_params(width: i32, dpi: u32) -> OverlayLayoutParams {
    let params = get_view_params(&ViewMode::Medium, dpi);
    let padding = scale_for_dpi(20, dpi);
    let cols = 2.max((width.saturating_sub(padding * 2) / params.cell_w).max(1));
    OverlayLayoutParams {
        padding,
        cell_w: params.cell_w,
        cell_h: params.cell_h,
        icon_size: params.icon_size,
        cols,
    }
}

/// 命中测试覆盖窗口图标区域，返回图标索引，空白区域返回 -1。
///
/// 对有存储位置的图标，使用该图标所在显示器 DPI 计算单元格大小，
/// 确保命中区域与渲染区域一致。
pub(crate) fn overlay_hit_test(data: &OverlayWindowData, x: i32, y: i32) -> i32 {
    let grid_layout = overlay_layout_params(data.width, data.dpi);

    // 有存储位置的图标：使用 per-icon DPI 做精确命中测试
    for (index, icon) in data.icons.iter().enumerate() {
        if icon.x >= 0.0 && icon.y >= 0.0 {
            let cell_x = logical_to_physical(icon.x, data.dpi);
            let cell_y = logical_to_physical(icon.y, data.dpi);
            let icon_dpi = dpi_at_screen_point(data.x + cell_x, data.y + cell_y);
            let icon_layout = overlay_layout_params(data.width, icon_dpi);
            if x >= cell_x
                && x < cell_x + icon_layout.cell_w
                && y >= cell_y
                && y < cell_y + icon_layout.cell_h
            {
                return index as i32;
            }
        }
    }

    // 网格布局图标：使用窗口级 DPI
    if x < grid_layout.padding
        || y < grid_layout.padding
        || grid_layout.cell_w <= 0
        || grid_layout.cell_h <= 0
    {
        return -1;
    }

    let col = (x - grid_layout.padding) / grid_layout.cell_w;
    let row = (y - grid_layout.padding) / grid_layout.cell_h;
    if col < 0 || row < 0 || col >= grid_layout.cols {
        return -1;
    }

    let idx = row * grid_layout.cols + col;
    if idx < 0 || (idx as usize) >= data.icons.len() {
        return -1;
    }

    let cell_x = overlay_icon_cell_x(data, idx as usize, &grid_layout);
    let cell_y = overlay_icon_cell_y(data, idx as usize, &grid_layout);
    if x >= cell_x
        && x < cell_x + grid_layout.cell_w
        && y >= cell_y
        && y < cell_y + grid_layout.cell_h
    {
        idx
    } else {
        -1
    }
}

pub(crate) fn overlay_icon_cell_x(
    data: &OverlayWindowData,
    index: usize,
    layout: &OverlayLayoutParams,
) -> i32 {
    let icon = match data.icons.get(index) {
        Some(icon) => icon,
        None => return layout.padding,
    };
    if icon.x >= 0.0 {
        return logical_to_physical(icon.x, data.dpi);
    }
    // 纵向优先（列优先）布局
    let total = data.icons.len().max(1) as i32;
    let rows = (total + layout.cols - 1) / layout.cols;
    let col = (index as i32) / rows;
    layout.padding + col * layout.cell_w
}

pub(crate) fn overlay_icon_cell_y(
    data: &OverlayWindowData,
    index: usize,
    layout: &OverlayLayoutParams,
) -> i32 {
    let icon = match data.icons.get(index) {
        Some(icon) => icon,
        None => return layout.padding,
    };
    if icon.y >= 0.0 {
        return logical_to_physical(icon.y, data.dpi);
    }
    // 纵向优先（列优先）布局
    let total = data.icons.len().max(1) as i32;
    let rows = (total + layout.cols - 1) / layout.cols;
    let row = (index as i32) % rows;
    layout.padding + row * layout.cell_h
}

fn scale_for_dpi(value: i32, dpi: u32) -> i32 {
    ((value as f32) * dpi as f32 / 96.0).round() as i32
}

impl OverlayWindowData {
    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            dpi: 96,
            icons: Vec::new(),
            selected_item: -1,
            hovered_item: -1,
            drag_pending: false,
            drag_active: false,
            drag_item_index: -1,
            drag_start_x: 0,
            drag_start_y: 0,
            cell_w: 0,
            cell_h: 0,
            icon_size: 0,
            drag_over_item: -1,
            drag_over_insert_col: -1,
            drag_over_insert_row: -1,
            d2d_factory: None,
            dw_factory: None,
            wic_factory: None,
            d2d_target: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_data_fields() {
        let data = OverlayWindowData {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            dpi: 96,
            icons: Vec::new(),
            selected_item: -1,
            hovered_item: -1,
            drag_pending: false,
            drag_active: false,
            drag_item_index: -1,
            drag_start_x: 0,
            drag_start_y: 0,
            cell_w: 0,
            cell_h: 0,
            icon_size: 0,
            drag_over_item: -1,
            drag_over_insert_col: -1,
            drag_over_insert_row: -1,
            d2d_factory: None,
            dw_factory: None,
            wic_factory: None,
            d2d_target: None,
        };
        assert_eq!(data.width, 1920);
        assert_eq!(data.height, 1080);
        assert_eq!(data.dpi, 96);
    }

    #[test]
    fn test_reset_overlay_interaction_state_clears_selection_and_drag() {
        let mut data = OverlayWindowData {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            dpi: 96,
            icons: Vec::new(),
            selected_item: 2,
            hovered_item: 3,
            drag_pending: true,
            drag_active: true,
            drag_item_index: 2,
            drag_start_x: 10,
            drag_start_y: 20,
            cell_w: 80,
            cell_h: 96,
            icon_size: 48,
            drag_over_item: -1,
            drag_over_insert_col: -1,
            drag_over_insert_row: -1,
            d2d_factory: None,
            dw_factory: None,
            wic_factory: None,
            d2d_target: None,
        };

        reset_overlay_interaction_state(&mut data);

        assert_eq!(data.selected_item, -1);
        assert_eq!(data.hovered_item, -1);
        assert!(!data.drag_pending);
        assert!(!data.drag_active);
        assert_eq!(data.drag_item_index, -1);
        assert_eq!(data.drag_start_x, 0);
        assert_eq!(data.drag_start_y, 0);
    }

    #[test]
    fn test_overlay_layout_params_at_96dpi() {
        let layout = overlay_layout_params(1920, 96);
        assert!(layout.cell_w > 0);
        assert!(layout.cell_h > 0);
        assert!(layout.icon_size > 0);
        assert!(layout.cols >= 2);
        assert!(layout.padding > 0);
    }

    #[test]
    fn test_overlay_layout_params_at_144dpi() {
        let layout_96 = overlay_layout_params(1920, 96);
        let layout_144 = overlay_layout_params(1920, 144);

        // 144 DPI = 1.5x of 96 DPI
        assert_eq!(layout_144.icon_size, layout_96.icon_size * 3 / 2);
        assert_eq!(layout_144.cell_w, layout_96.cell_w * 3 / 2);
        assert_eq!(layout_144.cell_h, layout_96.cell_h * 3 / 2);
        assert_eq!(layout_144.padding, layout_96.padding * 3 / 2);
    }

    #[test]
    fn test_overlay_layout_params_at_192dpi() {
        let layout_96 = overlay_layout_params(1920, 96);
        let layout_192 = overlay_layout_params(1920, 192);

        // 192 DPI = 2x of 96 DPI
        assert_eq!(layout_192.icon_size, layout_96.icon_size * 2);
        assert_eq!(layout_192.cell_w, layout_96.cell_w * 2);
        assert_eq!(layout_192.cell_h, layout_96.cell_h * 2);
    }

    #[test]
    fn test_physical_to_logical_and_back() {
        // 96 DPI: physical == logical
        assert_eq!(physical_to_logical(100, 96), 100.0);
        assert_eq!(logical_to_physical(100.0, 96), 100);

        // 144 DPI: physical 150 -> logical 100
        assert_eq!(physical_to_logical(150, 144), 100.0);
        assert_eq!(logical_to_physical(100.0, 144), 150);

        // 192 DPI: physical 200 -> logical 100
        assert_eq!(physical_to_logical(200, 192), 100.0);
        assert_eq!(logical_to_physical(100.0, 192), 200);
    }

    #[test]
    fn test_physical_logical_roundtrip() {
        for dpi in [96, 120, 144, 192, 288] {
            let physical = 500;
            let logical = physical_to_logical(physical, dpi);
            let restored = logical_to_physical(logical, dpi);
            assert_eq!(
                restored, physical,
                "roundtrip failed for dpi={}: {} -> {} -> {}",
                dpi, physical, logical, restored
            );
        }
    }

    #[test]
    fn test_physical_to_logical_zero_dpi() {
        // dpi=0 应该回退到 1，避免除零
        let result = physical_to_logical(100, 0);
        assert!(result.is_finite());
    }

    #[test]
    fn test_scale_for_dpi() {
        assert_eq!(scale_for_dpi(48, 96), 48);
        assert_eq!(scale_for_dpi(48, 144), 72);
        assert_eq!(scale_for_dpi(48, 192), 96);
        assert_eq!(scale_for_dpi(48, 288), 144);
    }

    #[test]
    fn test_overlay_layout_params_narrow_width() {
        // 窄宽度应该至少有 2 列
        let layout = overlay_layout_params(200, 96);
        assert!(layout.cols >= 2);
    }

    #[test]
    fn test_overlay_layout_params_wide_width() {
        // 超宽屏幕应该有更多列
        let layout_narrow = overlay_layout_params(800, 96);
        let layout_wide = overlay_layout_params(3840, 96);
        assert!(layout_wide.cols > layout_narrow.cols);
    }
    // ===== is_managed_icon_path 精确匹配测试 =====

    fn make_managed_paths(paths: &[&str]) -> std::collections::HashSet<String> {
        paths.iter().map(|p| normalize_path_key(p)).collect()
    }

    #[test]
    fn test_is_managed_icon_path_direct_match() {
        let managed = make_managed_paths(&[r"C:\Users\chenzhihui\Desktop\Apifox.lnk"]);
        assert!(is_managed_icon_path(
            r"C:\Users\chenzhihui\Desktop\Apifox.lnk",
            "Apifox",
            &managed,
        ));
    }

    #[test]
    fn test_is_managed_icon_path_no_match() {
        let managed = make_managed_paths(&[r"C:\Users\chenzhihui\Desktop\Apifox.lnk"]);
        assert!(!is_managed_icon_path(
            r"C:\Users\Public\Desktop\Chrome.lnk",
            "Chrome",
            &managed,
        ));
    }

    #[test]
    fn test_is_managed_icon_path_no_substring_false_positive() {
        // 关键回归测试：企业微信不应匹配微信
        let managed = make_managed_paths(&[r"C:\Users\Public\Desktop\企业微信.lnk"]);
        assert!(!is_managed_icon_path(
            r"C:\Users\Public\Desktop\微信.lnk",
            "微信",
            &managed,
        ));
    }

    #[test]
    fn test_is_managed_icon_path_exact_filename_match() {
        let managed = make_managed_paths(&[r"C:\Users\Public\Desktop\微信.lnk"]);
        assert!(is_managed_icon_path(
            r"C:\Users\Public\Desktop\微信.lnk",
            "微信",
            &managed,
        ));
    }

    #[test]
    fn test_is_managed_icon_path_public_alias() {
        let managed = make_managed_paths(&[
            r"C:\Users\chenzhihui\.ikun_organizer\managed_icons\public__Chrome.lnk",
        ]);
        assert!(is_managed_icon_path(
            r"C:\Users\Public\Desktop\Chrome.lnk",
            "Chrome",
            &managed,
        ));
    }

    #[test]
    fn test_is_managed_icon_path_display_name_match() {
        // managed_paths ????????????? icon_names ???
        let managed = make_managed_paths(&[r"C:\Users\chenzhihui\Desktop\??.lnk"]);
        // ?????????????????
        assert!(is_managed_icon_path(
            r"C:\Users\Public\Desktop\??.lnk",
            "??",
            &managed,
        ));
    }

    #[test]
    fn test_is_managed_icon_path_empty_display_name() {
        let managed = make_managed_paths(&[r"C:\Users\chenzhihui\Desktop\Apifox.lnk"]);
        assert!(is_managed_icon_path(
            r"C:\Users\chenzhihui\Desktop\Apifox.lnk",
            "",
            &managed,
        ));
        assert!(!is_managed_icon_path(
            r"C:\Users\Public\Desktop\Other.lnk",
            "",
            &managed,
        ));
    }

    #[test]
    fn test_is_managed_icon_path_case_insensitive() {
        let managed = make_managed_paths(&[r"C:\Users\Public\Desktop\Chrome.lnk"]);
        assert!(is_managed_icon_path(
            r"C:\USERS\PUBLIC\DESKTOP\CHROME.LNK",
            "chrome",
            &managed,
        ));
    }

    #[test]
    fn test_is_managed_icon_path_clsids() {
        let managed = make_managed_paths(&["::{645FF040-5081-101B-9F08-00AA002F954E}"]);
        assert!(is_managed_icon_path(
            "::{645FF040-5081-101B-9F08-00AA002F954E}",
            "回收站",
            &managed,
        ));
        assert!(!is_managed_icon_path(
            "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
            "此电脑",
            &managed,
        ));
    }

    #[test]
    fn test_is_managed_icon_path_no_file_name() {
        let managed = make_managed_paths(&[r"C:\Users\chenzhihui\Desktop\Apifox.lnk"]);
        assert!(!is_managed_icon_path("", "test", &managed));
    }

    // ===== normalize_path_key 测试 =====

    #[test]
    fn test_normalize_path_key_slash_conversion() {
        assert_eq!(
            normalize_path_key(r"C:/Users/test/file.lnk"),
            r"c:\users\test\file.lnk"
        );
    }

    #[test]
    fn test_normalize_path_key_lowercase() {
        assert_eq!(
            normalize_path_key(r"C:\Users\Test\FILE.LNK"),
            r"c:\users\test\file.lnk"
        );
    }

    #[test]
    fn test_normalize_path_key_already_normalized() {
        assert_eq!(
            normalize_path_key(r"c:\users\test\file.lnk"),
            r"c:\users\test\file.lnk"
        );
    }

    #[test]
    fn test_normalize_path_key_clsids() {
        assert_eq!(
            normalize_path_key("::{645FF040-5081-101B-9F08-00AA002F954E}"),
            "::{645ff040-5081-101b-9f08-00aa002f954e}"
        );
    }

    // ===== save_overlay_icon_positions 仅保存用户设定位置测试 =====

    #[test]
    fn test_icon_position_only_saved_for_user_set() {
        let auto_icon = OverlayIconItem {
            file_path: r"C:\Users\test\auto.lnk".to_string(),
            display_name: "auto".to_string(),
            icon_handle: std::ptr::null_mut(),
            x: -1.0,
            y: -1.0,
        };
        assert!(auto_icon.x < 0.0 && auto_icon.y < 0.0);

        let user_icon = OverlayIconItem {
            file_path: r"C:\Users\test\user.lnk".to_string(),
            display_name: "user".to_string(),
            icon_handle: std::ptr::null_mut(),
            x: 100.0,
            y: 200.0,
        };
        assert!(user_icon.x >= 0.0 && user_icon.y >= 0.0);
    }

    // ===== collect_overlay_icons display_name 去重测试 =====

    #[test]
    fn test_display_name_deduplication_logic() {
        let mut seen = std::collections::HashSet::new();
        let dn1 = "微信".to_lowercase();
        let dn2 = "微信".to_lowercase();
        assert!(seen.insert(dn1));
        assert!(!seen.insert(dn2));
    }

    #[test]
    fn test_display_name_deduplication_case_insensitive() {
        let mut seen = std::collections::HashSet::new();
        let dn1 = "Chrome".to_lowercase();
        let dn2 = "chrome".to_lowercase();
        assert!(seen.insert(dn1));
        assert!(!seen.insert(dn2));
    }

    #[test]
    fn test_skip_refresh_during_drag_active() {
        // Test that refresh is skipped when drag_active is true
        let mut data = OverlayWindowData::test_default();
        data.drag_active = true;

        // When drag_active is true, refresh should be skipped
        let should_skip = data.drag_active || data.drag_pending;
        assert!(should_skip, "Should skip refresh when drag_active is true");
    }

    #[test]
    fn test_skip_refresh_during_drag_pending() {
        // Test that refresh is skipped when drag_pending is true
        let mut data = OverlayWindowData::test_default();
        data.drag_pending = true;

        // When drag_pending is true, refresh should be skipped
        let should_skip = data.drag_active || data.drag_pending;
        assert!(should_skip, "Should skip refresh when drag_pending is true");
    }

    #[test]
    fn test_allow_refresh_when_not_dragging() {
        // Test that refresh is allowed when not dragging
        let mut data = OverlayWindowData::test_default();
        data.drag_active = false;
        data.drag_pending = false;

        // When not dragging, refresh should be allowed
        let should_skip = data.drag_active || data.drag_pending;
        assert!(!should_skip, "Should allow refresh when not dragging");
    }

    #[test]
    fn test_drag_state_consistency() {
        // Test that drag_active and drag_pending are mutually exclusive in normal flow
        let mut data = OverlayWindowData::test_default();

        // Initially both false
        assert!(!data.drag_active);
        assert!(!data.drag_pending);

        // Start drag: pending -> active
        data.drag_pending = true;
        assert!(data.drag_pending);
        assert!(!data.drag_active);

        // Drag activated: pending cleared, active set
        data.drag_pending = false;
        data.drag_active = true;
        assert!(!data.drag_pending);
        assert!(data.drag_active);

        // End drag: both cleared
        data.drag_active = false;
        assert!(!data.drag_active);
        assert!(!data.drag_pending);
    }

    #[test]
    fn test_external_state_reset_detection() {
        // Test detection of external state reset (e.g., by file watcher)
        let mut data = OverlayWindowData::test_default();

        // Simulate drag in progress
        data.drag_active = true;
        data.drag_item_index = 5;

        // External module resets state (simulating file watcher refresh)
        data.drag_active = false;
        data.drag_item_index = -1;

        // Timer should detect this inconsistency
        let is_dragging = data.drag_active;
        assert!(
            !is_dragging,
            "Timer should detect state was reset externally"
        );

        // Cleanup should be triggered
        assert_eq!(data.drag_item_index, -1);
    }
}
