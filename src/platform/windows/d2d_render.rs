//! Direct2D/DirectWrite 渲染代码

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! 本模块包含使用 Direct2D 和 DirectWrite 渲染格子窗口内容的所有代码。
//! 包括设备初始化、分层窗口渲染和图标绘制。
//!
//! # 图标渲染架构
//!
//! `render_icon_grid` 是独立的图标渲染方法，通过参数驱动：
//! - `GridLayout`：布局位置（cols/cell_w/cell_h/grid_left/grid_top）
//! - `ViewParams`：视图模式参数（icon_size/show_label 等）
//!
//! HICON 转换为 D2D 位图时会裁剪外围透明画布，并在目标图标区域内保持
//! 内容宽高比居中；图标区域尺寸仍由 `ViewParams`/`GridLayout` 决定。
//!
//! 支持 4 种图标模式（ExtraLarge/Large/Medium/Small），未来扩展列表/详细信息/平铺/内容。

use crate::box_core::layout_calc::{calculate_grid, GridLayout};
use crate::box_core::view_engine::{get_view_params, ViewParams};

use super::grid_window::GridFileItem;
use super::icon_bitmap::{icon_destination_rect, visible_pixel_bounds};

use std::collections::HashMap;
use std::ffi::c_void;

use windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED;
use windows::Win32::Graphics::Direct2D::{
    Common::{D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U},
    ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Image, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES,
    D2D1_BITMAP_PROPERTIES1, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::DXGI_PRESENT;
use windows::Win32::Graphics::Gdi::HDC as GdiHDC;
use windows::Win32::Graphics::Imaging::IWICImagingFactory;
use windows_numerics::{Matrix3x2, Vector2};

use super::ffi_imports::*;
use super::ffi_types::*;
use super::grid_window::{DcompState, GridWindowData};
use super::render_devices::TextFormatSpec;

/// 初始化窗口的 D2D DC 渲染目标（首次渲染时调用）。
///
/// Direct2D / DirectWrite / WIC 三个工厂已收敛到 render_devices 按渲染线程共享：
/// 每个 ID2D1Factory 内部都持有一个独立 D3D11 设备，按窗口各建一套会让
/// 5 个分类框额外占用约 200MB 私有内存。
///
/// DC 渲染目标仍按窗口持有：它需要与各窗口自己的 DIB 绑定，
/// 共享同一个 target 会在并发 BindDC 时互相踩画面。
pub fn init_d2d_devices(data: &mut GridWindowData) {
    if data.d2d_target.is_some() {
        return;
    }
    if let Some(target) = super::render_devices::create_dc_render_target(0.0) {
        data.d2d_target = Some(target);
        log::info!("D2D DC RenderTarget 已创建（共享 D2D 工厂）");
    }
}

/// 丢弃图标位图缓存中已不再被任何条目引用的 HICON。
///
/// 缓存以 `HICON` 地址为键，而图标句柄在条目被删除 / 替换时就会
/// `DestroyIcon`。旧实现从不清理，于是：
/// - 缓存单调增长，长期运行后每个分类框都背着一堆无主位图；
/// - 句柄地址被系统回收复用后会命中过期条目，画出错误图标。
///
/// 键空间实际上只有「可见图标数 × 图标尺寸种类」（尺寸同一时刻只有一种），
/// 因此只在超出预算时才做一次全量剪枝，正常帧零开销。
fn prune_icon_bitmap_cache(data: &mut GridWindowData) {
    // 预留 4 倍余量：切换视图模式会短暂共存新旧两种图标尺寸
    let budget = data.items.len() * 4 + 16;
    if data.icon_bitmap_cache.len() <= budget {
        return;
    }
    let live: Vec<usize> = data
        .items
        .iter()
        .map(|item| item.icon_handle as usize)
        .collect();
    let before = data.icon_bitmap_cache.len();
    data.icon_bitmap_cache
        .retain(|(hicon, _), _| live.contains(hicon));
    log::debug!(
        "图标位图缓存剪枝: {} → {} 条",
        before,
        data.icon_bitmap_cache.len()
    );
}

/// 释放内容层缓存（DIB + DC + D2D 位图）。
///
/// 内容层与主 DIB 尺寸永远一致，所以两者必须同生同灭：
/// 只释放主 DIB 会让下一帧拿着旧尺寸的 `content_bitmap` 去合成，
/// 表现为内容整体错位或只覆盖窗口一角，而且不会报错。
fn release_content_layer(data: &mut GridWindowData) {
    data.content_bitmap = None;
    data.content_valid = false;
    if !data.content_dc.is_null() {
        // D2D 目标可能还绑在这个 DC 上，先解绑再删对象
        if let Some(ref target) = data.d2d_target {
            let empty = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            let _ = unsafe { target.BindDC(GdiHDC(std::ptr::null_mut()), &empty) };
        }
        unsafe {
            if !data.content_old_bmp.is_null() {
                SelectObject(data.content_dc, data.content_old_bmp);
            }
            if !data.content_dib.is_null() {
                DeleteObject(data.content_dib as HGDIOBJ);
            }
            DeleteDC(data.content_dc);
        }
    }
    data.content_dib = std::ptr::null_mut();
    data.content_dc = std::ptr::null_mut();
    data.content_old_bmp = std::ptr::null_mut();
    data.content_bits = std::ptr::null_mut();
    data.content_w = 0;
    data.content_h = 0;
}

/// 释放分类框复用的 DIB 渲染后端。
///
/// 先把 D2D DC 渲染目标从旧 DC 解绑，再还原默认位图并删除 GDI 对象，
/// 避免 D2D 继续向已删除的 DC 提交画面。窗口销毁和尺寸变化时调用。
///
/// 内容层缓存一并释放：它的尺寸与主 DIB 绑定，留着必然对不上。
pub(crate) fn release_grid_dib(data: &mut GridWindowData) {
    release_content_layer(data);
    if data.dib_dc.is_null() {
        return;
    }
    if let Some(ref target) = data.d2d_target {
        let empty = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let _ = unsafe { target.BindDC(GdiHDC(std::ptr::null_mut()), &empty) };
    }
    unsafe {
        if !data.dib_old_bmp.is_null() {
            SelectObject(data.dib_dc, data.dib_old_bmp);
        }
        if !data.dib_section.is_null() {
            DeleteObject(data.dib_section as HGDIOBJ);
        }
        DeleteDC(data.dib_dc);
    }
    data.dib_section = std::ptr::null_mut();
    data.dib_dc = std::ptr::null_mut();
    data.dib_old_bmp = std::ptr::null_mut();
    // DIB 已 DeleteObject，像素地址随之失效；不清空会让下一帧的玻璃
    // 直写落到已释放内存上 —— 不会立刻崩，只会随机写坏别的堆块。
    data.dib_bits = std::ptr::null_mut();
    data.dib_w = 0;
    data.dib_h = 0;
}

/// 确保分类框持有一个与 width × height 匹配的 DIB 渲染后端。
///
/// 尺寸一致时直接复用（绝大多数帧命中这条路径）；尺寸变化时先释放旧后端再重建。
fn ensure_grid_dib(data: &mut GridWindowData, width: i32, height: i32) -> bool {
    if !data.dib_dc.is_null() && data.dib_w == width && data.dib_h == height {
        return true;
    }
    release_grid_dib(data);
    if width <= 0 || height <= 0 {
        return false;
    }

    unsafe {
        // 屏幕 DC 仅用于创建兼容 DC，创建完成后立即归还
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return false;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        if mem_dc.is_null() {
            return false;
        }

        let bi = BITMAPINFOHEADER {
            bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            bi_width: width,
            bi_height: -height, // 自上而下
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
        let dib = CreateDIBSection(
            mem_dc,
            &bi as *const BITMAPINFOHEADER as *const c_void,
            DIB_RGB_COLORS,
            &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
            std::ptr::null_mut(),
            0,
        );
        if dib.is_null() || bits.is_null() {
            DeleteDC(mem_dc);
            return false;
        }

        data.dib_old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);
        data.dib_section = dib;
        data.dib_dc = mem_dc;
        data.dib_bits = bits;
        data.dib_w = width;
        data.dib_h = height;
        true
    }
}

/// 取（必要时创建并缓存）该渲染目标上的纯色画刷。
///
/// 返回 `windows::core::Result` 而不是 `Option`，是为了让所有调用点
/// 保持原来 `if let Ok(brush) = ...` 的写法，改动面最小也最不容易改错。
/// 缓存的意义与安全性论证见 `render_devices::solid_brush_rgba`。
pub(crate) fn cached_brush(
    target: &ID2D1DCRenderTarget,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> windows::core::Result<windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush> {
    match super::render_devices::solid_brush_rgba(target, r, g, b, a) {
        Some(brush) => Ok(brush),
        // 缓存这条路不该失败；真拿不到就照旧直接创建，错误交给 D2D 报
        None => unsafe { target.CreateSolidColorBrush(&d2d_color(r, g, b, a), None) },
    }
}

/// 确保内容层持有一个与 width × height 匹配的 DIB 后端。
///
/// 与 `ensure_grid_dib` 同构：尺寸一致直接复用（绝大多数帧命中），
/// 变化时先释放再重建。返回 false 表示建不出来，调用方必须退回
/// 「每帧全量重画」的老路，绝不能拿着旧尺寸的内容层去合成。
fn ensure_content_dib(data: &mut GridWindowData, width: i32, height: i32) -> bool {
    if !data.content_dc.is_null() && data.content_w == width && data.content_h == height {
        return true;
    }
    release_content_layer(data);
    if width <= 0 || height <= 0 {
        return false;
    }
    unsafe {
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return false;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        if mem_dc.is_null() {
            return false;
        }
        let bi = BITMAPINFOHEADER {
            bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            bi_width: width,
            bi_height: -height, // 自上而下，与主 DIB 一致
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
        let dib = CreateDIBSection(
            mem_dc,
            &bi as *const BITMAPINFOHEADER as *const c_void,
            DIB_RGB_COLORS,
            &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
            std::ptr::null_mut(),
            0,
        );
        if dib.is_null() || bits.is_null() {
            DeleteDC(mem_dc);
            return false;
        }
        data.content_old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);
        data.content_dib = dib;
        data.content_dc = mem_dc;
        data.content_bits = bits;
        data.content_w = width;
        data.content_h = height;
        true
    }
}

// =============================================================================
// 内容层指纹
// =============================================================================

/// 极简 FNV-1a 64 位累加器。
///
/// 内容层指纹每帧算一次（每框每秒最多 5 次），要哈希的字节数在几 KB 量级，
/// 逐字节就够快；换成 `DefaultHasher` 反而要引入 `Hash` 派生，
/// 而 `GridWindowData` 里有裸指针字段，派生不了。
struct FnvHasher(u64);

impl FnvHasher {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, value: &[u8]) {
        for byte in value {
            self.0 ^= *byte as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }
    fn f32(&mut self, value: f32) {
        // 用位模式，避免 -0.0 / NaN 之类的相等性陷阱
        self.bytes(&value.to_bits().to_le_bytes());
    }
    fn flag(&mut self, value: bool) {
        self.bytes(&[u8::from(value)]);
    }
    fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
        self.bytes(&[0]); // 分隔符：防止 ("ab","c") 与 ("a","bc") 撞同一个指纹
    }
    fn finish(self) -> u64 {
        self.0
    }
}

/// 让 `FnvHasher` 能直接吃 `std::hash::Hash`。
///
/// `ViewMode` / `SortMode` / `GroupBy` 都不是 `Copy`，没法 `as i32`；
/// 而 `std::mem::discriminant` 返回的 `Discriminant<T>` 实现了 `Hash`，
/// 正好用来把「枚举是哪一个变体」稳定地混进指纹，且**零分配**
/// （用 `format!("{:?}")` 也能做到，但那每次要分配一个小字符串，
/// 而这条路径每框每秒要跑 5 次，没必要给堆添堵）。
impl std::hash::Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        self.bytes(bytes);
    }
}

/// 算出「玻璃之上那一层」的状态指纹。
///
/// # 漏字段的后果，以及为什么它仍然安全
///
/// 指纹只在**周期 tick** 这条路上用来判断能不能复用内容层。
/// 所有显式重绘入口（`WM_*` 处理、拖拽、菜单、配置变更、增删条目）
/// 走的都是全量重画，不看指纹。所以漏掉某个字段的后果不是画面错误，
/// 只是那个字段变化时少省一次重画 —— 方向是安全的。
///
/// 反过来，**多列字段没有代价**（顶多多重画一次），所以这里宁滥勿缺，
/// 把 `GridWindowData` 里所有可能影响内容的字段全部列进来。
fn content_fingerprint(
    data: &GridWindowData,
    w: i32,
    h: i32,
    dpi: u32,
    style: &crate::config::BoxStyleConfig,
) -> u64 {
    let mut f = FnvHasher::new();
    // 几何与 DPI
    f.i32(w);
    f.i32(h);
    f.u32(dpi);
    f.i32(data.render_padding);
    f.i32(data.title_h);
    f.i32(data.cell_w);
    f.i32(data.cell_h);
    f.i32(data.cols);
    f.i32(data.icon_size);
    f.i32(data.scroll_offset);
    // 标题与按钮状态
    f.text(&data.title);
    f.flag(data.locked);
    f.flag(data.lock_hovered);
    f.flag(data.more_hovered);
    f.i32(data.hover_edge);
    f.flag(data.menu.open);
    f.i32(data.menu.hovered_item);
    f.i32(data.menu.expanded_item);
    f.i32(data.menu.sub_hovered);
    f.flag(data.menu.sub_on_left);
    // 选中 / 多选 / 框选
    f.i32(data.selected_item);
    f.i32(data.hovered_item);
    f.i32(data.shift_anchor);
    f.flag(data.marquee_active);
    f.i32(data.marquee_start_x);
    f.i32(data.marquee_start_y);
    f.i32(data.marquee_current_x);
    f.i32(data.marquee_current_y);
    f.i32(data.multi_drag_items.len() as i32);
    for index in &data.multi_drag_items {
        f.i32(*index as i32);
    }
    // 拖拽覆盖反馈
    f.i32(data.drag_over_item);
    f.i32(data.drag_over_insert_col);
    f.i32(data.drag_over_insert_row);
    f.flag(data.drop_highlight);
    f.i32(data.renaming_item_index);
    // 视图 / 排序 / 分组
    {
        use std::hash::Hash;
        let mut hasher = FnvHasher(f.0);
        std::mem::discriminant(&data.view_mode).hash(&mut hasher);
        std::mem::discriminant(&data.sort_mode).hash(&mut hasher);
        std::mem::discriminant(&data.group_by).hash(&mut hasher);
        f.u64(hasher.finish());
    }
    f.flag(data.sort_ascending);
    f.bytes(data.config_id.as_bytes());
    // 条目本体：显示名 + 图标句柄 + 手工摆放的行列
    f.i32(data.items.len() as i32);
    for item in &data.items {
        f.text(&item.display_name);
        f.u64(item.icon_handle as usize as u64);
        f.i32(item.grid_col);
        f.i32(item.grid_row);
    }
    // 样式里影响内容层的字段（背景色/透明度属于玻璃那一趟，不在这里）
    f.flag(style.icon_only_mode);
    f.f32(style.title_font_size);
    f.text(&style.title_font_family);
    f.bytes(&style.title_font_color);
    f.f32(style.label_font_size);
    f.bytes(&style.label_font_color);
    f.i32(style.icon_size);
    f.i32(style.cell_spacing);
    f.i32(style.title_bar_height);
    f.bytes(&style.title_bar_color);
    f.bytes(&style.highlight_color);
    f.bytes(&style.hover_color);
    f.i32(style.columns);
    f.flag(style.lock_button_visible);
    f.flag(style.close_button_visible);
    f.finish()
}

