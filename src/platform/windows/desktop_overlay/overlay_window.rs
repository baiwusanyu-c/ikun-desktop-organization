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
    /// D2D Factory
    pub(crate) d2d_factory: Option<ID2D1Factory>,
    /// DirectWrite Factory
    pub(crate) dw_factory: Option<IDWriteFactory>,
    /// WIC Factory
    pub(crate) wic_factory: Option<IWICImagingFactory>,
    /// DC RenderTarget
    pub(crate) d2d_target: Option<ID2D1DCRenderTarget>,
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
    let (x, y, width, height) =
        crate::platform::windows::grid_window::get_cursor_monitor_work_area_phys();

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

    if data.icons.iter().any(|item| item.file_path == file_path) {
        log::debug!("框→覆盖窗口: 覆盖窗口已存在该图标，跳过 {}", display_name);
        return true;
    }

    let position = if drop_x >= 0 && drop_y >= 0 {
        let logical_x = physical_to_logical(drop_x.saturating_sub(data.x), data.dpi);
        let logical_y = physical_to_logical(drop_y.saturating_sub(data.y), data.dpi);
        Some([logical_x, logical_y])
    } else {
        None
    };

    let icon_handle = crate::platform::windows::extract_file_icon(&file_path)
        .map(|info| info.hicon)
        .unwrap_or(std::ptr::null_mut());
    data.icons.push(OverlayIconItem::with_position(
        file_path.clone(),
        display_name.clone(),
        icon_handle,
        position,
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
    let overlay_config = AppConfig::load()
        .map(|config| config.overlay)
        .unwrap_or_else(|e| {
            log::warn!("覆盖窗口读取位置配置失败，使用默认位置: {}", e);
            OverlayConfig::default()
        });
    for icon in manager.get_desktop_icons() {
        let path = icon.path.to_string_lossy().to_string();
        if is_managed_icon_path(&path, &icon.display_name, &managed_paths) {
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
    let Some(stem) = Path::new(path).file_stem().and_then(|name| name.to_str()) else {
        return false;
    };

    let public_alias = format!("public__{}", file_name);
    let public_stem_alias = format!("public__{}", stem);
    managed_paths.iter().any(|managed| {
        managed.ends_with(&normalize_path_key(&public_alias))
            || managed.ends_with(&normalize_path_key(&public_stem_alias))
            || managed.ends_with(&normalize_path_key(file_name))
            || managed.ends_with(&normalize_path_key(display_name))
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
                let position = resolved_icon_position(data, icon);
                positions.insert(icon.file_path.clone(), position);
                positions.insert(normalize_path_key(&icon.file_path), position);
            }
        }
    }

    let mut config = AppConfig::load().unwrap_or_else(|e| {
        log::warn!("保存覆盖窗口位置前加载配置失败，使用默认配置: {}", e);
        AppConfig::default()
    });
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
    let col = (index as i32) % layout.cols;
    let row = (index as i32) / layout.cols;
    [
        physical_to_logical(layout.padding + col * layout.cell_w, data.dpi),
        physical_to_logical(layout.padding + row * layout.cell_h, data.dpi),
    ]
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
pub(crate) fn overlay_hit_test(data: &OverlayWindowData, x: i32, y: i32) -> i32 {
    let layout = overlay_layout_params(data.width, data.dpi);
    for (index, icon) in data.icons.iter().enumerate() {
        if icon.x >= 0.0 && icon.y >= 0.0 {
            let cell_x = logical_to_physical(icon.x, data.dpi);
            let cell_y = logical_to_physical(icon.y, data.dpi);
            if x >= cell_x
                && x < cell_x + layout.cell_w
                && y >= cell_y
                && y < cell_y + layout.cell_h
            {
                return index as i32;
            }
        }
    }

    if x < layout.padding || y < layout.padding || layout.cell_w <= 0 || layout.cell_h <= 0 {
        return -1;
    }

    let col = (x - layout.padding) / layout.cell_w;
    let row = (y - layout.padding) / layout.cell_h;
    if col < 0 || row < 0 || col >= layout.cols {
        return -1;
    }

    let idx = row * layout.cols + col;
    if idx < 0 || (idx as usize) >= data.icons.len() {
        return -1;
    }

    let cell_x = overlay_icon_cell_x(data, idx as usize, &layout);
    let cell_y = overlay_icon_cell_y(data, idx as usize, &layout);
    if x >= cell_x && x < cell_x + layout.cell_w && y >= cell_y && y < cell_y + layout.cell_h {
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
    let col = (index as i32) % layout.cols;
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
    let row = (index as i32) / layout.cols;
    layout.padding + row * layout.cell_h
}

fn scale_for_dpi(value: i32, dpi: u32) -> i32 {
    ((value as f32) * dpi as f32 / 96.0).round() as i32
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
}