/// 渲染分层窗口内容（Direct2D/DirectWrite → DIB → UpdateLayeredWindow）
///
/// 背景 rgba(1,1,1,0.15) = alpha 38/255 ≈ 15% 不透明
/// 标题文字和图标以完全不透明（alpha=255）绘制
///
/// 重要：使用 GetWindowRect（而非 GetClientRect）获取尺寸。
/// 对 WS_POPUP|WS_THICKFRAME 分层窗口，UpdateLayeredWindow 的 psize 参数
/// 会直接设置窗口尺寸。
pub fn render_layered_window(hwnd: HWND) {
    render_layered_window_inner(hwnd, false)
}

/// 周期 tick 专用的重绘入口：**允许复用内容层缓存**。
///
/// 与 `render_layered_window` 的唯一区别是 `glass_only = true`：
/// 内容层（图标/标签/标题/按钮/各种交互反馈）状态指纹没变就不重画，
/// 每帧只重画玻璃背景再做一次合成。所有显式重绘入口都必须继续走
/// `render_layered_window`（全量），否则可能停在旧内容上。
pub(crate) fn render_layered_window_glass_only(hwnd: HWND) {
    render_layered_window_inner(hwnd, true)
}

fn render_layered_window_inner(hwnd: HWND, glass_only: bool) {
    // 递归保护：防止 UpdateLayeredWindow 触发 WM_SIZE → 再次进入本函数
    use std::sync::atomic::{AtomicBool, Ordering};
    static RENDERING: AtomicBool = AtomicBool::new(false);
    static LAYERED_SUBMIT_FAILED: AtomicBool = AtomicBool::new(false);
    if RENDERING.swap(true, Ordering::SeqCst) {
        return; // 已在渲染中，跳过
    }

    let data = unsafe {
        match get_data(hwnd) {
            Some(d) => d,
            None => {
                RENDERING.store(false, Ordering::SeqCst);
                return;
            }
        }
    };

    // 确保窗口的 DC 渲染目标已初始化（D2D/DW/WIC 工厂按渲染线程共享）
    init_d2d_devices(data);
    prune_icon_bitmap_cache(data);

    let (Some(dw_factory), Some(wic_factory), Some(target)) = (
        super::render_devices::dw_factory(),
        super::render_devices::wic_factory(),
        data.d2d_target.clone(),
    ) else {
        log::warn!("D2D 设备未初始化，跳过渲染");
        RENDERING.store(false, Ordering::SeqCst);
        return;
    };

    // 使用 GetWindowRect 获取完整窗口尺寸
    let mut wr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut wr) };
    let outer_w = wr.right - wr.left;
    let outer_h = wr.bottom - wr.top;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    // 留白取窗口当前生效值，不能每帧重新按 DPI 推算：
    // 玻璃开关刚切换、窗口外框还没收缩完时，重新推算会让本帧
    // 按新留白在旧外框上作画，表现为内容整体错位一帧。
    let mut shadow_padding = data.render_padding;
    // 安全钳制：外框比 2×外投影留白还小时（192 DPI 尺寸自适应未生效等
    // 遗留问题会让外框停在 96 DPI 的尺寸、而留白按 192 DPI 算），
    // 内容宽高会算成负数，`w<=0||h<=0` 直接 return，分类框**整块不画** ——
    // 不崩、不报错、日志干净，是最难发现的一类故障。
    // 钳到「至少留出 8px 内容」，保证分类框永远可见。
    let max_padding = ((outer_w.min(outer_h) - 8) / 2).max(0);
    if shadow_padding > max_padding {
        shadow_padding = max_padding;
    }
    let w = outer_w - shadow_padding * 2;
    let h = outer_h - shadow_padding * 2;
    if w <= 0 || h <= 0 || outer_w <= 0 || outer_h <= 0 {
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    let ww = w as f32;
    let hh = h as f32;

    // ===== 复用的 32 位 ARGB DIB Section（D2D 渲染后端）=====
    //
    // 旧实现每帧 CreateDIBSection + DeleteObject：分类框定时器 200ms 一次、
    // 5 个框即每秒上百 MB 的 GDI 分配/释放，是主进程内存持续爬升的主因之一。
    // 现在 DIB 与内存 DC 存放在 GridWindowData 中，仅尺寸变化或窗口销毁时重建。
    // DComp 呈现路径的画面直接进合成交换链，**根本不经过 GDI DIB**。
    // 跳过它每框省一整张 outer_w × outer_h 的 32bpp 位图（5 框合计 24MB）。
    let dcomp_active = data.dcomp.is_some();
    if !dcomp_active && !ensure_grid_dib(data, outer_w, outer_h) {
        log::error!("分类框 DIB 渲染后端创建失败: {outer_w}x{outer_h}");
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }
    let mem_dc = data.dib_dc;

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    if screen_dc.is_null() {
        log::error!("分类框渲染获取屏幕 DC 失败");
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    // 绑定用的矩形：内容层与合成层同尺寸（都是整窗外框，含外投影留白）
    let bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: outer_w,
        bottom: outer_h,
    };
    let identity = Matrix3x2 {
        M11: 1.0,
        M22: 1.0,
        ..Default::default()
    };
    // 清空为全透明（premultiplied alpha: 所有通道为 0）
    let transparent = D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    // ===== 两趟共用的参数 =====
    let box_style = super::liquid_glass::get_box_style_config();
    let glass_config = box_style.liquid_glass.clone();
    let dpi_scale = dpi as f32 / 96.0;
    // border_radius 为逻辑像素值；render_background 内部会自行乘以 dpi_scale
    let corner_radius = box_style.border_radius;
    // D2D 坐标系为物理像素，回退路径需手动缩放以与液态玻璃路径保持一致
    let fallback_radius = box_style.border_radius * dpi_scale;

    // 内容层绑定矩形：**只覆盖内容矩形，不含 128px 外投影留白**。
    //
    // 内容层里一个像素都不会画到留白区（绘制全部用内容局部坐标，
    // 图标区还额外被 clip_rect 夹在 (0,title_h)-(w,h) 内，实测内容层
    // 那 398 行里负坐标/减 padding 的绘制为 0 处）。所以留白那一圈
    // 在内容层里恒为全透明 —— 为它分配 DIB、每帧再做一次全窗
    // 软件 blit，纯属白烧。
    //
    // 按内容矩形建层之后：blit 面积与内存都按 (w·h)/((w+2p)·(h+2p))
    // 缩小（128px 留白下约 -40%），合成时用 destRect 平移回留白之内。
    // 但内容层**不能**刚好等于内容矩形：标题与锁定/更多按钮的描边阴影
    // 会画到 `origin.Y = -shadow_offset`（96 DPI 下 -1px），图标区之外
    // 没有 clip 兜着。旧的整窗内容层有 128px 留白吸收它，收紧之后
    // 就会被裁掉一行 —— 那是静默的视觉回归，看日志和返回值都发现不了。
    //
    // 所以四周各外扩 `content_margin`：足以接住 ±1px 描边加抗锯齿溢出，
    // 相对 128px 留白仍可忽略（(w+4)(h+4) vs (w+256)(h+256)）。
    // 绘制时平移 margin 进来，合成时再平移回 (padding - margin)。
    let content_margin = scale_for_dpi(2, dpi);
    let content_w = w + content_margin * 2;
    let content_h = h + content_margin * 2;
    let content_bind_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: content_w,
        bottom: content_h,
    };
    let content_transform = Matrix3x2 {
        M11: 1.0,
        M22: 1.0,
        M31: content_margin as f32,
        M32: content_margin as f32,
        ..Default::default()
    };

    // ===== 第一趟：内容层（只在状态真的变了时才重画）=====
    //
    // 内容层 = 玻璃之上的一切：分隔线、标题、锁定/更多按钮、图标网格、
    // 选中/hover/框选/拖拽反馈、边缘指示条。
    //
    // 为什么值得缓存：DC 渲染目标是 **GDI 互操作 / 软件光栅化** 路径，
    // 没有硬件加速的余地，每个 draw op 都要从进程堆申请扫描线/跨度/字形
    // scratch。实测一帧 27.6ms 里有 17.4ms（63%）花在这一趟上，
    // 而每个标签要画 5 次 DrawTextLayout（4 方向阴影描边 + 正文），
    // 5 个条目就是 25 次字形光栅化。
    //
    // 而用户最在意的两个高频场景 —— **拖拽分类框**和**动态壁纸** ——
    // 内容层一个像素都没变，变的只有玻璃背景。
    //
    // 只有周期 tick 才允许复用缓存（glass_only=true）：所有显式重绘入口
    // （WM_* 处理、拖拽、菜单、配置变更、增删条目）都走全量重画。
    // 因此「状态变了但没人显式重绘」这种漏网情况由指纹兜住，而指纹
    // 漏字段的后果只是少省一次重画，方向是安全的。
    let fingerprint = content_fingerprint(data, w, h, dpi, &box_style);
    let need_content = !glass_only
        || !data.content_valid
        || data.content_w != content_w
        || data.content_h != content_h
        || data.content_fingerprint != fingerprint
        || data.content_bitmap.is_none();

    // 分段计时：这一趟是软件光栅化，与 GPU 段分开看才知道刀该往哪切。
    // content_started 单独包住第一趟：命中缓存时它应当接近 0，
    // 否则说明指纹每帧都在误判成「变了」，缓存等于白做。
    let dib_started = std::time::Instant::now();
    let content_started = std::time::Instant::now();
    if need_content {
        if !ensure_content_dib(data, content_w, content_h) {
            log::error!("分类框内容层 DIB 创建失败: {content_w}x{content_h}");
            unsafe {
                ReleaseDC(std::ptr::null_mut(), screen_dc);
            }
            RENDERING.store(false, Ordering::SeqCst);
            return;
        }
        let content_dc = data.content_dc;
        if unsafe { target.BindDC(GdiHDC(content_dc), &content_bind_rect) }.is_err() {
            log::error!("内容层 BindDC 失败");
            release_grid_dib(data);
            unsafe {
                ReleaseDC(std::ptr::null_mut(), screen_dc);
            }
            RENDERING.store(false, Ordering::SeqCst);
            return;
        }
        unsafe { target.BeginDraw() };
        unsafe { target.SetTransform(&identity) };
        unsafe { target.Clear(Some(&transparent)) };
        // 平移 margin：内容仍按内容局部坐标绘制（与改造前逐字节一致），
        // 只是落点整体挪进边距里，好接住 -1px 的描边阴影。
        unsafe { target.SetTransform(&content_transform) };

        // ===== 绘制标题栏与主体的分隔线 =====
        let title_h = data.title_h;
        let sep_brush = cached_brush(&target, 100, 100, 100, 100);
        if let Ok(ref brush) = sep_brush {
            let sep_rect = d2d_rect(0.0, (title_h - 1) as f32, ww, title_h as f32);
            unsafe { target.FillRectangle(&sep_rect, brush) };
        }

        // ===== 创建文字画刷（标题专用，图标标签画刷在 render_icon_grid 内创建）=====
        let white_brush = cached_brush(&target, 255, 255, 255, 255);
        let shadow_brush = cached_brush(&target, 0, 0, 0, 200);

        // ===== 标题文字（DirectWrite 渲染）=====
        let title_font_size = scale_for_dpi(15, dpi) as f32;
        // 标题格式：雅黑 UI + 粗体 + 不换行；对齐沿用 DirectWrite 默认的 LEADING / NEAR
        let title_spec = TextFormatSpec::new(
            "Microsoft YaHei UI",
            title_font_size,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            DWRITE_WORD_WRAPPING_NO_WRAP,
        );
        let title_format = super::render_devices::text_format(&title_spec);

        if title_format.is_some() {
            // 锁定按钮区域（标题栏右侧）
            let (btn_x, btn_y, btn_size) = super::grid_layout::lock_btn_rect(w, title_h, dpi);

            let title_text: Vec<u16> = data.title.encode_utf16().collect();
            let text_max_w = (btn_x as f32 - 10.0).max(40.0);
            // 标题的段落对齐在布局上覆盖为 CENTER（文字格式里是 NEAR），
            // 两者不一致，所以对齐必须作为缓存键的一部分显式传入。
            let layout = super::render_devices::text_layout(
                &title_text,
                &title_spec,
                text_max_w,
                title_h as f32,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                false,
            );

            if let Some(ref layout) = layout {
                let shadow_offset = scale_for_dpi(1, dpi) as f32;
                if let Ok(ref sb) = shadow_brush {
                    for &(dx, dy) in &[
                        (shadow_offset, 0.0),
                        (-shadow_offset, 0.0),
                        (0.0, shadow_offset),
                        (0.0, -shadow_offset),
                    ] {
                        let origin = Vector2 {
                            X: 10.0 + dx,
                            Y: dy,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, layout, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                        }
                    }
                }
                if let Ok(ref wb) = white_brush {
                    let origin = Vector2 { X: 10.0, Y: 0.0 };
                    unsafe {
                        target.DrawTextLayout(origin, layout, wb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }

            // ===== 绘制锁定按钮（标题栏右侧）=====
            let lock_icon = if data.locked { "🔒" } else { "🔓" };
            let lock_font_size = scale_for_dpi(14, dpi) as f32;
            let lock_spec = TextFormatSpec::new(
                "Segoe UI Emoji",
                lock_font_size,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                DWRITE_WORD_WRAPPING_WRAP,
            );
            let lock_format = super::render_devices::text_format(&lock_spec);

            if lock_format.is_some() {
                let lock_text: Vec<u16> = lock_icon.encode_utf16().collect();
                let lock_layout = super::render_devices::text_layout(
                    &lock_text,
                    &lock_spec,
                    btn_size as f32,
                    btn_size as f32,
                    DWRITE_TEXT_ALIGNMENT_CENTER,
                    DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                    false,
                );

                if let Some(ref ll) = lock_layout {
                    // 锁定按钮 hover 背景高亮
                    if data.lock_hovered {
                        let hover_brush = cached_brush(&target, 255, 255, 255, 40);
                        if let Ok(ref hb) = hover_brush {
                            let pad = scale_for_dpi(3, dpi) as f32;
                            let hover_rect = D2D1_ROUNDED_RECT {
                                rect: d2d_rect(
                                    (btn_x - pad as i32) as f32,
                                    (btn_y - pad as i32) as f32,
                                    (btn_x + btn_size + pad as i32) as f32,
                                    (btn_y + btn_size + pad as i32) as f32,
                                ),
                                radiusX: 4.0,
                                radiusY: 4.0,
                            };
                            unsafe { target.FillRoundedRectangle(&hover_rect, hb) };
                        }
                    }

                    // 锁定按钮阴影
                    let shadow_offset = scale_for_dpi(1, dpi) as f32;
                    if let Ok(ref sb) = shadow_brush {
                        for &(sdx, sdy) in &[
                            (shadow_offset, 0.0),
                            (-shadow_offset, 0.0),
                            (0.0, shadow_offset),
                            (0.0, -shadow_offset),
                        ] {
                            let origin = Vector2 {
                                X: btn_x as f32 + sdx,
                                Y: btn_y as f32 + sdy,
                            };
                            unsafe {
                                target.DrawTextLayout(origin, ll, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                            }
                        }
                    }
                    // 锁定按钮主文字
                    if let Ok(ref wb) = white_brush {
                        let origin = Vector2 {
                            X: btn_x as f32,
                            Y: btn_y as f32,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, ll, wb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                        }
                    }
                }
            }
        }

        // ===== 绘制"更多"按钮（⋮，锁定按钮右侧）=====
        let (mbtn_x, mbtn_y, mbtn_size) = super::grid_layout::more_btn_rect(w, title_h, dpi);
        let more_icon = "\u{22EE}"; // ⋮ 三个竖点
        let more_font_size = scale_for_dpi(16, dpi) as f32;
        let more_spec = TextFormatSpec::new(
            "Segoe UI Symbol",
            more_font_size,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
            DWRITE_WORD_WRAPPING_WRAP,
        );
        let more_format = super::render_devices::text_format(&more_spec);

        if more_format.is_some() {
            let more_text: Vec<u16> = more_icon.encode_utf16().collect();
            let more_layout = super::render_devices::text_layout(
                &more_text,
                &more_spec,
                mbtn_size as f32,
                mbtn_size as f32,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                false,
            );

            if let Some(ref ml) = more_layout {
                // hover 背景高亮
                if data.more_hovered || data.menu.open {
                    let hover_brush = cached_brush(&target, 255, 255, 255, 40);
                    if let Ok(ref hb) = hover_brush {
                        let pad = scale_for_dpi(3, dpi) as f32;
                        let hover_rect = D2D1_ROUNDED_RECT {
                            rect: d2d_rect(
                                (mbtn_x - pad as i32) as f32,
                                (mbtn_y - pad as i32) as f32,
                                (mbtn_x + mbtn_size + pad as i32) as f32,
                                (mbtn_y + mbtn_size + pad as i32) as f32,
                            ),
                            radiusX: 4.0,
                            radiusY: 4.0,
                        };
                        unsafe { target.FillRoundedRectangle(&hover_rect, hb) };
                    }
                }

                // 阴影
                let shadow_offset = scale_for_dpi(1, dpi) as f32;
                if let Ok(ref sb) = shadow_brush {
                    for &(sdx, sdy) in &[
                        (shadow_offset, 0.0),
                        (-shadow_offset, 0.0),
                        (0.0, shadow_offset),
                        (0.0, -shadow_offset),
                    ] {
                        let origin = Vector2 {
                            X: mbtn_x as f32 + sdx,
                            Y: mbtn_y as f32 + sdy,
                        };
                        unsafe {
                            target.DrawTextLayout(origin, ml, sb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                        }
                    }
                }
                // 主文字
                if let Ok(ref wb) = white_brush {
                    let origin = Vector2 {
                        X: mbtn_x as f32,
                        Y: mbtn_y as f32,
                    };
                    unsafe {
                        target.DrawTextLayout(origin, ml, wb, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
        }

        // ===== 绘制图标网格（独立方法，参数化设计）=====
        // 通过 view_engine 获取视图参数，layout_calc 计算网格布局
        let mut view_params = get_view_params(&data.view_mode, dpi);
        // 纯图标模式：隐藏图标下方的文本标签（仅显示图标）
        if box_style.icon_only_mode {
            view_params.show_label = false;
        }
        let grid_layout = calculate_grid(w, title_h, &view_params, dpi);

        // 裁剪区域：标题栏下方到窗口底部，防止滚动时图标溢出到标题区域
        let clip_rect = d2d_rect(0.0, title_h as f32, w as f32, h as f32);
        unsafe { target.PushAxisAlignedClip(&clip_rect, D2D1_ANTIALIAS_MODE_ALIASED) };

        // 构建多选参数
        let multi_select_ct = Some(super::multi_select::ContainerType::GridBox);
        let multi_select_id = super::multi_select::container_id_from_uuid(data.config_id);
        let marquee = if data.marquee_active {
            Some((
                data.marquee_start_x,
                data.marquee_start_y,
                data.marquee_current_x,
                data.marquee_current_y,
            ))
        } else {
            None
        };

        render_icon_grid(
            &target,
            &dw_factory,
            &wic_factory,
            &data.items,
            &grid_layout,
            &view_params,
            data.selected_item,
            data.hovered_item,
            dpi,
            data.scroll_offset,
            h,
            data.drag_over_item,
            data.drag_over_insert_col,
            data.drag_over_insert_row,
            data.renaming_item_index,
            &mut data.icon_bitmap_cache,
            multi_select_ct,
            multi_select_id,
            marquee,
        );

        unsafe { target.PopAxisAlignedClip() };

        // ===== 绘制边缘拖拽指示条（未锁定时鼠标悬停边缘显示）=====
        // 边缘条避开圆角区域，避免直角突出于圆角背景。
        if !data.locked && data.hover_edge > 0 {
            let edge_brush = cached_brush(&target, 120, 180, 255, 100);
            if let Ok(ref eb) = edge_brush {
                let border_w = scale_for_dpi(3, dpi) as f32;
                let corner_r = fallback_radius;
                let (l, t, r, b) = compute_edge_rect(data.hover_edge, ww, hh, border_w, corner_r);
                unsafe { target.FillRectangle(&d2d_rect(l, t, r, b), eb) };
            }
        }

        unsafe { target.SetTransform(&identity) };
        let mut ct_tag1: u64 = 0;
        let mut ct_tag2: u64 = 0;
        if let Err(e) = unsafe { target.EndDraw(Some(&mut ct_tag1), Some(&mut ct_tag2)) } {
            log::error!("内容层 D2D EndDraw 失败: {:?}", e);
        }

        // 把内容层拷成 D2D 位图：之后每帧合成只需一次 DrawBitmap，
        // 而不是重跑二十几次字形光栅化。
        let content_size = D2D_SIZE_U {
            width: content_w as u32,
            height: content_h as u32,
        };
        let content_pitch = (content_w as u32) * std::mem::size_of::<Pixel32>() as u32;
        let content_props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let content_src = data.content_bits as *const c_void;
        // 尺寸没变就原地更新，别每帧重建一张整窗位图
        let reused = match data.content_bitmap.as_ref() {
            Some(bitmap) => {
                let dst = windows::Win32::Graphics::Direct2D::Common::D2D_RECT_U {
                    left: 0,
                    top: 0,
                    right: content_size.width,
                    bottom: content_size.height,
                };
                // dstRect 必须显式传：传 None 时从 DC 渲染目标派生的位图
                // 会返回 Ok 但实际不写入（玻璃位图那边踩过这个坑）
                unsafe { bitmap.CopyFromMemory(Some(&dst), content_src, content_pitch) }.is_ok()
            }
            None => false,
        };
        if !reused {
            data.content_bitmap = unsafe {
                target.CreateBitmap(
                    content_size,
                    Some(content_src),
                    content_pitch,
                    &content_props,
                )
            }
            .ok();
        }
        data.content_fingerprint = fingerprint;
        data.content_valid = data.content_bitmap.is_some();
    }
    let content_us = content_started.elapsed().as_micros() as u64;

    // ===== DirectComposition 呈现：玻璃与内容全程留在 GPU =====
    //
    // 与下面的 `UpdateLayeredWindow` 路径相比，**画什么完全一样**
    // （同一个玻璃着色器、同一份内容层 DIB、同一套预乘 source-over），
    // 唯一区别是画面怎么送到屏幕：
    //
    //   ULW   : GPU 渲染 → `Map` 阻塞回读 → memcpy 进 GDI DIB → ULW 交给 DWM
    //   DComp : GPU 渲染 → 输出纹理零拷贝包成 D2D 位图 → 后缓冲上合成 → Present
    //
    // 实测 ULW 那条链里「等 GPU 4.90 + 搬运 0.57 + 光栅合成 3.32 + 提交 0.68」
    // 共 9.5ms，而真正的模糊 + 着色只有 0.44ms —— 97% 的时间在搬像素。
    // DComp 把回读、GDI 表面搬运、ULW 三段全部消掉。
    if dcomp_active {
        // `Box<DcompState>` 先取出来，避免与 `&mut data` 其他字段的借用打架
        let mut slot = data.dcomp.take();
        let presented = {
            let state = slot.as_deref_mut().expect("dcomp_active 已确认为 Some");
            render_via_dcomp(
                state,
                data,
                outer_w,
                outer_h,
                w,
                h,
                wr.left,
                wr.top,
                shadow_padding,
                content_margin,
                content_w,
                content_h,
                corner_radius,
                dpi_scale,
                need_content,
                &glass_config,
                &box_style,
            )
        };
        data.dcomp = slot;

        // 呈现失败时**不退回 ULW**：这个窗口没有 WS_EX_LAYERED，
        // UpdateLayeredWindow 必然失败。DComp 会保留上一次 Present 的画面，
        // 所以跳过一帧表现为画面静止而不是闪黑 —— 比硬凑一张错的画面安全。
        let dib_us = (dib_started.elapsed().as_micros() as u64)
            .saturating_sub(super::mem_probe::take_last_glass_us());
        if presented {
            super::mem_probe::add_frame_timing_ex(dib_us, 0, content_us, !need_content);
            super::grid_window::note_grid_frame_rendered(data);
        } else {
            log::debug!("[DComp] 本帧未提交，保留上一次画面");
        }
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
        }
        RENDERING.store(false, Ordering::SeqCst);
        return;
    }

    // ===== 第二趟：玻璃直写主 DIB，D2D 只负责叠内容层 =====
    //
    // 玻璃像素此刻已完整躺在 `data.glass_pixels`：预乘 BGRA、自上而下、
    // 行距 = 表面宽 × 4，与主 DIB 的布局**逐字节一致**（32bpp DIB 每行
    // 天然 DWORD 对齐，没有行尾填充），尺寸也都等于
    // (w + 2·padding) × (h + 2·padding)。所以一次顺序 memcpy 就是完整背景。
    //
    // 这替代了原来每框每帧的三次全窗内存往返：
    //   ① `Clear` 整窗
    //   ② `CopyFromMemory` 把玻璃上传成 D2D 位图
    //   ③ `DrawBitmap` 全窗软件混合
    // 顺带让常驻的整窗玻璃 D2D 位图彻底消失（5 框合计 24MB）。
    //
    // 能这么做的前提是 D2D1 的 DC 渲染目标在 `BeginDraw` 时**保留** DC
    // 既有像素、同一 DC 上重复 `BindDC` 每帧都重新读入、且半透明像素
    // 字节级原样往返。这三条不是推测，由本模块两条单测钉死：
    // `dc_render_target_preserves_existing_dib_pixels`、
    // `dc_render_target_rereads_dib_on_repeated_binddc`。
    let glass_drawn = if glass_config.enabled {
        render_glass_into_dib(
            data,
            wr.left,
            wr.top,
            w,
            h,
            shadow_padding,
            corner_radius,
            dpi_scale,
            &glass_config,
        )
    } else {
        false
    };

    // ===== 快路径：玻璃已直写 DIB，内容合成完全不进 D2D =====
    //
    // `ID2D1DCRenderTarget` 绑定 GDI DC 后，`BeginDraw` 必须先把**整张**
    // GDI 表面读进 D2D 内部缓冲（否则无法保留既有像素 —— 这正是
    // `dc_render_target_preserves_existing_dib_pixels` 证明的行为），
    // `EndDraw` 再整张写回：每帧两次全窗搬运，实测约 6.8ms，占单帧
    // 三分之一，而真正要画的只是内容层里非透明的那一小部分。
    //
    // 玻璃既然已经 memcpy 进 DIB，剩下的就只是「内容矩形内的预乘
    // source-over」这件纯算术的事，逐像素做反而快一个数量级：
    // alpha=0 的像素（内容层的绝大多数）直接跳过，alpha=255 直接覆盖。
    //
    // 内容层缺失时（首帧就失败等极端情况）退回 D2D 慢路径：
    // 宁可慢，也绝不能把没有图标标题的画面提交上去。
    let cpu_composited = glass_drawn && !data.content_bits.is_null() && data.content_valid;
    if cpu_composited {
        let origin = shadow_padding - content_margin;
        unsafe {
            composite_premultiplied_over(
                data.dib_bits,
                outer_w,
                outer_h,
                data.content_bits,
                content_w,
                content_h,
                origin,
                origin,
            );
        }
    }

    // ===== 慢路径：玻璃不可用或内容层缺失，仍用 D2D 画回退背景 + 内容 =====
    if !cpu_composited {
        if unsafe { target.BindDC(GdiHDC(mem_dc), &bind_rect) }.is_err() {
            log::error!("BindDC 失败");
            release_grid_dib(data);
            unsafe {
                ReleaseDC(std::ptr::null_mut(), screen_dc);
            }
            RENDERING.store(false, Ordering::SeqCst);
            return;
        }
        unsafe { target.BeginDraw() };
        unsafe { target.SetTransform(&identity) };

        // 玻璃直写成功时，DIB 每个像素都已经是本帧内容，Clear 是纯浪费；
        // 失败或玻璃关闭时 DIB 里还是上一帧残留，必须清干净再画回退背景。
        if !glass_drawn {
            unsafe { target.Clear(Some(&transparent)) };
            let [red, green, blue] = box_style.background_color;
            let bg_brush = cached_brush(&target, red, green, blue, box_style.background_alpha);
            if let Ok(ref brush) = bg_brush {
                let rounded_rect = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(
                        shadow_padding as f32,
                        shadow_padding as f32,
                        shadow_padding as f32 + ww,
                        shadow_padding as f32 + hh,
                    ),
                    radiusX: fallback_radius,
                    radiusY: fallback_radius,
                };
                unsafe { target.FillRoundedRectangle(&rounded_rect, brush) };
            }
        }

        // 后续标题、图标与交互反馈继续使用原有内容坐标系；仅在提交位图时
        // 平移到透明外投影留白之内。

        // 内容层盖在玻璃之上。
        //
        // 用 D2D 的 source-over 合成而不是 GDI AlphaBlend：改造前是
        // 「同一次 BeginDraw 里先画玻璃、再画内容」，premultiplied source-over
        // 满足结合律，(玻璃 ⊕ A) ⊕ B == 玻璃 ⊕ (A ⊕ B)，因此分层合成与
        // 一次性绘制在数学上等价，只有 8 位取整的舍入路径不同（±1 LSB）。
        //
        // destinationRectangle 必须显式传，不能图省事传 None：
        // None 时 D2D 用「位图自己的 DIP 尺寸」当目标矩形，而内容层位图
        // 是按 96 DPI 建的，DIP 尺寸 == 像素尺寸只在渲染目标也是 96 DPI
        // 时才成立。玻璃那一趟一直是显式传 `surface_w × surface_h`，
        // 内容层用同一套写法（outer_w == surface_w、outer_h == surface_h）
        // 就与玻璃严格同构，DPI 语义上不存在第二种解释。
        if let Some(bitmap) = data.content_bitmap.clone() {
            // 内容层位图带 content_margin 外扩，合成时要把它落回
            // 「留白内缩 margin」的位置，内容本体才恰好压在内容矩形上。
            let origin = (shadow_padding - content_margin) as f32;
            let content_dst = d2d_rect(
                origin,
                origin,
                origin + content_w as f32,
                origin + content_h as f32,
            );
            unsafe {
                target.DrawBitmap(
                    &bitmap,
                    Some(&content_dst),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                )
            };
        }

        // ===== D2D 渲染结束 =====
        unsafe { target.SetTransform(&identity) };
        let mut tag1: u64 = 0;
        let mut tag2: u64 = 0;
        if let Err(e) = unsafe { target.EndDraw(Some(&mut tag1), Some(&mut tag2)) } {
            log::error!("D2D EndDraw 失败: {:?}", e);
        }
    }

    // `dib_started` 包住了第二趟整段，而玻璃那一段（`render_background`）
    // 是在第二趟**内部**跑的、已经单独记过账。这里必须扣掉，
    // 否则「玻璃 + 光栅合成」会把玻璃算两遍（实测虚高 52%）。
    let dib_us = (dib_started.elapsed().as_micros() as u64)
        .saturating_sub(super::mem_probe::take_last_glass_us());

    // ===== 调用 UpdateLayeredWindow 提交渲染结果 =====
    let submit_started = std::time::Instant::now();
    let size = POINT {
        x: outer_w,
        y: outer_h,
    };
    let src_pt = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        blend_op: AC_SRC_OVER,
        blend_flags: 0,
        source_constant_alpha: 255,
        alpha_format: AC_SRC_ALPHA,
    };

    if super::mem_probe::dump_frame_enabled() && !data.dib_bits.is_null() {
        super::grid_window::dump_dib_frame(data.dib_bits as *const Pixel32, outer_w, outer_h, hwnd);
    }

    let submit_result = unsafe {
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
        )
    };
    if submit_result == 0 {
        let error_code = unsafe { GetLastError() };
        if !LAYERED_SUBMIT_FAILED.swap(true, Ordering::Relaxed) {
            log::error!(
                "UpdateLayeredWindow 提交分类框画面失败: hwnd={:?}, size={}x{}, error={}",
                hwnd,
                outer_w,
                outer_h,
                error_code
            );
        }
    } else if LAYERED_SUBMIT_FAILED.swap(false, Ordering::Relaxed) {
        log::info!("UpdateLayeredWindow 提交分类框画面已恢复: hwnd={:?}", hwnd);
    }

    super::mem_probe::add_frame_timing_ex(
        dib_us,
        submit_started.elapsed().as_micros() as u64,
        content_us,
        !need_content,
    );

    // DIB 与内存 DC 保留给下一帧复用，仅在尺寸变化或窗口销毁时释放
    unsafe {
        ReleaseDC(std::ptr::null_mut(), screen_dc);
    }

    // 记一笔「已渲染」，下一个 200ms tick 据此判断能不能跳帧。
    // 必须放在渲染完成之后：本帧自己的那次 capture_output 会推进
    // 桌面代号，提前记会漏掉这一步，下一个 tick 就得白扫一遍队列。
    super::grid_window::note_grid_frame_rendered(data);

    RENDERING.store(false, Ordering::SeqCst);
}
/// 把液态玻璃表面**直接写进分类框主 DIB**，不经过 D2D。
///
/// # 为什么可以绕过 D2D
///
/// `render_background` 返回后，玻璃像素已完整躺在 `data.glass_pixels`：
/// 预乘 BGRA、自上而下、行距 = 表面宽 × 4。主 DIB 是 32bpp 自上而下
/// DIB Section，每行天然 DWORD 对齐、行距同样是宽 × 4，两者尺寸都等于
/// `(w + 2·padding) × (h + 2·padding)`。布局逐字节一致，因此一次
/// `copy_nonoverlapping` 就是完整背景。写完后 D2D 的 `BeginDraw` 会把
/// 这些像素当作既有内容保留，内容层直接叠在上面。
///
/// # 相比旧实现省掉了什么
///
/// 旧路径是「回读 → `CopyFromMemory` 上传成整窗 D2D 位图 → `Clear` 整窗
/// → `DrawBitmap` 全窗软件混合」，每框每帧三次全窗内存往返，外加一张
/// 常驻整窗位图。现在只剩一次顺序 memcpy，那张位图整个消失。
///
/// # 返回 false 的含义
///
/// 这一帧拿不到玻璃（捕获失败 / 尺寸对不上）。调用方**必须**先 `Clear`
/// 再画回退背景 —— 此时 DIB 里是上一帧残留，不清就会留下重影。
#[allow(clippy::too_many_arguments)]
fn render_glass_into_dib(
    data: &mut GridWindowData,
    screen_x: i32,
    screen_y: i32,
    width: i32,
    height: i32,
    padding: i32,
    corner_radius: f32,
    dpi_scale: f32,
    glass_config: &crate::config::LiquidGlassConfig,
) -> bool {
    if !super::liquid_glass::render_background(
        screen_x,
        screen_y,
        width,
        height,
        padding,
        corner_radius,
        dpi_scale,
        glass_config,
        &mut data.glass_pixels,
        // UpdateLayeredWindow 路径必须拿到 CPU 像素：画面最终要落进 GDI DIB。
        true,
        data.grid_id,
        // 拖拽期不再同步读当前帧：阻塞 Map 会等整个 GPU 队列排空
        // （实测单帧 100-400ms 卡顿）。WM_MOVING 已把渲染节奏拉到 8ms，
        // 读上一帧的背景滞后仅 8ms，视觉不可感知。
        false,
    ) {
        return false;
    }

    let surface_w = width.saturating_add(padding.saturating_mul(2));
    let surface_h = height.saturating_add(padding.saturating_mul(2));
    if surface_w <= 0 || surface_h <= 0 {
        return false;
    }
    let surface_pixels = surface_w as usize * surface_h as usize;
    // 回读缓冲必须刚好是一张完整表面：直写没有 D2D 的裁剪保护，
    // 长度对不上就是越界写主 DIB，属于内存破坏而不是画面错误。
    if data.glass_pixels.len() != surface_pixels {
        log::warn!(
            "液态玻璃回读尺寸不匹配: {} != {surface_pixels}",
            data.glass_pixels.len()
        );
        return false;
    }
    // 主 DIB 由 `ensure_grid_dib` 按 outer_w × outer_h 建好，而
    // outer_w == surface_w、outer_h == surface_h。这里再校一次是纵深防御：
    // 留白切换的那一帧窗口外框可能还没收缩完。
    if data.dib_bits.is_null() || data.dib_w != surface_w || data.dib_h != surface_h {
        log::warn!(
            "主 DIB({}x{}) 与玻璃表面({surface_w}x{surface_h}) 尺寸不符，本帧回退半透明背景",
            data.dib_w,
            data.dib_h
        );
        return false;
    }

    let dst = data.dib_bits;
    let src = data.glass_pixels.as_ptr();
    // SAFETY: 上面已校验两边长度都恰好是 surface_pixels 个 Pixel32，
    // 且分属 DIB 段与 Vec 两块互不重叠的内存。
    unsafe { std::ptr::copy_nonoverlapping(src, dst, surface_pixels) };
    true
}

/// 预乘 source-over 的单通道计算：`src + round(dst × inv / 255)`。
///
/// `inv` 是 `255 - srcAlpha`。除以 255 用「先加 128 再 folded-shift」的
/// 恒等式代替整数除法：
///
/// ```text
/// t = dst × inv;  u = t + 128;  result = (u + (u >> 8)) >> 8
/// ```
///
/// 对 `0..=255 × 0..=255` 的全部 65536 种输入都与四舍五入的精确除法
/// **逐位相同**（`over_channel_matches_exact_rounding_division` 穷举验证）。
///
/// # 一个踩过就写进测试的坑
///
/// 直觉写法 `(t + 128 + (t >> 8)) >> 8` 是**错的**：移位取的是 `t`
/// 而不是 `t + 128` 的高位，进位少算一次。穷举测试当场抓到
/// `dst=152, inv=229` 时得 136 而正确答案是 137。差值只有 1/255，
/// 肉眼绝对看不出来，日志也干净 —— 正是那种只能靠穷举才发现的错误。
#[inline]
fn over_channel(src: u8, dst: u8, inv: u32) -> u8 {
    let t = dst as u32 * inv;
    let u = t + 128;
    let scaled = (u + (u >> 8)) >> 8;
    (src as u32 + scaled).min(255) as u8
}

/// 把内容层（预乘 BGRA）以 source-over 叠到已写好玻璃的主 DIB 上。
///
/// # 为什么这一步不用 D2D
///
/// `ID2D1DCRenderTarget` 绑定 GDI DC 之后，`BeginDraw` 必须先把**整张**
/// GDI 表面读进 D2D 内部缓冲（否则无法保留既有像素 —— 这正是
/// `dc_render_target_preserves_existing_dib_pixels` 证明的行为），
/// `EndDraw` 再整张写回。每帧两次全窗搬运，实测约 6.8ms，
/// 占单帧三分之一，而真正要画的只是内容层里非透明的那一小部分。
///
/// 玻璃已经用 memcpy 直写进 DIB 了，合成只剩「内容矩形内的预乘
/// source-over」这一件纯算术的事，逐像素做反而快得多：alpha=0 的像素
/// （内容层的绝大多数）直接跳过，alpha=255 的直接覆盖。
///
/// # 越界保护
///
/// 内容层带 `content_margin` 外扩，贴边或留白为 0（玻璃关闭）时
/// 原点可能是负数、右下也可能超出主 DIB，因此先夹到目标范围内。
/// 夹错方向会写坏相邻堆内存，且不会有任何报错。
#[allow(clippy::too_many_arguments)]
unsafe fn composite_premultiplied_over(
    dst: *mut Pixel32,
    dst_w: i32,
    dst_h: i32,
    src: *const Pixel32,
    src_w: i32,
    src_h: i32,
    origin_x: i32,
    origin_y: i32,
) {
    if dst.is_null() || src.is_null() {
        return;
    }
    if dst_w <= 0 || dst_h <= 0 || src_w <= 0 || src_h <= 0 {
        return;
    }
    let x0 = origin_x.max(0);
    let y0 = origin_y.max(0);
    let x1 = origin_x.saturating_add(src_w).min(dst_w);
    let y1 = origin_y.saturating_add(src_h).min(dst_h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        let srow = src.add((y - origin_y) as usize * src_w as usize);
        let drow = dst.add(y as usize * dst_w as usize);
        for x in x0..x1 {
            let s = *srow.add((x - origin_x) as usize);
            let sa = s.a as u32;
            // 全透明：目标像素原样保留。内容层里这是绝大多数像素。
            if sa == 0 {
                continue;
            }
            let slot = drow.add(x as usize);
            // 完全不透明：直接覆盖，省掉三次乘法
            if sa == 255 {
                *slot = s;
                continue;
            }
            let d = *slot;
            let inv = 255 - sa;
            *slot = Pixel32 {
                b: over_channel(s.b, d.b, inv),
                g: over_channel(s.g, d.g, inv),
                r: over_channel(s.r, d.r, inv),
                a: over_channel(s.a, d.a, inv),
            };
        }
    }
}

/// DirectComposition 呈现路径：玻璃与内容全程留在 GPU，不回读、不走 GDI。
///
/// # 与 `UpdateLayeredWindow` 路径的关系
///
/// **画的内容完全一样**：同一个玻璃着色器（`PSMain`）、同一份内容层
/// DIB（仍由 `ID2D1DCRenderTarget` 以 GDI 兼容方式光栅化，所以文字
/// hinting 与改造前逐字节一致）、同一套预乘 source-over 合成顺序。
/// 变的只有「像素怎么到屏幕」这一段。
///
/// # 省掉了什么
///
/// 实测 `UpdateLayeredWindow` 那条链每框每帧要付：
///
/// | 段 | 耗时 | DComp 下 |
/// |---|---|---|
/// | `Map` 阻塞等 GPU 排空 | 4.90ms | **消失**（不回读） |
/// | 逐行 memcpy 出映射内存 | 0.57ms | **消失** |
/// | 光栅合成（GDI 表面搬进搬出） | 3.32ms | **消失**（GPU 合成） |
/// | `UpdateLayeredWindow` | 0.68ms | `Present` ≈0.2ms |
/// | 模糊 + 着色（真正的渲染） | 0.44ms | 不变 |
///
/// 关键前提是玻璃输出纹理与合成端现在同属**一个** D3D11/D2D1 设备
/// （见 `render_devices::SharedGpu`），所以能用
/// `glass_output_bitmap()` 零拷贝包成 D2D 位图直接采样。
///
/// # 返回 false 的含义
///
/// 这一帧没能提交。调用方**不会**退回 `UpdateLayeredWindow`（窗口没有
/// `WS_EX_LAYERED`，ULW 必然失败），而是直接跳过：DComp 会保留上一次
/// `Present` 的画面，所以表现为画面静止一帧而不是闪黑。
#[allow(clippy::too_many_arguments)]
fn render_via_dcomp(
    state: &mut DcompState,
    data: &mut GridWindowData,
    outer_w: i32,
    outer_h: i32,
    w: i32,
    h: i32,
    screen_x: i32,
    screen_y: i32,
    shadow_padding: i32,
    content_margin: i32,
    content_w: i32,
    content_h: i32,
    corner_radius: f32,
    dpi_scale: f32,
    content_redrawn: bool,
    glass_config: &crate::config::LiquidGlassConfig,
    box_style: &crate::config::BoxStyleConfig,
) -> bool {
    if outer_w <= 0 || outer_h <= 0 {
        return false;
    }

    // DComp 路径永远不回读，那块每框约 4.8MB 的回读缓冲应当归还系统。
    // 只在还有分配时做一次，之后每帧都是空操作。
    if data.glass_pixels.capacity() > 0 {
        data.glass_pixels = Vec::new();
    }

    // 1) 交换链尺寸跟上窗口外框（DPI 变化 / 用户缩放都会走到这里）
    if !super::grid_window::resize_dcomp(state, outer_w as u32, outer_h as u32) {
        return false;
    }
    let Some(backbuffer) = super::grid_window::ensure_dcomp_backbuffer(state) else {
        return false;
    };

    // 2) 玻璃：渲染进共享输出纹理，need_pixels=false → 整段回读跳过
    let mut glass_ok = false;
    if glass_config.enabled {
        glass_ok = super::liquid_glass::render_background(
            screen_x,
            screen_y,
            w,
            h,
            shadow_padding,
            corner_radius,
            dpi_scale,
            glass_config,
            &mut data.glass_pixels,
            false,
            data.grid_id,
            false,
        );
        // 玻璃开着却拿不到画面：这一帧不提交，DComp 保留上一次画面。
        // 比画一张半透明回退背景更稳 —— 后者会在 DXGI 偶发抖动时闪一下。
        if !glass_ok {
            return false;
        }
    }

    // 3) 内容层上传到**共享设备**。只在内容真的重画过、或尺寸变了时做，
    //    稳态下每帧零上传（内容层命中率实测 100%）。
    let need_upload = content_redrawn
        || state.content.is_none()
        || state.content_w != content_w as u32
        || state.content_h != content_h as u32;
    if need_upload {
        if data.content_bits.is_null() || content_w <= 0 || content_h <= 0 {
            log::warn!("[DComp] 内容层尚未就绪，本帧跳过");
            return false;
        }
        // ID2D1DeviceContext::CreateBitmap 收 PROPERTIES1（不是 v1 的 PROPERTIES）
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        let uploaded = unsafe {
            state.context.CreateBitmap(
                D2D_SIZE_U {
                    width: content_w as u32,
                    height: content_h as u32,
                },
                Some(data.content_bits as *const c_void),
                (content_w as u32) * std::mem::size_of::<Pixel32>() as u32,
                &props,
            )
        };
        match uploaded {
            Ok(bitmap) => {
                state.content = Some(bitmap);
                state.content_w = content_w as u32;
                state.content_h = content_h as u32;
            }
            Err(error) => {
                log::error!("[DComp] 内容层上传到共享设备失败: {error:?}");
                return false;
            }
        }
    }

    // 4) 在后缓冲上合成
    let identity = Matrix3x2 {
        M11: 1.0,
        M22: 1.0,
        ..Default::default()
    };
    let transparent = D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    unsafe {
        state.context.SetTarget(&backbuffer);
        state.context.BeginDraw();
        state.context.SetTransform(&identity);
        // 后缓冲保留着上一帧，必须清成全透明再画，否则会叠影
        state.context.Clear(Some(&transparent));

        if glass_ok {
            // 零拷贝：直接采样玻璃着色器的输出纹理。
            // 输出纹理是所有分类框**共享**的一张（尺寸取历史最大），
            // 所以必须用 source rect 只取左上角本框那一块，
            // 否则会画到别的框留下的残留像素。
            if let Some(glass) = super::liquid_glass::glass_output_bitmap() {
                let dst = d2d_rect(0.0, 0.0, outer_w as f32, outer_h as f32);
                let src = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: outer_w as f32,
                    bottom: outer_h as f32,
                };
                state.context.DrawBitmap(
                    &glass,
                    Some(&dst),
                    1.0,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    Some(&src),
                    // 透视变换：不用，传 None
                    None,
                );
            } else {
                log::warn!("[DComp] 拿不到玻璃零拷贝位图，本帧跳过");
                state.context.SetTarget(None::<&ID2D1Image>);
                return false;
            }
        } else {
            // 玻璃关闭：画原来那张半透明圆角背景
            let [red, green, blue] = box_style.background_color;
            let color = d2d_color(red, green, blue, box_style.background_alpha);
            if let Ok(brush) = state.context.CreateSolidColorBrush(&color, None) {
                let radius = box_style.border_radius * dpi_scale;
                let rounded = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(
                        shadow_padding as f32,
                        shadow_padding as f32,
                        shadow_padding as f32 + w as f32,
                        shadow_padding as f32 + h as f32,
                    ),
                    radiusX: radius,
                    radiusY: radius,
                };
                state.context.FillRoundedRectangle(&rounded, &brush);
            }
        }

        // 内容层盖在玻璃之上。落点与 CPU 快路径完全一致：
        // 内容层带 content_margin 外扩，所以要放在 (padding - margin) 处，
        // 内容本体才恰好压在内容矩形上。
        if let Some(content) = state.content.clone() {
            let origin = (shadow_padding - content_margin) as f32;
            let dst = d2d_rect(
                origin,
                origin,
                origin + content_w as f32,
                origin + content_h as f32,
            );
            state.context.DrawBitmap(
                &content,
                Some(&dst),
                1.0,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );
        }

        state.context.SetTransform(&identity);
        let mut tag1: u64 = 0;
        let mut tag2: u64 = 0;
        if let Err(error) = state.context.EndDraw(Some(&mut tag1), Some(&mut tag2)) {
            log::error!("[DComp] EndDraw 失败: {error:?}");
            state.context.SetTarget(None::<&ID2D1Image>);
            return false;
        }
        // 必须解绑：后缓冲在 ResizeBuffers 时会失效，
        // 一直被 SetTarget 持有着会让 ResizeBuffers 返回 INVALID_CALL。
        state.context.SetTarget(None::<&ID2D1Image>);
    }

    // 5) 提交。SyncInterval=0：合成交换链由 DWM 自己按刷新率取用，
    //    这里不该阻塞等垂直同步 —— 5 个框各自 Present 时若都等 vsync，
    //    会被串行化到合计 60Hz，反而把每框帧率压到 12fps。
    if super::mem_probe::dump_frame_enabled() {
        super::grid_window::dump_dcomp_backbuffer(state);
    }

    unsafe {
        // 注意：windows 0.62 里 `Present` 返回裸 `HRESULT` 而不是 `Result`，
        // 不能用 `if let Err(..)`，得自己判 `is_err()`。
        let hr = state.swapchain.Present(0, DXGI_PRESENT(0));
        if hr.is_err() {
            log::error!("[DComp] Present 失败: {hr:?}");
            return false;
        }
    }
    true
}

/// 渲染图标网格（独立方法，参数化设计）
///
/// 通过 `GridLayout` 确定位置（cols/cell_w/cell_h/grid_left/grid_top），
/// 通过 `ViewParams` 控制视图模式（icon_size/show_label 等）。
///
/// # 支持的视图模式
///
/// | 模式 | icon_size | 说明 |
/// |------|-----------|------|
/// | ExtraLarge | 96px | 超大图标 |
/// | Large | 64px | 大图标 |
/// | Medium | 48px | 中等图标（默认） |
/// | Small | 32px | 小图标 |
///
/// # 自动换行逻辑
///
/// `GridLayout` 已处理窄窗口边界：当窗口宽度不足以容纳一个完整 cell 时，
/// `cell_w` 被 clamp 到可用宽度，`cols` 强制为 1，保证图标始终可见。
///
/// # 参数
///
/// - `target`：D2D render target（已 BindDC）
/// - `dw_factory`：DirectWrite 工厂（用于创建文字格式）
/// - `wic_factory`：WIC 工厂（用于图标位图转换，保留扩展）
/// - `items`：要渲染的文件项列表
/// - `layout`：网格布局（由 `layout_calc::calculate_grid` 计算）
/// - `view_params`：视图参数（由 `view_engine::get_view_params` 获取）
/// - `selected_item`：当前选中项索引（-1 = 无选中）
/// - `hovered_item`：当前 hover 项索引（-1 = 无 hover）
/// - `dpi`：窗口 DPI（用于缩放字体和间距）
pub fn render_icon_grid(
    target: &ID2D1DCRenderTarget,
    dw_factory: &IDWriteFactory,
    _wic_factory: &IWICImagingFactory,
    items: &[GridFileItem],
    layout: &GridLayout,
    view_params: &ViewParams,
    selected_item: i32,
    hovered_item: i32,
    dpi: u32,
    scroll_offset: i32,
    window_height: i32,
    drag_over_item: i32,
    drag_over_insert_col: i32,
    drag_over_insert_row: i32,
    renaming_item_index: i32,
    icon_bitmap_cache: &mut HashMap<(usize, i32), ID2D1Bitmap>,
    multi_select_container_type: Option<super::multi_select::ContainerType>,
    multi_select_container_id: u64,
    marquee: Option<(i32, i32, i32, i32)>,
) {
    let icon_size = view_params.icon_size;
    let cell_w = layout.cell_w;
    let cell_h = layout.cell_h;
    let cols = layout.cols;
    let grid_left = layout.grid_left;
    let grid_top = layout.grid_top;
    let icon_label_h = scale_for_dpi(36, dpi); // 2 行文字高度（支持换行）

    // 选中项高亮画刷
    let highlight_brush = cached_brush(target, 60, 80, 120, 120);
    // Hover 高亮画刷（比选中色浅）
    let hover_brush = cached_brush(target, 70, 90, 130, 70);

    for (i, item) in items.iter().enumerate() {
        let (cell_x, cell_y) = if item.grid_col >= 0 && item.grid_row >= 0 {
            // Per-item grid position (user-placed via drag)
            (
                grid_left + item.grid_col * cell_w,
                grid_top + item.grid_row * cell_h,
            )
        } else {
            // Auto-layout: index-based position
            let col = (i as i32) % cols;
            let row = (i as i32) / cols;
            (grid_left + col * cell_w, grid_top + row * cell_h)
        };
        // Apply scroll offset
        let cell_y = cell_y - scroll_offset;
        let is_selected = i as i32 == selected_item
            || multi_select_container_type.is_some_and(|ct| {
                super::multi_select::is_selected(super::multi_select::SelectionKey {
                    container_type: ct,
                    container_id: multi_select_container_id,
                    item_index: i,
                })
            });
        let is_hovered = i as i32 == hovered_item;

        // Hover 背景（比选中浅，仅非选中项显示）
        if is_hovered && !is_selected {
            if let Ok(ref hb) = hover_brush {
                let hl_rect = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(
                        cell_x as f32,
                        cell_y as f32,
                        (cell_x + cell_w) as f32,
                        (cell_y + cell_h) as f32,
                    ),
                    radiusX: scale_for_dpi(6, dpi) as f32,
                    radiusY: scale_for_dpi(6, dpi) as f32,
                };
                unsafe { target.FillRoundedRectangle(&hl_rect, hb) };
            }
        }

        // 拖拽覆盖替换高亮（与 hover 背景一致，显示在拖拽项覆盖的目标项上）
        if i as i32 == drag_over_item && !is_selected {
            if let Ok(ref hb) = hover_brush {
                let hl_rect = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(
                        cell_x as f32,
                        cell_y as f32,
                        (cell_x + cell_w) as f32,
                        (cell_y + cell_h) as f32,
                    ),
                    radiusX: scale_for_dpi(6, dpi) as f32,
                    radiusY: scale_for_dpi(6, dpi) as f32,
                };
                unsafe { target.FillRoundedRectangle(&hl_rect, hb) };
            }
        }

        // 选中项高亮背景
        if is_selected {
            if let Ok(ref hb) = highlight_brush {
                let hl_rect = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(
                        cell_x as f32,
                        cell_y as f32,
                        (cell_x + cell_w) as f32,
                        (cell_y + cell_h) as f32,
                    ),
                    radiusX: scale_for_dpi(6, dpi) as f32,
                    radiusY: scale_for_dpi(6, dpi) as f32,
                };
                unsafe { target.FillRoundedRectangle(&hl_rect, hb) };
            }
        }

        // 绘制图标 + 标签文字（公共方法，与覆盖区域完全一致）
        let icon_x = cell_x + (cell_w - icon_size) / 2;
        let icon_y = cell_y + 4;
        let label_x = (cell_x + 2) as f32;
        let label_y = (icon_y + icon_size + scale_for_dpi(6, dpi)) as f32;
        let label_w = (cell_w - 4) as f32;
        let label_h = if is_selected {
            scale_for_dpi(36 * 3, dpi) as f32
        } else {
            icon_label_h as f32
        };

        if view_params.show_label && i as i32 != renaming_item_index {
            draw_icon_with_label_cached(
                target,
                dw_factory,
                Some(_wic_factory),
                item.icon_handle,
                &item.display_name,
                icon_x,
                icon_y,
                icon_size,
                label_x,
                label_y,
                label_w,
                label_h,
                dpi,
                !is_selected, // 未选中时应用省略号截断
                Some(&mut *icon_bitmap_cache),
            );
        } else if !item.icon_handle.is_null() {
            // 不显示标签时仅绘制图标
            draw_icon_d2d(
                target,
                Some(_wic_factory),
                item.icon_handle,
                icon_x,
                icon_y,
                icon_size,
                Some(&mut *icon_bitmap_cache),
            );
        }
    }

    // ===== 拖拽插入指示器（垂直分隔线）=====
    if drag_over_insert_col >= 0 && drag_over_insert_row >= 0 {
        let separator_x = (grid_left + drag_over_insert_col * cell_w) as f32;
        let separator_top =
            (grid_top + drag_over_insert_row * cell_h) as f32 - scroll_offset as f32;
        let separator_bottom = separator_top + cell_h as f32;
        let separator_w = scale_for_dpi(1, dpi) as f32;

        let insert_brush = cached_brush(target, 255, 255, 255, 255);
        if let Ok(ref ib) = insert_brush {
            let line_rect = d2d_rect(
                separator_x - separator_w / 2.0,
                separator_top + scale_for_dpi(4, dpi) as f32,
                separator_x + separator_w / 2.0,
                separator_bottom - scale_for_dpi(4, dpi) as f32,
            );
            unsafe { target.FillRectangle(&line_rect, ib) };
        }
    }

    // ===== Draw scrollbar if content overflows =====
    let total_rows = (items.len() as i32 + cols - 1) / cols;
    let total_content_h = total_rows * cell_h;
    let visible_h = window_height - grid_top;
    if total_content_h > visible_h && visible_h > 0 {
        // Calculate scrollbar thumb size and position
        let scrollbar_w = scale_for_dpi(6, dpi) as f32;
        let min_thumb_h = scale_for_dpi(30, dpi) as f32;
        let thumb_h =
            (visible_h as f32 * visible_h as f32 / total_content_h as f32).max(min_thumb_h);
        let max_scroll = total_content_h - visible_h;
        let scroll_ratio = if max_scroll > 0 {
            scroll_offset as f32 / max_scroll as f32
        } else {
            0.0
        };
        let track_h = visible_h as f32 - thumb_h;
        let thumb_y = grid_top as f32 + scroll_ratio * track_h;

        // Draw scrollbar track (subtle background)
        let track_brush = cached_brush(target, 255, 255, 255, 15);
        if let Ok(ref tb) = track_brush {
            let track_rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(
                    (layout.grid_left + cols * cell_w) as f32 + scale_for_dpi(2, dpi) as f32,
                    grid_top as f32,
                    (layout.grid_left + cols * cell_w) as f32
                        + scale_for_dpi(2, dpi) as f32
                        + scrollbar_w,
                    window_height as f32,
                ),
                radiusX: scrollbar_w / 2.0,
                radiusY: scrollbar_w / 2.0,
            };
            unsafe { target.FillRoundedRectangle(&track_rect, tb) };
        }

        // Draw scrollbar thumb
        let thumb_brush = cached_brush(target, 255, 255, 255, 80);
        if let Ok(ref hb) = thumb_brush {
            let thumb_rect = D2D1_ROUNDED_RECT {
                rect: d2d_rect(
                    (layout.grid_left + cols * cell_w) as f32 + scale_for_dpi(2, dpi) as f32,
                    thumb_y,
                    (layout.grid_left + cols * cell_w) as f32
                        + scale_for_dpi(2, dpi) as f32
                        + scrollbar_w,
                    thumb_y + thumb_h,
                ),
                radiusX: scrollbar_w / 2.0,
                radiusY: scrollbar_w / 2.0,
            };
            unsafe { target.FillRoundedRectangle(&thumb_rect, hb) };
        }
    }

    // ===== Marquee selection rectangle =====
    if let Some((mx1, my1, mx2, my2)) = marquee {
        let left = mx1.min(mx2) as f32;
        let top = (my1.min(my2) - scroll_offset) as f32;
        let right = mx1.max(mx2) as f32;
        let bottom = (my1.max(my2) - scroll_offset) as f32;
        // Semi-transparent blue fill
        let fill_brush = cached_brush(target, 0, 120, 215, 40);
        if let Ok(ref fb) = fill_brush {
            unsafe { target.FillRectangle(&d2d_rect(left, top, right, bottom), fb) };
        }
        // Blue border (1px)
        let border_brush = cached_brush(target, 0, 120, 215, 200);
        if let Ok(ref bb) = border_brush {
            let bw = 1.0_f32;
            unsafe {
                target.FillRectangle(&d2d_rect(left, top, right, top + bw), bb);
                target.FillRectangle(&d2d_rect(left, bottom - bw, right, bottom), bb);
                target.FillRectangle(&d2d_rect(left, top, left + bw, bottom), bb);
                target.FillRectangle(&d2d_rect(right - bw, top, right, bottom), bb);
            }
        }
    }
}

/// 使用 Direct2D 绘制图标（HICON → GetDIBits → premultiplied D2D Bitmap → DrawBitmap）
///
/// 绕过 WIC，直接从 HICON 的颜色位图读取原始 BGRA 像素，
/// 手动转换为 premultiplied alpha 后创建 D2D bitmap，确保边缘透明过渡正确。
fn draw_icon_d2d(
    target: &ID2D1DCRenderTarget,
    _wic_factory: Option<&IWICImagingFactory>,
    hicon: HICON,
    x: i32,
    y: i32,
    size: i32,
    icon_bitmap_cache: Option<&mut HashMap<(usize, i32), ID2D1Bitmap>>,
) {
    let cache_key = (hicon as usize, size);
    if let Some(bitmap) = icon_bitmap_cache
        .as_deref()
        .and_then(|cache| cache.get(&cache_key))
    {
        draw_icon_bitmap(target, bitmap, x, y, size);
        return;
    }

    // 1. 从 HICON 提取颜色位图
    let mut icon_info = ICONINFO {
        f_icon: 0,
        x_hotspot: 0,
        y_hotspot: 0,
        hbm_mask: std::ptr::null_mut(),
        hbm_color: std::ptr::null_mut(),
    };
    if unsafe { GetIconInfo(hicon, &mut icon_info) } == 0 {
        return;
    }

    // 必须有颜色位图（32-bit ARGB）；monochrome mask 无法正确渲染
    if icon_info.hbm_color.is_null() {
        if !icon_info.hbm_mask.is_null() {
            unsafe { DeleteObject(icon_info.hbm_mask as HGDIOBJ) };
        }
        return;
    }

    // 2. 用 GetDIBits 读取原始 BGRA 像素
    //    第一次查询：bi_bit_count 必须为 0，让系统返回实际格式
    let mut bi = BITMAPINFOHEADER {
        bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        bi_width: 0,
        bi_height: 0,
        bi_planes: 0,
        bi_bit_count: 0, // 关键：查询时必须为 0
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };

    // 第一次调用获取位图尺寸和格式
    if unsafe {
        GetDIBits(
            screen_dc,
            icon_info.hbm_color,
            0,
            0,
            std::ptr::null_mut(),
            &mut bi as *mut BITMAPINFOHEADER as *mut c_void,
            DIB_RGB_COLORS,
        )
    } == 0
    {
        log::warn!("GetDIBits 查询失败");
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    let bmp_w = bi.bi_width;
    let bmp_h = bi.bi_height.abs(); // 可能是负值（top-down DIB）

    if bmp_w <= 0 || bmp_h <= 0 || bi.bi_bit_count != 32 {
        log::debug!(
            "图标位图格式不支持: {}x{} {}bpp",
            bmp_w,
            bmp_h,
            bi.bi_bit_count
        );
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    // 分配像素缓冲区
    let pixel_count = (bmp_w as usize) * (bmp_h as usize);
    let mut pixels = vec![
        Pixel32 {
            b: 0,
            g: 0,
            r: 0,
            a: 0
        };
        pixel_count
    ];

    // 设置 bi_height 为负值以请求 top-down 输出，确保 bi_bit_count = 32
    bi.bi_height = -bmp_h;
    bi.bi_bit_count = 32;
    bi.bi_compression = 0;

    // 第二次调用读取像素数据
    if unsafe {
        GetDIBits(
            screen_dc,
            icon_info.hbm_color,
            0,
            bmp_h as u32,
            pixels.as_mut_ptr() as *mut c_void,
            &mut bi as *mut BITMAPINFOHEADER as *mut c_void,
            DIB_RGB_COLORS,
        )
    } == 0
    {
        unsafe {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            DeleteObject(icon_info.hbm_color as HGDIOBJ);
            DeleteObject(icon_info.hbm_mask as HGDIOBJ);
        }
        return;
    }

    unsafe { ReleaseDC(std::ptr::null_mut(), screen_dc) };

    // Remove transparent canvas padding before scaling the icon into its cell.
    let source_w = bmp_w as usize;
    let source_h = bmp_h as usize;
    let (mut pixels, bitmap_w, bitmap_h) = match visible_pixel_bounds(&pixels, source_w, source_h) {
        Some((left, top, width, height)) if width != source_w || height != source_h => {
            let mut cropped = Vec::with_capacity(width * height);
            for row in top..top + height {
                let start = row * source_w + left;
                cropped.extend_from_slice(&pixels[start..start + width]);
            }
            (cropped, width, height)
        }
        _ => (pixels, source_w, source_h),
    };

    // 3. 将 straight alpha (BGRA) 转换为 premultiplied alpha
    //    HBITMAP 像素是 straight alpha: RGB 是原始颜色，A 是透明度
    //    D2D 需要 premultiplied: RGB' = RGB * A/255
    for p in pixels.iter_mut() {
        let a = p.a as u16;
        p.b = ((p.b as u16 * a) / 255) as u8;
        p.g = ((p.g as u16 * a) / 255) as u8;
        p.r = ((p.r as u16 * a) / 255) as u8;
    }

    // 4. 创建 D2D bitmap（显式 premultiplied alpha 格式）
    let bitmap_props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };

    let d2d_bitmap = unsafe {
        target.CreateBitmap(
            D2D_SIZE_U {
                width: bitmap_w as u32,
                height: bitmap_h as u32,
            },
            Some(pixels.as_ptr() as *const c_void),
            (bitmap_w as u32) * 4, // pitch = width * bytes_per_pixel
            &bitmap_props,
        )
    };

    if let Ok(bmp) = d2d_bitmap {
        draw_icon_bitmap(target, &bmp, x, y, size);
        if let Some(cache) = icon_bitmap_cache {
            cache.insert(cache_key, bmp);
        }
    }

    // 5. 清理 GDI 位图资源
    if !icon_info.hbm_color.is_null() {
        unsafe { DeleteObject(icon_info.hbm_color as HGDIOBJ) };
    }
    if !icon_info.hbm_mask.is_null() {
        unsafe { DeleteObject(icon_info.hbm_mask as HGDIOBJ) };
    }
}

fn draw_icon_bitmap(target: &ID2D1DCRenderTarget, bitmap: &ID2D1Bitmap, x: i32, y: i32, size: i32) {
    let bitmap_size = unsafe { bitmap.GetPixelSize() };
    let (left, top, right, bottom) =
        icon_destination_rect(bitmap_size.width, bitmap_size.height, x, y, size);
    let dest_rect = d2d_rect(left, top, right, bottom);
    unsafe {
        target.DrawBitmap(
            bitmap,
            Some(&dest_rect),
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
    }
}

/// 从 HWND 获取 GridWindowData 引用（d2d_render 内部使用）
unsafe fn get_data<'a>(hwnd: HWND) -> Option<&'a mut GridWindowData> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GridWindowData;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

/// 绘制图标及其标签文字（分类框与覆盖区域共用）
///
/// 统一处理：图标绘制（`draw_icon_d2d`）+ DirectWrite 文本渲染（含截断、阴影）。
/// 确保分类框和覆盖区域的图标渲染、文本、大小完全一致。
///
/// # 参数
/// - `target`: D2D render target
/// - `dw_factory`: DirectWrite factory
/// - `wic_factory`: WIC factory（可选，用于图标绘制）
/// - `icon_handle`: 图标句柄（HICON）
/// - `display_name`: 标签文字
/// - `icon_x`, `icon_y`: 图标左上角坐标
/// - `icon_size`: 图标渲染尺寸（正方形）
/// - `label_x`, `label_y`: 标签区域左上角坐标
/// - `label_w`, `label_h`: 标签区域尺寸
/// - `dpi`: 当前 DPI（用于字体缩放和阴影偏移）
/// - `apply_ellipsis`: 是否应用字符级省略号截断（未选中项 = true，选中项 = false）
pub fn draw_icon_with_label(
    target: &ID2D1DCRenderTarget,
    dw_factory: &IDWriteFactory,
    wic_factory: Option<&IWICImagingFactory>,
    icon_handle: HICON,
    display_name: &str,
    icon_x: i32,
    icon_y: i32,
    icon_size: i32,
    label_x: f32,
    label_y: f32,
    label_w: f32,
    label_h: f32,
    dpi: u32,
    apply_ellipsis: bool,
) {
    draw_icon_with_label_cached(
        target,
        dw_factory,
        wic_factory,
        icon_handle,
        display_name,
        icon_x,
        icon_y,
        icon_size,
        label_x,
        label_y,
        label_w,
        label_h,
        dpi,
        apply_ellipsis,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_icon_with_label_cached(
    target: &ID2D1DCRenderTarget,
    // 已不再直接使用：文字布局改从 render_devices 的线程内缓存取。
    // 保留参数只为了不动 draw_icon_with_label 的公开签名
    // （桌面覆盖窗口也在调它）。
    _dw_factory: &IDWriteFactory,
    wic_factory: Option<&IWICImagingFactory>,
    icon_handle: HICON,
    display_name: &str,
    icon_x: i32,
    icon_y: i32,
    icon_size: i32,
    label_x: f32,
    label_y: f32,
    label_w: f32,
    label_h: f32,
    dpi: u32,
    apply_ellipsis: bool,
    icon_bitmap_cache: Option<&mut HashMap<(usize, i32), ID2D1Bitmap>>,
) {
    unsafe {
        // 1. 绘制图标
        if !icon_handle.is_null() {
            draw_icon_d2d(
                target,
                wic_factory,
                icon_handle,
                icon_x,
                icon_y,
                icon_size,
                icon_bitmap_cache,
            );
        }

        // 2. 绘制标签文字
        if display_name.is_empty() {
            return;
        }

        let label_font_size = scale_for_dpi(13, dpi) as f32;
        // 图标标签格式全进程缓存：每个分类框每帧每个图标都会用到，
        // 旧实现每次重新 CreateTextFormat，是分类框定时器最主要的对象churn来源。
        let label_spec = TextFormatSpec::new(
            "Microsoft YaHei UI",
            label_font_size,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            DWRITE_WORD_WRAPPING_WRAP,
        );
        let text_wide: Vec<u16> = display_name.encode_utf16().collect();
        // 省略号裁剪已在 text_layout 内部（入缓存前）一次性设定完毕，
        // 所以 apply_ellipsis 是缓存键的一部分；命中后不得再改布局。
        let cached_layout = super::render_devices::text_layout(
            &text_wide,
            &label_spec,
            label_w,
            label_h,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            apply_ellipsis,
        );

        if let Some(ref layout) = cached_layout {
            // 4 方向阴影描边
            let shadow_offset = scale_for_dpi(1, dpi) as f32;
            if let Ok(shadow) = cached_brush(target, 0, 0, 0, 200) {
                for &(dx, dy) in &[
                    (shadow_offset, 0.0),
                    (-shadow_offset, 0.0),
                    (0.0, shadow_offset),
                    (0.0, -shadow_offset),
                ] {
                    target.DrawTextLayout(
                        Vector2 {
                            X: label_x + dx,
                            Y: label_y + dy,
                        },
                        layout,
                        &shadow,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }

            // 主文字（浅灰白色）
            if let Ok(brush) = cached_brush(target, 250, 250, 255, 255) {
                target.DrawTextLayout(
                    Vector2 {
                        X: label_x,
                        Y: label_y,
                    },
                    layout,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
    } // unsafe
}

// =============================================================================
// 拖拽覆盖位图渲染（D2D + DirectWrite，高质量）
// =============================================================================

/// 为拖拽覆盖窗口渲染复合位图（图标 + 标签文字）
///
/// 使用 Direct2D + DirectWrite 渲染，与分类框内图标标签质量一致。
/// 返回 32-bit ARGB HBITMAP（premultiplied alpha），
/// 可直接用于 `UpdateLayeredWindow`。
///
/// # 参数
/// - `icon_handle`: 图标句柄（128×128 高清图标）
/// - `icon_size`: 图标渲染尺寸（DPI 自适应）
/// - `display_name`: 标签文字
/// - `total_w`: 位图总宽度
/// - `total_h`: 位图总高度（图标 + gap + 标签区域）
/// - `gap`: 图标与标签之间的间距（DPI 自适应，6px at 96 DPI）
/// - `label_h`: 标签区域高度（2 行文字高度，36px at 96 DPI）
/// - `dpi`: 当前 DPI（用于字体缩放）
pub fn render_drag_overlay_bitmap(
    icon_handle: HICON,
    icon_size: i32,
    display_name: &str,
    total_w: i32,
    total_h: i32,
    gap: i32,
    label_h: i32,
    dpi: u32,
) -> HBITMAP {
    unsafe {
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return std::ptr::null_mut();
        }

        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return std::ptr::null_mut();
        }

        // 创建 32-bit top-down DIB Section
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

        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(
            mem_dc,
            &bi as *const BITMAPINFOHEADER as *const c_void,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );

        if dib.is_null() || bits.is_null() {
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return std::ptr::null_mut();
        }

        let old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);

        // D2D 工厂与 DC 渲染目标改用全进程共享工厂（见 render_devices）。
        // 旧实现每次重建拖拽位图都新建一个 ID2D1Factory，等价于新建一个 D3D11 设备。
        let Some(target) = super::render_devices::create_dc_render_target(96.0) else {
            log::warn!("拖拽覆盖: 共享 DC RenderTarget 创建失败");
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return std::ptr::null_mut();
        };

        // 绑定 D2D render target 到 memory DC
        let bind_rect = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: total_w,
            bottom: total_h,
        };
        if target.BindDC(GdiHDC(mem_dc), &bind_rect).is_err() {
            log::warn!("拖拽覆盖: BindDC 失败");
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return std::ptr::null_mut();
        }

        // ===== D2D 渲染开始 =====
        target.BeginDraw();

        // 清空为全透明
        target.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        // 绘制图标 + 标签文字（公共方法，与分类框完全一致）
        let icon_x = (total_w - icon_size) / 2;
        let label_x = 2.0_f32;
        let label_y = (icon_size + gap) as f32;
        let label_w = (total_w - 4) as f32;

        // DirectWrite 工厂改为全进程共享，避免每次重建拖拽位图都新建一个工厂
        let Some(dw_factory) = super::render_devices::dw_factory() else {
            log::warn!("拖拽覆盖: 共享 DirectWrite Factory 不可用");
            let _ = target.EndDraw(None, None);
            SelectObject(mem_dc, old_bmp);
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return dib;
        };

        draw_icon_with_label(
            &target,
            &dw_factory,
            None,
            icon_handle,
            display_name,
            icon_x,
            0,
            icon_size,
            label_x,
            label_y,
            label_w,
            label_h as f32,
            dpi,
            true, // 覆盖区域始终应用省略号截断
        );

        let _ = target.EndDraw(None, None);

        // 清理
        SelectObject(mem_dc, old_bmp);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);

        dib
    }
}

/// 计算边缘拖拽指示条的矩形区域
///
/// 根据 hover_edge 值计算边缘条的矩形，边缘条避开圆角区域以避免视觉冲突。
///
/// # Arguments
/// * `hover_edge` - 边缘标识：0=无, 1=左, 2=右, 3=上, 4=下
/// * `window_width` - 窗口宽度
/// * `window_height` - 窗口高度  
/// * `border_width` - 边缘条宽度
/// * `corner_radius` - 圆角半径
///
/// # Returns
/// (left, top, right, bottom) 格式的矩形坐标
pub fn compute_edge_rect(
    hover_edge: i32,
    window_width: f32,
    window_height: f32,
    border_width: f32,
    corner_radius: f32,
) -> (f32, f32, f32, f32) {
    match hover_edge {
        1 => (
            0.0,
            corner_radius,
            border_width,
            window_height - corner_radius,
        ), // 左边缘
        2 => (
            window_width - border_width,
            corner_radius,
            window_width,
            window_height - corner_radius,
        ), // 右边缘
        3 => (
            corner_radius,
            0.0,
            window_width - corner_radius,
            border_width,
        ), // 上边缘
        4 => (
            corner_radius,
            window_height - border_width,
            window_width - corner_radius,
            window_height,
        ), // 下边缘
        _ => (0.0, 0.0, 0.0, 0.0), // 无边缘
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== compute_edge_rect 单元测试 =====
    // 测试边缘拖拽指示条的矩形计算，验证边缘条避开圆角区域。

    #[test]
    fn test_edge_rect_left_edge() {
        let (l, t, r, b) = compute_edge_rect(1, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 0.0);
        assert_eq!(t, 16.0); // 避开上方圆角
        assert_eq!(r, 3.0);
        assert_eq!(b, 584.0); // 避开下方圆角 (600 - 16)
    }

    #[test]
    fn test_edge_rect_right_edge() {
        let (l, t, r, b) = compute_edge_rect(2, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 397.0); // 400 - 3
        assert_eq!(t, 16.0); // 避开上方圆角
        assert_eq!(r, 400.0);
        assert_eq!(b, 584.0); // 避开下方圆角
    }

    #[test]
    fn test_edge_rect_top_edge() {
        let (l, t, r, b) = compute_edge_rect(3, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 16.0); // 避开左侧圆角
        assert_eq!(t, 0.0);
        assert_eq!(r, 384.0); // 避开右侧圆角 (400 - 16)
        assert_eq!(b, 3.0);
    }

    #[test]
    fn test_edge_rect_bottom_edge() {
        let (l, t, r, b) = compute_edge_rect(4, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 16.0); // 避开左侧圆角
        assert_eq!(t, 597.0); // 600 - 3
        assert_eq!(r, 384.0); // 避开右侧圆角
        assert_eq!(b, 600.0);
    }

    #[test]
    fn test_edge_rect_no_edge() {
        let (l, t, r, b) = compute_edge_rect(0, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 0.0);
        assert_eq!(t, 0.0);
        assert_eq!(r, 0.0);
        assert_eq!(b, 0.0);
    }

    #[test]
    fn test_edge_rect_invalid_edge() {
        let (l, t, r, b) = compute_edge_rect(5, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 0.0);
        assert_eq!(t, 0.0);
        assert_eq!(r, 0.0);
        assert_eq!(b, 0.0);
    }

    #[test]
    fn test_edge_rect_negative_edge() {
        let (l, t, r, b) = compute_edge_rect(-1, 400.0, 600.0, 3.0, 16.0);
        assert_eq!(l, 0.0);
        assert_eq!(t, 0.0);
        assert_eq!(r, 0.0);
        assert_eq!(b, 0.0);
    }

    /// 验证边缘条不超出窗口边界
    #[test]
    fn test_edge_rect_within_bounds() {
        for edge in 1..=4 {
            let (l, t, r, b) = compute_edge_rect(edge, 400.0, 600.0, 3.0, 16.0);
            assert!(l >= 0.0, "left edge {} should be >= 0", l);
            assert!(t >= 0.0, "top edge {} should be >= 0", t);
            assert!(r <= 400.0, "right edge {} should be <= 400", r);
            assert!(b <= 600.0, "bottom edge {} should be <= 600", b);
        }
    }

    /// 验证边缘条避开圆角区域（边缘条起点/终点不应在圆角区域内）
    #[test]
    fn test_edge_rect_avoids_corners() {
        let corner_r = 16.0_f32;
        let ww = 400.0_f32;
        let hh = 600.0_f32;
        let bw = 3.0_f32;

        // 左边缘：top >= corner_r, bottom <= hh - corner_r
        let (_l, t, _r, b) = compute_edge_rect(1, ww, hh, bw, corner_r);
        assert!(t >= corner_r, "left edge top should be >= corner_r");
        assert!(
            b <= hh - corner_r,
            "left edge bottom should be <= hh - corner_r"
        );

        // 右边缘：top >= corner_r, bottom <= hh - corner_r
        let (_l, t, _r, b) = compute_edge_rect(2, ww, hh, bw, corner_r);
        assert!(t >= corner_r, "right edge top should be >= corner_r");
        assert!(
            b <= hh - corner_r,
            "right edge bottom should be <= hh - corner_r"
        );

        // 上边缘：left >= corner_r, right <= ww - corner_r
        let (l, _t, r, _b) = compute_edge_rect(3, ww, hh, bw, corner_r);
        assert!(l >= corner_r, "top edge left should be >= corner_r");
        assert!(
            r <= ww - corner_r,
            "top edge right should be <= ww - corner_r"
        );

        // 下边缘：left >= corner_r, right <= ww - corner_r
        let (l, _t, r, _b) = compute_edge_rect(4, ww, hh, bw, corner_r);
        assert!(l >= corner_r, "bottom edge left should be >= corner_r");
        assert!(
            r <= ww - corner_r,
            "bottom edge right should be <= ww - corner_r"
        );
    }

    /// 验证不同 DPI 缩放下的边缘条宽度计算（scale_for_dpi 逻辑验证）
    #[test]
    fn test_edge_rect_with_dpi_scaled_border() {
        // 模拟 150% DPI：scale_for_dpi(3, 144) = 3 * 144 / 96 = 4
        let border_w_150 = 4.0_f32;
        let (l, _t, r, _b) = compute_edge_rect(1, 400.0, 600.0, border_w_150, 16.0);
        assert_eq!(r - l, 4.0, "border width should match DPI-scaled value");

        // 模拟 200% DPI：scale_for_dpi(3, 192) = 3 * 192 / 96 = 6
        let border_w_200 = 6.0_f32;
        let (l, _t, r, _b) = compute_edge_rect(2, 400.0, 600.0, border_w_200, 16.0);
        assert_eq!(r - l, 6.0, "border width should match DPI-scaled value");
    }

    /// 验证小窗口场景（窗口尺寸接近圆角直径）
    #[test]
    fn test_edge_rect_small_window() {
        // 窗口 40x40，圆角 16：边缘条只有 40-16*2=8 像素可用
        let (_l, t, _r, b) = compute_edge_rect(1, 40.0, 40.0, 3.0, 16.0);
        assert!(
            t < b,
            "left edge should have positive height: t={}, b={}",
            t,
            b
        );
        assert_eq!(b - t, 8.0, "edge height should be 40 - 16*2 = 8");
    }
    // ===== ID2D1DCRenderTarget 是否保留 DC 既有内容 =====
    //
    // 这条测试是一个**架构决策的前置事实**，不是普通的回归防护。
    //
    // 液态玻璃的像素在 `render_background` 之后已经完整躺在
    // `data.glass_pixels`（CPU 侧、预乘 BGRA、行距与主 DIB 完全一致）。
    // 现在的做法是：把它 `CopyFromMemory` 上传成一张 D2D 位图，
    // 再 `Clear` 整窗、再 `DrawBitmap` 全窗软件混合 —— 三次全窗
    // 内存往返，实测占单帧 18.07ms 里的 4~5ms，另外还常驻一张
    // 整窗 D2D 位图（5 框合计 24MB）。
    //
    // 如果 D2D1 的 DC 渲染目标在 `BeginDraw` 时**保留** DC 里已有的
    // 像素，就可以直接 memcpy 玻璃进主 DIB，然后只让 D2D 在上面叠
    // 一层内容：省掉 Clear、省掉玻璃那次全窗 blit、省掉整张玻璃位图。
    // 如果不保留，memcpy 会被 BeginDraw 抹掉，画面直接变黑 ——
    // 而这类错误不会有任何返回值或日志，只能靠像素级验证。
    //
    // 所以这里用一个 8x8 的最小 DIB 把这件事钉死：先自己填红，
    // 再让 D2D 只画左上角 4x4 蓝块且**不 Clear**，最后逐像素回读。
    #[test]
    fn dc_render_target_preserves_existing_dib_pixels() {
        const W: i32 = 8;
        const H: i32 = 8;
        unsafe {
            let screen_dc = GetDC(std::ptr::null_mut());
            assert!(!screen_dc.is_null(), "拿不到屏幕 DC，无法验证");
            let mem_dc = CreateCompatibleDC(screen_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            assert!(!mem_dc.is_null(), "CreateCompatibleDC 失败");

            let bi = BITMAPINFOHEADER {
                bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                bi_width: W,
                bi_height: -H, // 自上而下，与分类框主 DIB 一致
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
            let dib = CreateDIBSection(
                mem_dc,
                &bi as *const BITMAPINFOHEADER as *const c_void,
                DIB_RGB_COLORS,
                &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
                std::ptr::null_mut(),
                0,
            );
            if dib.is_null() || bits.is_null() {
                DeleteDC(mem_dc);
                panic!("CreateDIBSection 失败，无法验证 DC 渲染目标语义");
            }
            let old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);

            // 整张填不透明红（预乘 BGRA：b=0 g=0 r=255 a=255）
            for index in 0..(W * H) as isize {
                *bits.offset(index) = Pixel32 {
                    b: 0,
                    g: 0,
                    r: 255,
                    a: 255,
                };
            }

            let Some(target) = super::super::render_devices::create_dc_render_target(96.0) else {
                SelectObject(mem_dc, old_bmp);
                DeleteObject(dib as HGDIOBJ);
                DeleteDC(mem_dc);
                eprintln!("跳过：当前环境建不出 DC RenderTarget");
                return;
            };

            let bind = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: W,
                bottom: H,
            };
            target.BindDC(GdiHDC(mem_dc), &bind).expect("BindDC 失败");
            target.BeginDraw();
            // 关键：**不调用 Clear**，只在左上角画 4x4 不透明蓝
            let blue = target
                .CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    None,
                )
                .expect("建画刷失败");
            target.FillRectangle(&d2d_rect(0.0, 0.0, 4.0, 4.0), &blue);
            let mut tag1: u64 = 0;
            let mut tag2: u64 = 0;
            target
                .EndDraw(Some(&mut tag1), Some(&mut tag2))
                .expect("EndDraw 失败");

            let top_left = *bits; // (0,0) 应为蓝
            let bottom_right = *bits.offset((W * H - 1) as isize); // (7,7) 应仍为红

            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);

            assert_eq!(
                (top_left.b, top_left.g, top_left.r, top_left.a),
                (255, 0, 0, 255),
                "D2D 画的蓝块没落到 DIB 上（预乘 BGRA 里蓝是 b=255）"
            );
            assert_eq!(
                (
                    bottom_right.b,
                    bottom_right.g,
                    bottom_right.r,
                    bottom_right.a
                ),
                (0, 0, 255, 255),
                "BeginDraw 抹掉了 DC 既有内容：不能直接把玻璃 memcpy 进主 DIB"
            );
        }
    }
    /// 连续两帧都「先直写 DIB、再 BindDC + BeginDraw（不 Clear）」，
    /// 第二帧的直写内容必须照样保留。
    ///
    /// 单帧通过不足以支撑改造：真实渲染循环是**同一个 mem_dc 上反复
    /// BindDC**。如果 D2D1 在首次 BindDC 时缓存了表面、之后不再从 DC
    /// 重新读入，第二帧的 memcpy 就会被上一帧的内部表面覆盖掉 ——
    /// 表现为玻璃「冻在第一帧」，而且没有任何报错。
    #[test]
    fn dc_render_target_rereads_dib_on_repeated_binddc() {
        const W: i32 = 8;
        const H: i32 = 8;
        unsafe {
            let screen_dc = GetDC(std::ptr::null_mut());
            assert!(!screen_dc.is_null(), "拿不到屏幕 DC");
            let mem_dc = CreateCompatibleDC(screen_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            assert!(!mem_dc.is_null(), "CreateCompatibleDC 失败");
            let bi = BITMAPINFOHEADER {
                bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                bi_width: W,
                bi_height: -H, // 自上而下，与分类框主 DIB 一致
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
            let dib = CreateDIBSection(
                mem_dc,
                &bi as *const BITMAPINFOHEADER as *const c_void,
                DIB_RGB_COLORS,
                &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
                std::ptr::null_mut(),
                0,
            );
            if dib.is_null() || bits.is_null() {
                DeleteDC(mem_dc);
                panic!("CreateDIBSection 失败");
            }
            let old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);
            let Some(target) = super::super::render_devices::create_dc_render_target(96.0) else {
                SelectObject(mem_dc, old_bmp);
                DeleteObject(dib as HGDIOBJ);
                DeleteDC(mem_dc);
                eprintln!("跳过：当前环境建不出 DC RenderTarget");
                return;
            };
            let bind = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: W,
                bottom: H,
            };
            let fill = |value: Pixel32| {
                for index in 0..(W * H) as isize {
                    *bits.offset(index) = value;
                }
            };
            // 每帧只在左上角画一个 4x4 蓝块，其余像素全靠直写 DIB 提供
            let draw_corner = |target: &ID2D1DCRenderTarget| {
                target.BindDC(GdiHDC(mem_dc), &bind).expect("BindDC 失败");
                target.BeginDraw();
                let blue = target
                    .CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: 0.0,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        },
                        None,
                    )
                    .expect("建画刷失败");
                target.FillRectangle(&d2d_rect(0.0, 0.0, 4.0, 4.0), &blue);
                let mut t1: u64 = 0;
                let mut t2: u64 = 0;
                target
                    .EndDraw(Some(&mut t1), Some(&mut t2))
                    .expect("EndDraw 失败");
            };

            // 第 1 帧：底色填红
            fill(Pixel32 {
                b: 0,
                g: 0,
                r: 255,
                a: 255,
            });
            draw_corner(&target);
            let frame1_corner = *bits;
            let frame1_rest = *bits.offset((W * H - 1) as isize);

            // 第 2 帧：同一个 DC 再 BindDC 一次，底色改填**半透明**绿。
            //
            // 必须用半透明：不透明像素下「预乘」与「直通」alpha 的字节
            // 完全相同，测不出 D2D 有没有在读入/写回时偷偷做一次
            // premultiply ↔ unpremultiply 转换。液态玻璃的外投影整圈
            // 都是低 alpha 像素，一旦被转换过就会整体发灰或发黑，
            // 而这种损坏不会有任何返回值或日志。
            // 取 a=128、g=200：预乘值 200*128/255=100.4 → 若 D2D 当作
            // 直通 alpha 再乘一次，写回会变成 200*128/255*128/255≈50，
            // 与 200 差 150，断言必然炸。
            fill(Pixel32 {
                b: 0,
                g: 200,
                r: 0,
                a: 128,
            });
            draw_corner(&target);
            let frame2_corner = *bits;
            let frame2_rest = *bits.offset((W * H - 1) as isize);

            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);

            assert_eq!(
                (frame1_corner.b, frame1_corner.a),
                (255, 255),
                "第1帧蓝块未画上"
            );
            assert_eq!(
                (frame1_rest.g, frame1_rest.r),
                (0, 255),
                "第1帧直写的红底被 BeginDraw 抹掉"
            );
            assert_eq!(
                (frame2_corner.b, frame2_corner.a),
                (255, 255),
                "第2帧蓝块未画上"
            );
            assert_eq!(
                (frame2_rest.b, frame2_rest.g, frame2_rest.r, frame2_rest.a),
                (0, 200, 0, 128),
                "半透明像素未字节级原样往返：D2D 在 BindDC/EndDraw 之间做了 \
                 premultiply/unpremultiply 转换，不能把玻璃直接 memcpy 进主 DIB"
            );
        }
    }
    // ===== 纯 CPU 预乘合成的正确性 =====
    //
    // 这条路径取代了 D2D 的 `DrawBitmap`，一旦公式写错就是**静默的**
    // 全画面偏色：不崩、不报错、日志干净，只有肉眼能发现。
    // 所以这里对 `over_channel` 做 256×256 全输入穷举，
    // 与「四舍五入的精确除法」逐位对齐。

    /// `(t + 128 + (t >> 8)) >> 8` 必须等于 round(dst × inv / 255)，
    /// 对全部 65536 种 (dst, inv) 组合成立。
    #[test]
    fn over_channel_matches_exact_rounding_division() {
        for dst in 0u32..=255 {
            for inv in 0u32..=255 {
                // 必须调**生产函数**本身。src=0 时返回值就是
                // round(dst*inv/255)（dst*inv/255 恒 <= 255，不会被 clamp 改变）。
                //
                // 这里曾经把公式在测试里重抄一遍再自比 —— 那样即使
                // 生产代码写错也照样通过，等于没测。抄写版本还恰好
                // 抄的是错误公式，掩盖了真实的 off-by-one。
                let fast = super::over_channel(0, dst as u8, inv) as u32;
                // 精确值：四舍五入的 dst*inv/255
                let exact = ((dst * inv) as f64 / 255.0).round() as u32;
                assert_eq!(
                    fast, exact,
                    "dst={dst} inv={inv}: 快速除法 {fast} != 精确 {exact}"
                );
            }
        }
    }

    /// 结果必须落在 [0,255]，且 inv=0（源完全不透明）时只剩 src。
    #[test]
    fn over_channel_stays_in_range() {
        for dst in 0u16..=255 {
            for inv in 0u16..=255 {
                let v = super::over_channel(200, dst as u8, inv as u32);
                assert!((v as u16) <= 255);
            }
        }
        // inv=0 意味着 srcAlpha=255：目标完全不参与
        assert_eq!(super::over_channel(37, 255, 0), 37);
        // inv=255 意味着 srcAlpha=0：只剩目标
        assert_eq!(super::over_channel(0, 123, 255), 123);
    }

    /// 合成的三条硬性质：全透明不动目标、全不透明完全覆盖、
    /// 越界原点被夹住而不是写坏相邻内存。
    #[test]
    fn composite_respects_alpha_and_clips() {
        const W: i32 = 6;
        const H: i32 = 4;
        let glass = Pixel32 {
            b: 10,
            g: 20,
            r: 30,
            a: 255,
        };
        let mut dst = vec![glass; (W * H) as usize];

        // 源：第 0 行全透明、第 1 行全不透明、第 2 行半透明、第 3 行全透明
        let mut src = vec![
            Pixel32 {
                b: 0,
                g: 0,
                r: 0,
                a: 0
            };
            (W * H) as usize
        ];
        for x in 0..W as usize {
            src[W as usize + x] = Pixel32 {
                b: 200,
                g: 100,
                r: 50,
                a: 255,
            };
            src[2 * W as usize + x] = Pixel32 {
                b: 128,
                g: 128,
                r: 128,
                a: 128,
            };
        }

        unsafe {
            super::composite_premultiplied_over(dst.as_mut_ptr(), W, H, src.as_ptr(), W, H, 0, 0);
        }
        // 第 0 行：全透明 → 原样
        assert_eq!((dst[0].b, dst[0].g, dst[0].r, dst[0].a), (10, 20, 30, 255));
        // 第 1 行：全不透明 → 完全覆盖
        let r1 = dst[W as usize];
        assert_eq!((r1.b, r1.g, r1.r, r1.a), (200, 100, 50, 255));
        // 第 2 行：半透明 → src + round(dst*(255-128)/255)
        let r2 = dst[2 * W as usize];
        assert_eq!(r2.b, 128 + ((10u32 * 127 + 128) / 255) as u8);
        assert_eq!(r2.a, 128 + ((255u32 * 127 + 128) / 255) as u8);

        // 越界：原点 (-3,-2) 且源比目标大，必须夹住且不 panic / 不越界写
        let mut small = vec![glass; 4];
        let big = vec![
            Pixel32 {
                b: 9,
                g: 9,
                r: 9,
                a: 255
            };
            100
        ];
        unsafe {
            super::composite_premultiplied_over(
                small.as_mut_ptr(),
                2,
                2,
                big.as_ptr(),
                10,
                10,
                -3,
                -2,
            );
        }
        assert_eq!(small.len(), 4);
        assert_eq!((small[0].b, small[0].r), (9, 9), "夹取后应只覆盖可见部分");

        // 完全在目标之外：一个像素都不该动
        let mut untouched = vec![glass; 4];
        unsafe {
            super::composite_premultiplied_over(
                untouched.as_mut_ptr(),
                2,
                2,
                big.as_ptr(),
                2,
                2,
                50,
                50,
            );
        }
        assert!(untouched.iter().all(|p| p.b == 10 && p.r == 30));
    }
    /// **A/B 等价性验证**：纯 CPU 合成必须与 D2D `DrawBitmap` 逐像素一致。
    ///
    /// 快路径把「内容层叠到玻璃上」从 D2D 换成了手写循环。这类替换的
    /// 失败模式是最坏的一种 —— 不崩、不报错、日志干净，只是整幅画面
    /// 偏色或边缘发虚，只有肉眼能发现。所以这里用真实 D2D 做对照：
    /// 同一张玻璃底、同一张内容层，一份走 CPU 合成、一份走 D2D
    /// `DrawBitmap`，然后逐像素比。
    ///
    /// 允许 ±1 LSB：两条路径的中间舍入位置不同（D2D 内部精度更高），
    /// 但预乘 source-over 满足结合律，理论上最多差一个最低位。
    /// 超过 1 就说明公式或 alpha 语义写错了。
    #[test]
    fn cpu_composite_matches_d2d_drawbitmap() {
        const W: usize = 64;
        const H: usize = 48;
        // 确定性伪随机：同一个种子两次运行必须给出同一张图，
        // 否则测试失败时无法复现。
        let mut state: u32 = 0x1234_5678;
        let mut rnd = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        };
        // 合法预乘像素：先取 alpha，再取不超过 alpha 的颜色分量
        let mut premultiplied_pixel = |rnd: &mut dyn FnMut() -> u8| {
            let a = rnd();
            let pick = |v: u8| ((v as u16 * a as u16) / 255) as u8;
            Pixel32 {
                b: pick(rnd()),
                g: pick(rnd()),
                r: pick(rnd()),
                a,
            }
        };
        let glass: Vec<Pixel32> = (0..W * H).map(|_| premultiplied_pixel(&mut rnd)).collect();
        let content: Vec<Pixel32> = (0..W * H).map(|_| premultiplied_pixel(&mut rnd)).collect();

        // ---- 路径 A：纯 CPU 合成 ----
        let mut cpu = glass.clone();
        unsafe {
            super::composite_premultiplied_over(
                cpu.as_mut_ptr(),
                W as i32,
                H as i32,
                content.as_ptr(),
                W as i32,
                H as i32,
                0,
                0,
            );
        }

        // ---- 路径 B：真实 D2D DrawBitmap ----
        unsafe {
            let screen_dc = GetDC(std::ptr::null_mut());
            assert!(!screen_dc.is_null());
            let mem_dc = CreateCompatibleDC(screen_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            assert!(!mem_dc.is_null());
            let bi = BITMAPINFOHEADER {
                bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                bi_width: W as i32,
                bi_height: -(H as i32),
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
            let dib = CreateDIBSection(
                mem_dc,
                &bi as *const BITMAPINFOHEADER as *const c_void,
                DIB_RGB_COLORS,
                &mut bits as *mut *mut Pixel32 as *mut *mut c_void,
                std::ptr::null_mut(),
                0,
            );
            if dib.is_null() || bits.is_null() {
                DeleteDC(mem_dc);
                panic!("CreateDIBSection 失败");
            }
            let old_bmp = SelectObject(mem_dc, dib as HGDIOBJ);
            std::ptr::copy_nonoverlapping(glass.as_ptr(), bits, W * H);

            let Some(target) = super::super::render_devices::create_dc_render_target(96.0) else {
                SelectObject(mem_dc, old_bmp);
                DeleteObject(dib as HGDIOBJ);
                DeleteDC(mem_dc);
                eprintln!("跳过：建不出 DC RenderTarget");
                return;
            };
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            let bitmap = target
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: W as u32,
                        height: H as u32,
                    },
                    Some(content.as_ptr() as *const c_void),
                    (W * 4) as u32,
                    &props,
                )
                .expect("CreateBitmap 失败");
            let bind = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: W as i32,
                bottom: H as i32,
            };
            target.BindDC(GdiHDC(mem_dc), &bind).expect("BindDC 失败");
            target.BeginDraw();
            // 故意不 Clear：与生产快路径一致，玻璃已经在 DIB 里
            target.DrawBitmap(
                &bitmap,
                Some(&d2d_rect(0.0, 0.0, W as f32, H as f32)),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
            let mut t1: u64 = 0;
            let mut t2: u64 = 0;
            target
                .EndDraw(Some(&mut t1), Some(&mut t2))
                .expect("EndDraw 失败");

            let mut max_diff: i32 = 0;
            let mut differing = 0usize;
            for i in 0..W * H {
                let a = *cpu.get_unchecked(i);
                let b = *bits.add(i);
                for (x, y) in [(a.b, b.b), (a.g, b.g), (a.r, b.r), (a.a, b.a)] {
                    let d = (x as i32 - y as i32).abs();
                    if d > max_diff {
                        max_diff = d;
                    }
                }
                if a.b != b.b || a.g != b.g || a.r != b.r || a.a != b.a {
                    differing += 1;
                }
            }
            SelectObject(mem_dc, old_bmp);
            DeleteObject(dib as HGDIOBJ);
            DeleteDC(mem_dc);

            eprintln!(
                "A/B 比对: {W}x{H} = {} 像素，最大通道差 {max_diff}，有差异的像素 {differing} 个",
                W * H
            );
            assert!(
                max_diff <= 1,
                "CPU 合成与 D2D DrawBitmap 偏差 {max_diff} LSB（> 1）：\
                 预乘/alpha 语义或舍入公式写错了"
            );
        }
    }
}
