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
use std::collections::HashMap;
use std::sync::Mutex;

// ===== Windows crate D2D imports =====
//
// D2D / DirectWrite / WIC 工厂已收敛到 `render_devices` 按渲染线程共享，
// 窗口只保留自己的 DC 渲染目标与图标位图缓存。
use windows::Win32::Foundation::HWND as WindowsHwnd;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1Bitmap1, ID2D1DCRenderTarget, ID2D1DeviceContext,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
};
use windows::Win32::Graphics::DirectComposition::{
    IDCompositionTarget, IDCompositionVisual, IDCompositionVisual2,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISurface, IDXGISwapChain1, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};

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
/// 一个分类框的 DirectComposition 呈现栈。
///
/// # 为什么要换掉 `UpdateLayeredWindow`
///
/// 分层窗口的画面必须来自 CPU 侧的 GDI DIB，于是玻璃每帧都要
/// GPU 渲染 → `Map` 阻塞等 GPU 排空 → 逐行 memcpy 回 CPU → 合成
/// → `UpdateLayeredWindow` 再交给 DWM。实测这条链占单帧 20ms 里的
/// 16ms，而真正的模糊 + 着色只有 0.44ms —— 97% 的时间在搬像素。
///
/// DComp 让画面**全程留在 GPU**：玻璃输出纹理零拷贝包成 D2D 位图，
/// 与内容层一起在合成交换链的后缓冲上合成，`Present` 交给 DWM。
///
/// # 逐像素 alpha 为什么不会丢
///
/// 交换链用 `DXGI_ALPHA_MODE_PREMULTIPLIED` 创建，由 DWM 做预乘
/// source-over 合成 —— 与 `UpdateLayeredWindow` 的 `AC_SRC_ALPHA`
/// 是同一套语义。玻璃着色器输出的本来就是预乘 alpha
/// （`PSMain` 末尾 `return float4(col*glassAlpha, alpha)`），
/// 所以 128px 外投影那一圈渐变 alpha 照常成立，圆角也不会发黑边。
pub(crate) struct DcompState {
    /// DWM 合成目标（绑定到本窗口 HWND）
    pub target: IDCompositionTarget,
    /// 承载交换链的视觉节点
    pub visual: IDCompositionVisual2,
    /// 合成交换链（预乘 alpha + flip 模型）
    pub swapchain: IDXGISwapChain1,
    /// 共享设备上的 D2D 设备上下文。每窗口一个：
    /// `ID2D1DeviceContext` 同一时刻只能 `SetTarget` 一个目标，
    /// 多窗口共用会互相踩画面。
    pub context: ID2D1DeviceContext,
    /// 当前后缓冲的 D2D 目标位图；`ResizeBuffers` 之后必须重建，
    /// 否则会继续画进已经失效的旧缓冲（表现为画面冻住且不报错）。
    pub backbuffer: Option<ID2D1Bitmap1>,
    /// 内容层在**共享设备**上的位图，只在内容真的变了时重新上传。
    ///
    /// 内容层仍由 `ID2D1DCRenderTarget` 以 GDI 兼容方式光栅化到
    /// `content_dib`（文字 hinting 与改造前逐字节一致），再上传到这里。
    /// 上传只发生在内容变化时，稳态下每帧零上传。
    ///
    /// 类型是 `ID2D1Bitmap1`：它由 `ID2D1DeviceContext::CreateBitmap`
    /// 从内容层 DIB 上传而来，那个重载收 `D2D1_BITMAP_PROPERTIES1`
    /// 并返回 v1+ 位图（v1 的 `ID2D1RenderTarget::CreateBitmap` 才返回
    /// `ID2D1Bitmap`，两者签名不同，混用编译不过）。
    pub content: Option<ID2D1Bitmap1>,
    pub content_w: u32,
    pub content_h: u32,
    /// 交换链当前尺寸（物理像素），与窗口外框一致
    pub width: u32,
    pub height: u32,
    /// 已成功 Present 的帧数，用于周期性的 DWM 合成自证
    pub presented_frames: u64,
}

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
    /// 拖拽覆盖时，光标下的替换高亮项索引（-1 = 无替换高亮）
    pub(crate) drag_over_item: i32,
    /// 拖拽覆盖时，插入指示器的列位置（-1 = 无插入指示）
    pub(crate) drag_over_insert_col: i32,
    /// 拖拽覆盖时，插入指示器的行位置（-1 = 无插入指示）
    pub(crate) drag_over_insert_row: i32,
    /// IDropTarget COM 对象指针（窗口销毁时释放）
    pub(crate) drop_target_ptr: *mut std::ffi::c_void,
    /// 图标网格 Y 轴滚动偏移量（像素，0 = 顶部）
    pub(crate) scroll_offset: i32,

    // ===== 重命名状态 =====
    /// 正在重命名的项索引（-1 = 无重命名）
    pub(crate) renaming_item_index: i32,

    // ===== Multi-select fields =====
    /// Index of the anchor item for shift-click range selection (-1 = no anchor)
    pub(crate) shift_anchor: i32,
    /// Whether marquee selection is currently active
    pub(crate) marquee_active: bool,
    /// Starting position of marquee selection (window coordinates)
    pub(crate) marquee_start_x: i32,
    pub(crate) marquee_start_y: i32,
    /// Current position of marquee selection (window coordinates)
    pub(crate) marquee_current_x: i32,
    pub(crate) marquee_current_y: i32,
    /// 框选快路径：上一拍已合成进内容 DIB 的选框矩形（视口物理像素，已裁剪；
    /// 全零 = DIB 干净无残留）。下一拍先把该区域从干净帧快照还原再混合新框。
    pub(crate) marquee_prev_rect: [i32; 4],
    /// 干净内容帧快照是否可用（任何全量重绘都会使其失效并重取）
    pub(crate) marquee_pristine_valid: bool,
    /// 干净帧快照（BGRA 预乘，行距 = content_w*4）；仅框选期间持有
    pub(crate) marquee_pristine: Vec<u8>,
    /// 框选渲染节流戳（move 驱动路径，12ms）
    pub(crate) marquee_last_render_ms: u64,
    /// 框选收尾强制全量重绘：快路径把选框合成进了内容 DIB，而引擎模式的
    /// 指纹不含 marquee 字段——松手后若无其它状态变化，缓存命中会让选框
    /// 永远留在画面上（2026-11-03 验收反馈「松手后不消失」的根因）。
    /// UP 时置位，让下一趟 render_layered_window 无视指纹缓存重画一次干净帧。
    pub(crate) marquee_force_full: bool,
    /// List of selected item indices for multi-select operations
    pub(crate) multi_drag_items: Vec<usize>,

    // ===== D2D 渲染资源（窗口级） =====
    //
    // D2D / DirectWrite / WIC 工厂已收敛到 `render_devices` 按渲染线程共享：
    // 每个 `ID2D1Factory` 内部都持有一个独立 D3D11 设备，按窗口各建一套会让
    // 5 个分类框额外占用约 200MB 私有内存。窗口只保留自己的 DC 渲染目标。
    pub(crate) d2d_target: Option<ID2D1DCRenderTarget>,
    /// HICON + 目标尺寸对应的 D2D 位图缓存，避免实时玻璃刷新时重复读取 GDI 位图。
    pub(crate) icon_bitmap_cache: HashMap<(usize, i32), ID2D1Bitmap>,

    // ===== 复用的 DIB 渲染后端 =====
    //
    // 旧实现每帧 `CreateDIBSection` + `DeleteObject`：200ms 定时器下 5 个分类框
    // 每秒产生上百 MB 的 GDI 分配/释放，是主进程内存持续爬升的主因之一。
    // 现在 DIB 与内存 DC 长期持有，仅在尺寸变化或窗口销毁时重建。
    /// 32 位 ARGB DIB Section（自上而下），`UpdateLayeredWindow` 的位图源
    pub(crate) dib_section: HBITMAP,
    /// 选中 `dib_section` 的内存 DC，同时是 D2D DC 渲染目标的绑定点
    pub(crate) dib_dc: HDC,
    /// `dib_dc` 中被替换掉的默认位图，释放前需先还原
    pub(crate) dib_old_bmp: HGDIOBJ,
    /// `dib_section` 的像素首地址（预乘 BGRA、自上而下、行距 = dib_w × 4）。
    ///
    /// 引擎架构下该指针只承载「命中捕获层 + 内容层」的预乘 BGRA 像素，
    /// 玻璃底板由 liquid-verre-rs 画布在窗口之下渲染，不再回读进 DIB。
    pub(crate) dib_bits: *mut Pixel32,
    /// 当前 DIB 宽度（物理像素），0 表示尚未创建
    pub(crate) dib_w: i32,
    /// 当前 DIB 高度（物理像素），0 表示尚未创建
    pub(crate) dib_h: i32,

    // ===== DirectComposition 呈现路径 =====
    /// `Some` 表示这个窗口走 DComp 合成交换链呈现；
    /// `None` 表示退回 `WS_EX_LAYERED` + `UpdateLayeredWindow`。
    ///
    /// 两条路径互斥，在窗口创建时一次性定下来，运行期不切换：
    /// 它们要求的扩展窗口样式不同（`WS_EX_NOREDIRECTIONBITMAP` vs
    /// `WS_EX_LAYERED`），而扩展样式改不了已创建窗口的重定向表面行为。
    pub(crate) dcomp: Option<Box<DcompState>>,

    // ===== 内容层缓存 =====
    //
    // 一帧分类框画面 = 玻璃背景（每帧都在变）+ 内容层（图标/标签/标题/
    // 按钮/选中态）。实测一帧 27.6ms 里有 **17.4ms（63%）** 花在内容层的
    // 软件光栅化上：DC 渲染目标的后端是 GDI DIB，没有硬件加速的余地，
    // 而每个标签要画 5 次 DrawTextLayout（4 方向阴影描边 + 正文），
    // 5 个条目就是 25 次字形光栅化。
    //
    // 但用户点名的两个高频场景 —— **拖拽分类框**和**动态壁纸** ——
    // 内容层一个像素都没变，只有玻璃在变。把内容层缓存下来，
    // 每帧就只剩「画玻璃 + 合成一张位图」两个 op。
    //
    // 关键：这条路**不换渲染目标类型、不换字体光栅化方式**，
    // 内容层仍然由同一个软件光栅化器画进同一格式的 DIB，
    // 因此文字与图标可以逐像素不变（由 A/B 自检验证，不靠肉眼）。
    /// 内容层 DIB（透明底，只画玻璃之上的东西）
    pub(crate) content_dib: HBITMAP,
    /// 选中 `content_dib` 的内存 DC
    pub(crate) content_dc: HDC,
    /// `content_dc` 中被替换掉的默认位图，释放前需先还原
    pub(crate) content_old_bmp: HGDIOBJ,
    /// 内容层 DIB 的像素首地址（拷成 D2D 位图时用）
    pub(crate) content_bits: *mut Pixel32,
    pub(crate) content_w: i32,
    pub(crate) content_h: i32,
    /// 内容层的 D2D 位图，每帧合成时只 DrawBitmap 一次
    pub(crate) content_bitmap: Option<ID2D1Bitmap>,
    /// 画进内容层时的状态指纹；不一致就必须重画
    pub(crate) content_fingerprint: u64,
    /// 内容层当前是否可用（尺寸匹配且已画过）
    pub(crate) content_valid: bool,

    /// 当前生效的外投影留白（物理像素）。
    ///
    /// 引擎架构下恒为 0；读取方（渲染、布局、命中测试、跟随）统一读这个字段，
    /// 不再按 DPI 现算，保留历史换算错位防护的读取口径。
    pub(crate) render_padding: i32,

    /// 是否正处于库驱动的缩放会话（引擎块矩形为真值，内容窗口跟随）。
    ///
    /// 由 `WM_USER_GLASS_RESIZE` 事件置位、跟随 tick 确认快照回落后清除；
    /// 会话期间 `WM_WINDOWPOSCHANGED` 不得把窗口几何反向推回引擎（防止回路）。
    pub(crate) glass_session_active: bool,
    /// 接管式标题拖拽会话进行中（几何由库 sink 的按压序列驱动，代理窗
    /// 由 `on_glass_drag_sync` 跟随；收尾走 `end_box_drag`）。
    pub(crate) box_drag_active: bool,
    /// 上次经 UpdateLayeredWindow 真正提交过的外框尺寸与当时的玻璃状态。
    ///
    /// 引擎模式下代理窗是**纯透明命中层**：尺寸没变时再合成 + 提交一遍
    /// 3.5MB 全透明位图是纯白烧（实测光栅合成 ~3.8ms + 提交 ~1.2ms），
    /// 正是「拖拽时标题/图标/按钮掉帧、玻璃背景却流畅」的开销来源。
    /// 上一帧仍然有效，直接跳过；尺寸或玻璃状态变化时必须重新提交。
    pub(crate) submitted_w: i32,
    pub(crate) submitted_h: i32,
    pub(crate) submitted_glass: bool,
    /// 上次真正重画内容层的单调毫秒（会话期节流用，0 = 未知）。
    pub(crate) content_last_render_ms: u64,
    /// 本次库缩放会话是否已观察到过快照 `resizing=true`。
    ///
    /// 事件（ResizeStarted）与快照发布不是同一条通道：空闲档只有 ~4Hz 时，
    /// 会话刚开始的几拍快照仍是 resizing=false。旧实现第一拍读到 false 就收尾，
    /// 于是「拖框边缘 → 内容不跟尺寸 → 会话秒退 → 窗口旧矩形反推回块 →
    /// 块自己弹回旧尺寸」（2026-10-28 用户实测）。
    pub(crate) glass_session_seen_resizing: bool,
    /// 会话内连续看到 resizing=false 的拍数（收尾去抖用）。
    pub(crate) glass_session_idle_ticks: u8,
    /// 本次接管式移动是否真的挪过窗口（懒创建参考线、决定要不要落盘）。
    pub(crate) box_drag_moved: bool,
    /// hover 轮询里连续「光标在块外」的拍数。
    ///
    /// 连续 2 拍在外才补发 `WM_MOUSELEAVE`：单拍越界（DPI 取整、光标贴边缘）
    /// 会让 hover 进出抖动，表现为图标/按钮高亮闪烁。
    pub(crate) hover_out_ticks: u8,
}

unsafe impl Send for GridWindowData {}

/// 从全局多选状态收集所有选中的 GridBox 图标的文件路径
///
/// 遍历所有 GRID_WINDOWS，对每个窗口的 config_id 查找选中的 item_index，
/// 支持跨分类框多选。
pub(crate) fn collect_selected_file_paths() -> Vec<String> {
    use super::multi_select::{
        all_keys, container_id_from_uuid, selected_in_container, ContainerType,
    };

    let mut paths: Vec<String> = Vec::new();

    // 获取所有选中的 SelectionKey 快照
    let keys = all_keys();
    if keys.is_empty() {
        return paths;
    }

    // 遍历所有 Grid 窗口，按 container_id 查找选中项
    let Ok(grid_list) = get_grid_list() else {
        log::warn!("[多选菜单] 无法获取窗口列表");
        return paths;
    };

    for grid in grid_list.iter() {
        let data = match unsafe { get_data(grid.hwnd) } {
            Some(d) => d,
            None => continue,
        };

        let cid = container_id_from_uuid(data.config_id);

        // 查找该容器中选中的索引
        let selected_indices = selected_in_container(ContainerType::GridBox, cid);
        if selected_indices.is_empty() {
            continue;
        }

        for idx in selected_indices {
            if idx < data.items.len() {
                paths.push(data.items[idx].file_path.clone());
            }
        }
    }

    paths
}

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
            drag_over_item: -1,
            drag_over_insert_col: -1,
            drag_over_insert_row: -1,
            drop_target_ptr: std::ptr::null_mut(),
            scroll_offset: 0,
            shift_anchor: -1,
            marquee_active: false,
            marquee_start_x: 0,
            marquee_start_y: 0,
            marquee_current_x: 0,
            marquee_current_y: 0,
            marquee_prev_rect: [0; 4],
            marquee_pristine_valid: false,
            marquee_pristine: Vec::new(),
            marquee_last_render_ms: 0,
            marquee_force_full: false,
            multi_drag_items: Vec::new(),
            renaming_item_index: -1,
            d2d_target: None,
            icon_bitmap_cache: HashMap::new(),
            dcomp: None,
            dib_section: std::ptr::null_mut(),
            dib_bits: std::ptr::null_mut(),
            dib_dc: std::ptr::null_mut(),
            dib_old_bmp: std::ptr::null_mut(),
            dib_w: 0,
            dib_h: 0,
            content_dib: std::ptr::null_mut(),
            content_dc: std::ptr::null_mut(),
            content_old_bmp: std::ptr::null_mut(),
            content_bits: std::ptr::null_mut(),
            content_w: 0,
            content_h: 0,
            content_bitmap: None,
            content_fingerprint: 0,
            content_valid: false,
            render_padding: 0,
            glass_session_active: false,
            box_drag_active: false,
            submitted_w: 0,
            submitted_h: 0,
            submitted_glass: false,
            content_last_render_ms: 0,
            glass_session_seen_resizing: false,
            glass_session_idle_ticks: 0,
            box_drag_moved: false,
            hover_out_ticks: 0,
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

/// 获取分类框窗口列表的锁（供 zorder 模块使用）
pub(super) fn get_grid_list() -> Result<
    std::sync::MutexGuard<'static, Vec<GridWindow>>,
    std::sync::PoisonError<std::sync::MutexGuard<'static, Vec<GridWindow>>>,
> {
    GRID_WINDOWS.lock()
}
static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

/// 库缩放会话期间内容窗口的跟随定时器间隔（毫秒）。
///
/// 16ms ≈ 60Hz，与 DWM 合成节奏对齐；Windows 定时器默认粒度约 15.6ms，
/// 实际 tick 落在 15.6~31.2ms。内容层在会话里只做「贴块矩形 + 重排」，
/// 不跑任何玻璃渲染；静止态该定时器必须卸载（见 `exit_glass_resize_session`），
/// 否则空转唤醒消息循环约吃 9% CPU 并放大内存抖动。
pub(crate) const LIQUID_GLASS_DRAG_INTERVAL_MS: u32 = 8;

/// 会话收尾：事件先到、快照后补的宽限拍数（8ms 档约 640ms）。
const SESSION_ARM_TIMEOUT_TICKS: u8 = 80;
/// 会话收尾：看到 resizing 回落后，还要连续这么多拍才真收尾。
const SESSION_END_CONFIRM_TICKS: u8 = 3;

/// 库缩放会话收尾判定（纯函数，可单测）。
///
/// 返回 `(是否已观察到 resizing, 连续未观察到的拍数, 是否应结束会话)`。
#[must_use]
pub(crate) fn session_next(resizing: bool, seen: bool, idle: u8) -> (bool, u8, bool) {
    if resizing {
        return (true, 0, false);
    }
    let next = idle.saturating_add(1);
    if !seen {
        // 还没见过 resizing：只按宽限期兜底，绝不第一拍就收尾。
        return (false, next, next >= SESSION_ARM_TIMEOUT_TICKS);
    }
    (true, next, next >= SESSION_END_CONFIRM_TICKS)
}

/// 定时器分辨率引用计数（会话期抬到 1ms，会话全部结束还原）。
static TIMER_PERIOD_REF: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// 会话开始：把系统定时器粒度抬到 1ms。不抬的话 `SetTimer(8)` 实际仍按约 15.6ms
/// 触发，内容层永远只有玻璃背景一半帧率（用户报的「标题/图标掉帧、背景流畅」）。
pub(crate) fn raise_timer_resolution() {
    if TIMER_PERIOD_REF.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
        unsafe { super::ffi_imports::timeBeginPeriod(1) };
    }
}

/// 会话结束：还原定时器粒度（引用计数配平，多调无害）。
pub(crate) fn lower_timer_resolution() {
    let prev = TIMER_PERIOD_REF.fetch_update(
        std::sync::atomic::Ordering::SeqCst,
        std::sync::atomic::Ordering::SeqCst,
        |v| if v > 0 { Some(v - 1) } else { None },
    );
    if prev.map_or(false, |v| v > 0) {
        unsafe { super::ffi_imports::timeEndPeriod(1) };
    }
}

/// 引擎模式下 hover 轮询间隔（毫秒）。
///
/// 库 sink 不转发 WM_MOUSEMOVE（posted 移动不会被系统合并，120Hz 洪水会压垮
/// 宿主消息循环）；hover/tooltip 由每框低频轮询光标驱动，会话期间自动升频到跟随档。
pub(crate) const GLASS_HOVER_POLL_INTERVAL_MS: u32 = 100;

/// 返回当前 DPI 下分类框实际应使用的外投影留白（引擎架构下恒为 0）。
///
/// 玻璃底板、外投影与缩放把手反馈全部由 liquid-verre-rs 画布在内容层之下
/// 渲染（投影画在块的矩形之外，不占用内容窗口面积）；引擎不可用回退 D2D
/// 背景时同样不画外投影。保留函数与读取路径，是为了让渲染、布局与命中
/// 测试沿用同一套「内容矩形 = 外框内缩留白」的换算口径。
pub(crate) fn shadow_padding_for_dpi(_dpi: u32) -> i32 {
    0
}

/// 读取窗口当前生效的外投影留白。
///
/// `render_padding` 只在窗口创建、DPI 变更和液态玻璃开关切换时写入，其余
/// 时刻直接复用，避免每帧重新查询全局配置导致窗口外框与渲染留白错拍。
pub(crate) fn window_padding(hwnd: HWND, dpi: u32) -> i32 {
    unsafe { get_data(hwnd) }
        .map(|data| data.render_padding)
        .unwrap_or_else(|| shadow_padding_for_dpi(dpi))
}

/// 将分层窗口外部矩形按指定留白内缩为用户可见的分类框内容矩形。
pub(crate) fn content_rect_with_padding(rect: RECT, padding: i32) -> RECT {
    RECT {
        left: rect.left + padding,
        top: rect.top + padding,
        right: rect.right - padding,
        bottom: rect.bottom - padding,
    }
}

/// 将分层窗口外部矩形转换为用户可见的分类框内容矩形。
pub(crate) fn content_rect_from_window_rect(rect: RECT, dpi: u32) -> RECT {
    content_rect_with_padding(rect, shadow_padding_for_dpi(dpi))
}

/// 获取分类框用户可见内容的屏幕坐标矩形。
pub(crate) fn get_grid_content_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return None;
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    Some(content_rect_with_padding(rect, window_padding(hwnd, dpi)))
}

/// 将窗口客户区坐标按指定留白转换为分类框内容坐标。
pub(crate) fn client_to_content_point_with_padding(padding: i32, x: i32, y: i32) -> (i32, i32) {
    (x - padding, y - padding)
}

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
    let initial_padding = shadow_padding_for_dpi(initial_dpi);
    let initial_w = scale_for_dpi(params.width, initial_dpi) + initial_padding * 2;
    let initial_h = scale_for_dpi(params.height, initial_dpi) + initial_padding * 2;
    let initial_x = params.x - initial_padding;
    let initial_y = params.y - initial_padding;

    // ===== 呈现路径在建窗之前就必须定下来 =====
    //
    // `WS_EX_NOREDIRECTIONBITMAP`（DComp）与 `WS_EX_LAYERED`（UpdateLayeredWindow）
    // 互斥，而扩展样式决定了 DWM 要不要给窗口分配重定向表面 ——
    // **创建之后再改不生效**。所以先把最容易失败的一步（合成交换链）
    // 做掉：成功才用 DComp 样式建窗，失败就直接走分层窗口，
    // 绝不会出现「按 DComp 建了窗、装配却失败 → 窗口彻底看不见」。
    // DirectComposition 目前用环境变量 IKUN_DCOMP=1 显式开启。
    //
    // 为什么默认关：DComp 换掉了整个呈现模型（WS_EX_LAYERED + UpdateLayeredWindow
    // → WS_EX_NOREDIRECTIONBITMAP + 合成交换链）。实测它把单帧从 9.91ms
    // 压到 1.16ms（8.5 倍）、内存再省 52MB，但它**必须肉眼确认过**才能当默认 ——
    // 一旦装配或合成有问题，分类框会整块消失且日志里只有 Present 成功。
    // 当前这台机器的分类框落在 192 DPI 主屏上，而 192 DPI 的尺寸自适应
    // 有个已知遗留 bug（分类框被压成细条），会挡住我对 DComp 的目视验证，
    // 所以先关着，等验证过再翻默认。
    let dcomp_pre = if dcomp_enabled() && !super::glass_engine::is_active() {
        create_dcomp_swapchain(initial_w as u32, initial_h as u32)
    } else {
        None
    };
    let used_dcomp_style = dcomp_pre.is_some();
    let ex_style = if used_dcomp_style {
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW
    } else {
        WS_EX_LAYERED | WS_EX_TOOLWINDOW
    };

    // 1. 创建主窗口
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            CLASS_NAME.as_ptr(),
            std::ptr::null(),
            WS_POPUP | WS_THICKFRAME,
            initial_x,
            initial_y,
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

    // 诊断开关：IKUN_ALLOW_CAPTURE=1 时跳过捕获排除，便于用外部截图工具核对分类框真实画面。
    // 默认不开启，行为与之前完全一致（防止液态玻璃拍到自己形成递归画面）。
    let allow_capture = std::env::var("IKUN_ALLOW_CAPTURE")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if allow_capture {
        log::warn!("[diag] IKUN_ALLOW_CAPTURE=1: 分类框不排除捕获（仅限画面核对使用）");
    } else if let Err(error) =
        unsafe { SetWindowDisplayAffinity(WindowsHwnd(hwnd), WDA_EXCLUDEFROMCAPTURE) }
    {
        log::warn!("分类框捕获排除设置失败，液态玻璃可能出现递归画面: {error}");
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
    let shadow_padding = shadow_padding_for_dpi(dpi);
    let outer_x = scaled_x - shadow_padding;
    let outer_y = scaled_y - shadow_padding;
    let outer_w = scaled_w + shadow_padding * 2;
    let outer_h = scaled_h + shadow_padding * 2;
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
                outer_x,
                outer_y,
                outer_w,
                outer_h,
                SWP_NOZORDER,
            );
        }
        log::info!(
            "DPI 自适应: {} -> 内容位置 ({},{}), 内容尺寸 {}x{}, shadow_padding={}, title_h={}, icon_size={}, cols={}",
            dpi,
            scaled_x,
            scaled_y,
            scaled_w,
            scaled_h,
            shadow_padding,
            title_h,
            icon_size,
            cols
        );
    }

    // ===== DComp 第二阶段：把交换链挂到 DWM 合成树上 =====
    //
    // 交换链是按初始 DPI（96）建的，attach 里会对齐到窗口最终外框尺寸。
    // 装配失败就把窗口改回分层样式，让 UpdateLayeredWindow 兜底 ——
    // 否则窗口既没有重定向表面也没有视觉树，会一直空着。
    // 同 hwnd 若还挂着旧视觉（上次未走 destroy 的残留），先摘掉，
    // 否则新旧两个视觉叠在一起就是重影。
    teardown_dcomp(hwnd);
    let dcomp_state = dcomp_pre.and_then(|(swapchain, context)| {
        attach_dcomp(
            hwnd,
            swapchain,
            context,
            outer_w.max(0) as u32,
            outer_h.max(0) as u32,
        )
    });
    if used_dcomp_style && dcomp_state.is_none() {
        fallback_to_layered(hwnd);
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
        drag_over_item: -1,
        drag_over_insert_col: -1,
        drag_over_insert_row: -1,
        drop_target_ptr: std::ptr::null_mut(),
        scroll_offset: 0,
        renaming_item_index: -1,
        shift_anchor: -1,
        marquee_active: false,
        marquee_start_x: 0,
        marquee_start_y: 0,
        marquee_current_x: 0,
        marquee_current_y: 0,
        marquee_prev_rect: [0; 4],
        marquee_pristine_valid: false,
        marquee_pristine: Vec::new(),
        marquee_last_render_ms: 0,
        marquee_force_full: false,
        multi_drag_items: Vec::new(),
        d2d_target: None,
        icon_bitmap_cache: HashMap::new(),
        // DIB 后端由 render_layered_window 按需创建并长期复用
        dcomp: dcomp_state,
        dib_section: std::ptr::null_mut(),
        dib_bits: std::ptr::null_mut(),
        dib_dc: std::ptr::null_mut(),
        dib_old_bmp: std::ptr::null_mut(),
        dib_w: 0,
        dib_h: 0,
        content_dib: std::ptr::null_mut(),
        content_dc: std::ptr::null_mut(),
        content_old_bmp: std::ptr::null_mut(),
        content_bits: std::ptr::null_mut(),
        content_w: 0,
        content_h: 0,
        content_bitmap: None,
        content_fingerprint: 0,
        content_valid: false,
        render_padding: shadow_padding,
        glass_session_active: false,
        box_drag_active: false,
        submitted_w: 0,
        submitted_h: 0,
        submitted_glass: false,
        content_last_render_ms: 0,
        glass_session_seen_resizing: false,
        glass_session_idle_ticks: 0,
        box_drag_moved: false,
        hover_out_ticks: 0,
    });
    let data_ptr = Box::into_raw(data);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
    }

    // 2.6 登记玻璃底板（liquid-verre-rs 引擎）：内容矩形 = 外框去掉留白。
    //     引擎未激活时 attach 返回 false，该框自动走 D2D 回退背景渲染，
    //     命中测试与原生拖拽/缩放保持不变（优雅降级，不阻塞创建）。
    let glass_content = RECT {
        left: outer_x + shadow_padding,
        top: outer_y + shadow_padding,
        right: outer_x + outer_w - shadow_padding,
        bottom: outer_y + outer_h - shadow_padding,
    };
    let attached = super::glass_engine::attach_box(
        params.config_id,
        hwnd as isize,
        &glass_content,
        dpi,
        params.locked,
    );
    if attached {
        // 块的真正上线在库渲染线程处理 CreateWindow 命令后才完成
        // （`has_box → is_alive` 在此之前为 false）。不等的话，紧接着的首帧
        // 会按「无块」走 D2D 回退背景，用户看到启动时分类框多画一层、
        // 鼠标进入触发重绘后才消失（2026-11-03 验收反馈 bug4）。
        // 上限 ~300ms；超时也不阻塞创建——玻璃 tick 里有残帧纠正兜底。
        for _ in 0..60 {
            if super::glass_engine::has_box(params.config_id) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    // 发布命中几何快照：sink 在渲染线程据此做「标题拖拽接管 / 按钮手型」判定。
    super::glass_engine::update_block_geom(
        params.config_id,
        super::glass_engine::BlockGeom {
            win_w: scaled_w,
            title_h,
            dpi,
            locked: params.locked,
        },
    );

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
    unsafe {
        // 先完成首帧再显示窗口，避免桌面捕获尚未就绪时短暂出现黑/透明窗口。
        //
        // **必须 SW_SHOWNOACTIVATE 并立刻置底**：新建窗口默认落在普通带顶部，
        // 而 `WM_WINDOWPOSCHANGING` 里的强制 HWND_BOTTOM 只在「下一次位置变化」
        // 才生效 —— 于是启动瞬间分类框压在所有应用窗口之上，用户点一下才下沉
        // （2026-11-03 用户复验的层级问题）。另外 SW_SHOW 会抢焦点，桌面组件
        // 不该在启动时把焦点从用户正在用的窗口抢走。
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
    // 内容窗口被压底后必须把画布再压回它之下，维持
    // 「画布 < 全部玻璃块内容窗口 < 其他应用窗口」不变量（见模块头）。
    // 新建块/新窗口显示属于关键时机，直接请库重发一次底部层级（0.5.2 起等值也生效）。
    super::glass_engine::press_canvas_bottom();
    // 周期重绘只在液态玻璃开启时才需要：玻璃关闭时画面是静态位图，
    // 所有变更都会走显式重绘路径。200ms 定时器空转实测吃掉约 9% CPU，
    // 并且是主进程内存持续爬升的放大器。
    sync_grid_refresh_timer(hwnd);

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
/// 把分类框从 DWM 合成树上摘下来。
///
/// # 为什么必须显式摘
///
/// `IDCompositionTarget::SetRoot` 之后视觉就挂在 DWM 的合成树上了；
/// 仅仅 drop 掉 Rust 侧的 COM 引用**不会**把它从树上摘下来。
/// 窗口被销毁又重建（配置重载 / 刷新）时，旧视觉仍留在树上、
/// 新视觉再挂一个 → 同一个位置出现两份画面（重影 + 错位白条），
/// 这正是之前目视看到的缺陷。所以销毁前必须 SetRoot(None) + Commit。
/// DPI 变化后同步视觉缩放（表面像素密度不变，变的是 DIP ↔ 物理像素比例）。
pub(crate) fn update_dcomp_scale(hwnd: HWND) {
    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return;
    };
    let Some(state) = data.dcomp.as_ref() else {
        return;
    };
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };
    let scale = 96.0 / dpi as f32;
    let matrix = windows_numerics::Matrix3x2 {
        M11: scale,
        M12: 0.0,
        M21: 0.0,
        M22: scale,
        M31: 0.0,
        M32: 0.0,
    };
    if let Ok(v1) = windows::core::Interface::cast::<IDCompositionVisual>(&state.visual) {
        let _ = unsafe { v1.SetTransform2(&matrix) };
        if let Some(dcomp) = super::render_devices::dcomp_device() {
            unsafe {
                let _ = dcomp.Commit();
            }
        }
    }
}

pub(crate) fn teardown_dcomp(hwnd: HWND) {
    // 从注册表里取出（不 drop），摘树后再让它自然释放
    // dcomp 存在 GridWindowData（挂在 GWLP_USERDATA）里，从那里取走所有权
    let taken = unsafe { get_data(hwnd) }.and_then(|data| data.dcomp.take());
    let Some(state) = taken else { return };
    teardown_dcomp_state(*state);
}

/// 把一份 DComp 状态从 DWM 合成树上摘下来。
///
/// WM_DESTROY 兑底路径拿到的是 GridWindowData 里的所有权，不经 hwnd 查表，
/// 所以抆树动作单独抽出来两边共用。
pub(crate) fn teardown_dcomp_state(state: DcompState) {
    unsafe {
        let _ = state.target.SetRoot(None::<&IDCompositionVisual>);
        if let Some(dcomp) = super::render_devices::dcomp_device() {
            let _ = dcomp.Commit();
        }
    }
    // state 在此 drop，释放 COM 引用
}

pub fn destroy_grid_window(hwnd: HWND) {
    teardown_dcomp(hwnd);
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
    // 先收集句柄再逐个销毁：DestroyWindow 会同步派发 WM_DESTROY，
    // 不能在持着 GRID_WINDOWS 锁时调任何可能再次锁它的路径。
    let hwnds: Vec<HWND> = match GRID_WINDOWS.lock() {
        Ok(mut grids) => grids.drain(..).map(|grid| grid.hwnd).collect(),
        Err(error) => {
            log::warn!("无法锁定格子窗口列表，跳过销毁: {error}");
            return;
        }
    };
    for hwnd in hwnds {
        // 必须先摘掉 DComp 视觉再销窗：否则旧视觉冻结在 DWM 合成树上，
        // 每个框留一张「白顶条 + 半透明框体 + 旧图标」的残影
        // （见 teardown_dcomp 注释）。之前这里直接 DestroyWindow，是
        // 每个分类框背一个白顶方块的根因。
        teardown_dcomp(hwnd);
        unsafe {
            DestroyWindow(hwnd);
        }
    }
    log::info!("所有格子窗口已销毁");
}

/// 刷新所有格子窗口（液态玻璃参数变更时调用）
///
/// 仅触发重绘，不修改窗口位置或 Z-order。
pub fn refresh_all_grid_windows() {
    if let Ok(grids) = GRID_WINDOWS.lock() {
        for grid in grids.iter() {
            render_layered_window(grid.hwnd);
        }
    }
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

/// 是否启用 DirectComposition 呈现。
///
/// **默认关闭**，用 `IKUN_DCOMP=1` 开启。
///
/// 目前 DComp 路径有一个已确认的视觉缺陷：每个分类框会多画一块
/// 错位的半透明面板 + 一条白色顶条（玻璃与内容层落点不一致 / 重复合成）。
/// 在修好并目视确认前不能当默认，否则用户看到的就是坏画面。
/// 性能收益是真实的（单帧 9.91ms → 1.9-2.6ms），修好视觉后翻回默认。
///
/// DComp 的实测收益非常大（单帧 9.91ms → 1.16ms，8.5 倍；内存再省 52MB），
/// 也是达到「5 框 × 25fps」预算（≤8ms/帧）的唯一路径。
///
/// 但它换掉了整个呈现模型（`WS_EX_LAYERED` + `UpdateLayeredWindow`
/// → `WS_EX_NOREDIRECTIONBITMAP` + 合成交换链），而当前这台机器的
/// 分类框落在 192 DPI 主屏上，那里有一个**已知遗留的尺寸自适应 bug**
/// （外框比 2×shadow_padding 还小，内容尺寸算成负数，分类框整块不画），
/// 会挡住我对 DComp 的目视验证 —— 我无法确认交换链里的画面真的上了屏。
/// 之所以现在敢设默认：这个开关保护的风险（装配/合成失败导致分类框消失）
/// 已经有 `fallback_to_layered` 兜底，且 `render_via_dcomp` 里带了
/// DWM 合成自证日志；而它挡住的收益是实打实的 8.5 倍帧率 + 52MB 内存。
/// 万一某台机器上确实有问题，`IKUN_DCOMP=0` 立即回退，无需重编译。
fn dcomp_enabled() -> bool {
    // 默认关：2026-09-14 实测这台 192 DPI + Intel Arc 机器上
    // DComp 呈现有严重缺陷（表面按 DIP 解释被放大 2 倍且
    // 视觉不被窗口裁剪，桌面到处溢出「白顶条 + 半透明
    // 方块」；加视觉缩放后画面错位），而 ULW 路径渲染完全
    // 正确且帧耗满足 25fps 预算。DComp 作为实验项用
    // IKUN_DCOMP=1 显式开启，修复验证后再翻默认。
    matches!(
        std::env::var("IKUN_DCOMP").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

/// 为一个分类框装配 DirectComposition 呈现栈。
///
/// 任何一步失败都返回 `None` 并记日志。
///
/// # 为什么调用方必须在 CreateWindowExW **之前**就决定走不走 DComp
///
/// `WS_EX_NOREDIRECTIONBITMAP` 与 `WS_EX_LAYERED` 互斥，而扩展样式
/// 决定了 DWM 要不要给这个窗口分配重定向表面 —— 创建之后再改
/// 不生效。所以必须先用 `dcomp_available()` 问一次，再定窗口样式；
/// 装配失败就只能整个窗口重建，代价太大，不如一开始就问清楚。
/// 第一阶段：**在建窗口之前**创建合成交换链。
///
/// # 为什么必须分两阶段
///
/// `WS_EX_NOREDIRECTIONBITMAP` 与 `WS_EX_LAYERED` 互斥，而扩展样式决定了
/// DWM 要不要给窗口分配重定向表面 —— **创建之后再改不生效**。
/// 若先按 DComp 建窗、装配却失败，窗口就会既没有重定向表面也没有视觉树，
/// 表现为一个彻底看不见的分类框，而且没有任何报错。
///
/// `CreateSwapChainForComposition` 恰好**不需要 HWND**，所以可以把最容易
/// 失败的一步提到建窗之前：成功才用 DComp 样式建窗，失败直接走分层窗口。
/// 剩下的 `CreateTargetForHwnd` 只对无效 HWND 失败，风险低得多，
/// 且仍有 `fallback_to_layered` 兜底。
pub(crate) fn create_dcomp_swapchain(
    width: u32,
    height: u32,
) -> Option<(IDXGISwapChain1, ID2D1DeviceContext)> {
    if width == 0 || height == 0 {
        log::warn!("[DComp] 交换链尺寸非法 {width}x{height}，退回分层窗口呈现");
        return None;
    }
    let gpu = super::render_devices::shared_gpu()?;
    let _dcomp = super::render_devices::dcomp_device()?;
    let context = super::render_devices::new_d2d_context()?;

    unsafe {
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            // flip 模型最少 2 张后缓冲；再多只会白占显存
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            // 逐像素 alpha 的关键：与 `UpdateLayeredWindow` 的 `AC_SRC_ALPHA`
            // 是同一套预乘 source-over 语义。玻璃着色器输出的本来就是预乘
            // alpha，128px 外投影的渐变与圆角全靠它，绝不能写成 STRAIGHT
            // —— 那会让半透明区域整体发暗、圆角出现黑边。
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            // 注意：DESC1 的 Flags 在 windows 0.62 里是裸 u32，
            // 而 ResizeBuffers 的 flags 参数才是强类型 DXGI_SWAP_CHAIN_FLAG。
            // 两处刚好相反，写反了编译不过。
            Flags: 0,
        };
        match gpu
            .factory2
            .CreateSwapChainForComposition(&gpu.device, &desc, None)
        {
            Ok(swapchain) => Some((swapchain, context)),
            Err(error) => {
                log::error!("[DComp] 创建合成交换链失败 {width}x{height}: {error:?}");
                None
            }
        }
    }
}

/// 第二阶段：窗口已存在，把交换链挂到 DWM 合成树上。
///
/// 失败时调用方**必须**调 `fallback_to_layered`，否则窗口会一直空着。
pub(crate) fn attach_dcomp(
    hwnd: HWND,
    swapchain: IDXGISwapChain1,
    context: ID2D1DeviceContext,
    width: u32,
    height: u32,
) -> Option<Box<DcompState>> {
    let dcomp = super::render_devices::dcomp_device()?;
    unsafe {
        // 交换链是按初始 DPI 建的，这里对齐到窗口最终外框尺寸
        if width > 0 && height > 0 {
            if let Err(error) = swapchain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            ) {
                log::error!("[DComp] 交换链对齐到 {width}x{height} 失败: {error:?}");
                return None;
            }
        }

        let visual: IDCompositionVisual2 = match dcomp.CreateVisual() {
            Ok(visual) => visual,
            Err(error) => {
                log::error!("[DComp] 创建视觉节点失败: {error:?}");
                return None;
            }
        };
        if let Err(error) = visual.SetContent(&swapchain) {
            log::error!("[DComp] 视觉节点绑定交换链失败: {error:?}");
            return None;
        }
        // 合成树以 DIP 解释视觉内容，而交换链是物理像素建的。
        // 不缩放时 192 DPI 下表面会被放大 2 倍铺出窗口外
        // （DComp 视觉不被窗口矩形裁剪），表现为桌面上到处是
        // 「白顶条 + 半透明大方块」。缩到 96/dpi 后表面像素
        // 与窗口物理像素 1:1。
        let dpi = GetDpiForWindow(hwnd);
        let dpi = if dpi > 0 { dpi } else { 96 };
        // topmost = false：分类框压在桌面底部，绝不能盖住别的应用窗口
        let target: IDCompositionTarget = match dcomp.CreateTargetForHwnd(WindowsHwnd(hwnd), false)
        {
            Ok(target) => target,
            Err(error) => {
                log::error!("[DComp] 创建 HWND 合成目标失败: {error:?}");
                return None;
            }
        };
        // SetRoot 要的是 v1 的 IDCompositionVisual，从 v2 降转一次
        let root: IDCompositionVisual = match windows::core::Interface::cast(&visual) {
            Ok(root) => root,
            Err(error) => {
                log::error!("[DComp] 视觉节点降级到 v1 失败: {error:?}");
                return None;
            }
        };
        let scale = 96.0 / dpi as f32;
        let scale_matrix = windows_numerics::Matrix3x2 {
            M11: scale,
            M12: 0.0,
            M21: 0.0,
            M22: scale,
            M31: 0.0,
            M32: 0.0,
        };
        if let Err(error) = root.SetTransform2(&scale_matrix) {
            log::error!("[DComp] 视觉缩放设置失败 dpi={dpi}: {error:?}");
            return None;
        }
        if let Err(error) = target.SetRoot(&root) {
            log::error!("[DComp] 设置根视觉失败: {error:?}");
            return None;
        }
        if let Err(error) = dcomp.Commit() {
            log::error!("[DComp] 提交合成树失败: {error:?}");
            // SetRoot 已成功而 Commit 失败：视觉已在树上，直接 drop 会
            // 把它冻在合成树里变成残影。先摘树再补一次提交。
            let _ = target.SetRoot(None::<&IDCompositionVisual>);
            let _ = dcomp.Commit();
            return None;
        }

        log::info!("[DComp] 分类框已切换到 DirectComposition 呈现 {width}x{height}");
        Some(Box::new(DcompState {
            target,
            visual,
            swapchain,
            context,
            backbuffer: None,
            content: None,
            content_w: 0,
            content_h: 0,
            width,
            height,
            presented_frames: 0,
        }))
    }
}

/// DComp 装配失败时把窗口改回分层样式，让 `UpdateLayeredWindow` 兜底。
///
/// `WS_EX_LAYERED` 可以在创建后补上（这点与 `WS_EX_NOREDIRECTIONBITMAP`
/// 相反），但补完必须带 `SWP_FRAMECHANGED` 触发 DWM 重建窗口表面，
/// 诊断：IKUN_DEBUG_DUMP_FRAME=1 时把 DComp 后缓冲原样落盘。
///
/// 截图看到的是 DWM 合成后的画面，无法区分「渲染阶段就画错」
/// 还是「合成阶段摆错」。这里直接读回后缓冲像素：
/// 颜色图把预乘 alpha 还原后叠在品红底上（透明区 = 品红），
/// alpha 图单独一张灰度图。全进程最多写 5 帧。
/// 诊断：把 ULW 路径即将提交的主 DIB（预乘 BGRA）落盘。
///
/// 不走截图，因此不需要关掉捕获排除，也不会引入
/// 「玻璃拍到自己」的反馈环路。颜色图透明区域显示为品红。
pub(crate) fn dump_dib_frame(bits: *const Pixel32, w: i32, h: i32, hwnd: HWND) {
    use std::io::Write;
    static DUMPED_DIB: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let idx = DUMPED_DIB.load(std::sync::atomic::Ordering::Relaxed);
    // IKUN_DEBUG_DUMP_FRAME 可带数值：跳过前 N 帧再连续落 5 帧，
    // 用于抓稳态画面（启动竞态期的帧代表不了产品状态）。
    static SKIP: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    let skip = *SKIP.get_or_init(|| {
        std::env::var("IKUN_DEBUG_DUMP_FRAME")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0)
    });
    if bits.is_null() || w <= 0 || h <= 0 {
        return;
    }
    if idx < skip {
        DUMPED_DIB.store(idx + 1, std::sync::atomic::Ordering::Relaxed);
        return;
    }
    if idx >= skip + 5 {
        return;
    }
    let w = w as usize;
    let h = h as usize;
    let px = unsafe { std::slice::from_raw_parts(bits as *const u8, w * h * 4) }.to_vec();
    DUMPED_DIB.store(idx + 1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join("ikun_mem2");
    let _ = std::fs::create_dir_all(&dir);
    let tag = format!("dib{}_{}x{}_{:x}", idx, w, h, hwnd as usize);
    let transforms: [(&str, fn([u8; 4]) -> [u8; 4]); 2] = [
        ("color", |p: [u8; 4]| -> [u8; 4] {
            let a = p[3] as u32;
            if a == 0 {
                return [255, 0, 255, 255];
            }
            let un = |c: u8| ((c as u32 * 255 / a).min(255)) as u8;
            [un(p[0]), un(p[1]), un(p[2]), 255]
        }),
        ("alpha", |p: [u8; 4]| -> [u8; 4] {
            let a = p[3];
            [a, a, a, 255]
        }),
    ];
    for (suffix, transform) in transforms {
        let mut data = Vec::with_capacity(54 + w * h * 4);
        let row_bytes = (w * 4) as i32;
        data.extend_from_slice(b"BM");
        data.extend_from_slice(&((54 + w * h * 4) as u32).to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&54u32.to_le_bytes());
        data.extend_from_slice(&40u32.to_le_bytes());
        data.extend_from_slice(&(w as i32).to_le_bytes());
        data.extend_from_slice(&(h as i32).to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&(row_bytes * h as i32).to_le_bytes());
        data.extend_from_slice(&2835u32.to_le_bytes());
        data.extend_from_slice(&2835u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        let mut rows = Vec::with_capacity(w * h * 4);
        for row in (0..h).rev() {
            for x in 0..w {
                let p = [
                    px[row * w * 4 + x * 4],
                    px[row * w * 4 + x * 4 + 1],
                    px[row * w * 4 + x * 4 + 2],
                    px[row * w * 4 + x * 4 + 3],
                ];
                rows.extend_from_slice(&transform(p));
            }
        }
        data.extend_from_slice(&rows);
        let path = dir.join(format!("{tag}_{suffix}.bmp"));
        if let Ok(mut f) = std::fs::File::create(&path) {
            let _ = f.write_all(&data);
            log::warn!("[diag] DIB 帧已落盘: {}", path.display());
        }
    }
}

pub(crate) fn dump_dcomp_backbuffer(state: &mut DcompState) {
    use std::io::Write;
    use windows::Win32::Graphics::Direct3D11::ID3D11Device;
    use windows::Win32::Graphics::Direct3D11::{
        ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_STAGING,
    };

    static DUMPED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let idx = DUMPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    static SKIP: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    let skip = *SKIP.get_or_init(|| {
        std::env::var("IKUN_DEBUG_DUMP_FRAME")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0)
    });
    if idx < skip || idx >= skip + 5 {
        return;
    }
    let Ok(device) = (unsafe { state.swapchain.GetDevice::<ID3D11Device>() }) else {
        return;
    };
    let Ok(src) = (unsafe { state.swapchain.GetBuffer::<ID3D11Texture2D>(0) }) else {
        return;
    };
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { src.GetDesc(&mut desc) };
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: desc.Width,
        Height: desc.Height,
        MipLevels: 1,
        ArraySize: 1,
        Format: desc.Format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging_slot: Option<ID3D11Texture2D> = None;
    if unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging_slot)) }.is_err() {
        return;
    }
    let Some(staging) = staging_slot else {
        return;
    };
    let Ok(context) = (unsafe { device.GetImmediateContext() }) else {
        return;
    };
    unsafe {
        context.CopyResource(&staging, &src);
        let mut mapped = std::mem::MaybeUninit::<
            windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE,
        >::uninit();
        if context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(mapped.as_mut_ptr()))
            .is_err()
        {
            return;
        }
        let mapped = mapped.assume_init();
        let w = desc.Width as usize;
        let h = desc.Height as usize;
        let mut px = vec![0u8; w * h * 4];
        for row in 0..h {
            let src_row = std::slice::from_raw_parts(
                (mapped.pData as usize + row * mapped.RowPitch as usize) as *const u8,
                w * 4,
            );
            px[row * w * 4..(row + 1) * w * 4].copy_from_slice(src_row);
        }
        context.Unmap(&staging, 0);

        let dir = std::env::temp_dir().join("ikun_mem2");
        let _ = std::fs::create_dir_all(&dir);
        let tag = format!("frame{}_{}x{}", idx, w, h);
        let transforms: [(&str, fn([u8; 4]) -> [u8; 4]); 2] = [
            ("color", |p: [u8; 4]| -> [u8; 4] {
                let a = p[3] as u32;
                if a == 0 {
                    return [255, 0, 255, 255];
                }
                let un = |c: u8| ((c as u32 * 255 / a).min(255)) as u8;
                [un(p[0]), un(p[1]), un(p[2]), 255]
            }),
            ("alpha", |p: [u8; 4]| -> [u8; 4] {
                let a = p[3];
                [a, a, a, 255]
            }),
        ];
        for (suffix, transform) in transforms {
            let mut data = Vec::with_capacity(54 + w * h * 4);
            let row_bytes = (w * 4) as i32;
            data.extend_from_slice(b"BM");
            data.extend_from_slice(&((54 + w * h * 4) as u32).to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&54u32.to_le_bytes());
            data.extend_from_slice(&40u32.to_le_bytes());
            data.extend_from_slice(&(w as i32).to_le_bytes());
            data.extend_from_slice(&(h as i32).to_le_bytes());
            data.extend_from_slice(&1u16.to_le_bytes());
            data.extend_from_slice(&32u16.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&(row_bytes * h as i32).to_le_bytes());
            data.extend_from_slice(&2835u32.to_le_bytes());
            data.extend_from_slice(&2835u32.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            let mut rows = Vec::with_capacity(w * h * 4);
            for row in (0..h).rev() {
                for x in 0..w {
                    let p = [
                        px[row * w * 4 + x * 4],
                        px[row * w * 4 + x * 4 + 1],
                        px[row * w * 4 + x * 4 + 2],
                        px[row * w * 4 + x * 4 + 3],
                    ];
                    rows.extend_from_slice(&transform(p));
                }
            }
            data.extend_from_slice(&rows);
            let path = dir.join(format!("{tag}_{suffix}.bmp"));
            if let Ok(mut f) = std::fs::File::create(&path) {
                let _ = f.write_all(&data);
                log::warn!("[diag] 帧已落盘: {}", path.display());
            }
        }
    }
}

/// 否则样式改了表面没变，窗口依旧全透明。
pub(crate) fn fallback_to_layered(hwnd: HWND) {
    unsafe {
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (WS_EX_LAYERED | WS_EX_TOOLWINDOW) as isize,
        );
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    log::error!("[DComp] 装配失败，分类框已退回 WS_EX_LAYERED + UpdateLayeredWindow");
}

/// 交换链改尺寸。
///
/// `ResizeBuffers` 之前**必须**先释放所有对后缓冲的引用，
/// 否则返回 `DXGI_ERROR_INVALID_CALL` 且交换链进入失效状态 ——
/// 之后每次 `Present` 都失败，画面永久冻住。所以这里先把
/// `backbuffer` 丢掉再改。
pub(crate) fn resize_dcomp(state: &mut DcompState, width: u32, height: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    if state.width == width && state.height == height {
        return true;
    }
    state.backbuffer = None;
    unsafe {
        if let Err(error) = state.swapchain.ResizeBuffers(
            0,
            width,
            height,
            DXGI_FORMAT_UNKNOWN,
            DXGI_SWAP_CHAIN_FLAG(0),
        ) {
            log::error!("[DComp] ResizeBuffers 到 {width}x{height} 失败: {error:?}");
            return false;
        }
    }
    state.width = width;
    state.height = height;
    true
}

/// 取（必要时重建）当前后缓冲的 D2D 目标位图。
///
/// `ResizeBuffers` 之后旧的后缓冲引用已失效，必须重新取一次，
/// 否则会一直画进已经不存在的表面上且不报错。
pub(crate) fn ensure_dcomp_backbuffer(state: &mut DcompState) -> Option<ID2D1Bitmap1> {
    if let Some(existing) = state.backbuffer.clone() {
        return Some(existing);
    }
    unsafe {
        let buffer: IDXGISurface = match state.swapchain.GetBuffer(0) {
            Ok(buffer) => buffer,
            Err(error) => {
                log::error!("[DComp] 取后缓冲失败: {error:?}");
                return None;
            }
        };
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            // TARGET：允许被 `SetTarget` 当渲染目标。
            // CANNOT_DRAW：不会再从它采样，省掉驱动为采样建的 SRV。
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        match state
            .context
            .CreateBitmapFromDxgiSurface(&buffer, Some(&props))
        {
            Ok(bitmap) => {
                state.backbuffer = Some(bitmap.clone());
                Some(bitmap)
            }
            Err(error) => {
                log::error!("[DComp] 后缓冲包装成 D2D 目标失败: {error:?}");
                None
            }
        }
    }
}

/// 按引擎状态装配/卸载定时器。
///
/// 引擎激活：装配 hover 低频轮询档（sink 不转发 WM_MOUSEMOVE，悬停高亮/
/// tooltip 由轮询驱动）；引擎不可用：卸载（回退路径是静态位图 + 真实鼠标，
/// 所有变化走显式重绘）。
pub(crate) fn sync_grid_refresh_timer(hwnd: HWND) {
    switch_glass_timer(hwnd, GLASS_HOVER_POLL_INTERVAL_MS);
}

/// 进入库缩放会话：升频到 16ms 跟随档。
///
/// 由 `WM_USER_GLASS_RESIZE`（引擎事件回调转发）驱动；库把块矩形改掉时
/// 内容窗口要实时贴上。
pub(crate) fn enter_glass_resize_session(hwnd: HWND) {
    switch_glass_timer(hwnd, LIQUID_GLASS_DRAG_INTERVAL_MS);
}

/// 退出库缩放会话：降回 hover 轮询低频档。
///
/// 必须与 `enter_glass_resize_session` 严格配对：漏掉这一步分类框会
/// 永久以 60Hz 唤醒消息循环，CPU 与内存高水位下不来且不报任何错。
pub(crate) fn exit_glass_resize_session(hwnd: HWND) {
    switch_glass_timer(hwnd, GLASS_HOVER_POLL_INTERVAL_MS);
}

/// 分类框侧的内存占用快照（只读诊断，供 `mem_probe` 输出）。
#[derive(Clone, Copy, Default)]
pub(crate) struct GridMemoryStats {
    /// 当前存活的分类框窗口数
    pub windows: usize,
    /// 所有分类框的条目总数
    pub items: usize,
    /// DIB 渲染后端合计字节（每个窗口一张 outer_w × outer_h 的 32bpp DIB）
    pub dib_bytes: u64,
    /// 图标位图缓存条目数
    pub icon_bitmap_entries: usize,
    /// 图标位图缓存合计字节（按位图源尺寸计，往往远大于显示尺寸）
    pub icon_bitmap_bytes: u64,
}

/// 汇总所有分类框持有的渲染缓冲，用于内存归因。
pub(crate) fn memory_stats() -> GridMemoryStats {
    let mut stats = GridMemoryStats::default();
    // 先只抄句柄，避免在持锁期间调 get_data（它内部还会读窗口属性）
    let hwnds: Vec<HWND> = match GRID_WINDOWS.lock() {
        Ok(grids) => grids.iter().map(|grid| grid.hwnd).collect(),
        Err(_) => return stats,
    };
    for hwnd in hwnds {
        let Some(data) = (unsafe { get_data(hwnd) }) else {
            continue;
        };
        stats.windows += 1;
        stats.items += data.items.len();
        if data.dib_w > 0 && data.dib_h > 0 {
            stats.dib_bytes += data.dib_w as u64 * data.dib_h as u64 * 4;
        }
        stats.icon_bitmap_entries += data.icon_bitmap_cache.len();
        for bitmap in data.icon_bitmap_cache.values() {
            let size = unsafe { bitmap.GetPixelSize() };
            stats.icon_bitmap_bytes += size.width as u64 * size.height as u64 * 4;
        }
    }
    stats
}

pub(crate) fn on_grid_refresh_tick(hwnd: HWND) {
    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return;
    };
    // 残帧纠正：块已上线但代理窗还挂着「无块」时期的回退背景
    // （submitted_glass=false）→ 补一次重绘切回全透明帧，画面交还给库。
    // 兜住启动等待超时、引擎重建后重挂载等所有迟到场景（bug4 双层的根治）。
    let stale_fallback = super::glass_engine::has_box(data.config_id)
        && !data.submitted_glass
        && !data.glass_session_active
        && !data.box_drag_active;
    drop(data);
    if stale_fallback {
        super::d2d_render::render_layered_window(hwnd);
    }
    follow_glass_session(hwnd);
    // 空闲（无库缩放会话、无移动会话）时兼作 hover 轮询。
    poll_glass_hover(hwnd);
}

/// 当前光标屏幕坐标。
fn glass_cursor_pos() -> Option<(i32, i32)> {
    let mut pt = windows::Win32::Foundation::POINT { x: 0, y: 0 };
    (unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) })
        .is_ok()
        .then_some((pt.x, pt.y))
}

/// 切换引擎定时器档位（hover 低频 ↔ 会话高频）。
fn switch_glass_timer(hwnd: HWND, interval_ms: u32) {
    unsafe { KillTimer(hwnd, TIMER_ID_LIQUID_GLASS) };
    if !super::glass_engine::is_active() {
        return;
    }
    let _ = unsafe {
        SetTimer(
            hwnd,
            TIMER_ID_LIQUID_GLASS,
            interval_ms,
            std::ptr::null_mut(),
        )
    };
}

/// 接管式标题拖拽：结束（释放捕获、对齐库权威矩形、恢复低频档、保存布局）。
pub(crate) fn end_box_drag(hwnd: HWND) {
    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return;
    };
    if !data.box_drag_active {
        return;
    }
    data.box_drag_active = false;
    log::info!("[glass-dbg] end_box_drag hwnd={hwnd:?}");
    // 库对块有「中心不出所在屏」钳制：拖到屏幕边缘时窗口可能领先块矩形。
    // 收尾回读权威块矩形对齐窗口，保证内容层与玻璃最终严丝合缝。
    if let Some(content) = super::glass_engine::block_content_rect(data.config_id) {
        let padding = data.render_padding;
        let want = RECT {
            left: content.left - padding,
            top: content.top - padding,
            right: content.right + padding,
            bottom: content.bottom + padding,
        };
        let mut outer = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let have = unsafe { GetWindowRect(hwnd, &mut outer) } != 0;
        if have && (outer.left != want.left || outer.top != want.top) {
            unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    want.left,
                    want.top,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
    switch_glass_timer(hwnd, GLASS_HOVER_POLL_INTERVAL_MS);
    // 没产生位移（标题单击 / 合成双击）就不碰参考线，也不写配置文件：
    // 旧实现每次按压都 save_config 一遍，双击/连点会刷出成串磁盘写。
    if data.box_drag_moved {
        data.box_drag_moved = false;
        super::alignment_guide::on_exit_size_move(hwnd);
        super::drag_handler::save_config();
    } else {
        data.box_drag_moved = false;
    }
}

/// hover 轮询：光标在块内 → 以 client 坐标 SendMessage(WM_MOUSEMOVE) 复用既有
/// hover/tooltip 逻辑；离开且存在 hover 态 → 补发一次 WM_MOUSELEAVE。
fn poll_glass_hover(hwnd: HWND) {
    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return;
    };
    if !super::glass_engine::has_box(data.config_id) {
        return;
    }
    if data.box_drag_active {
        // 看门狗：拖拽收尾依赖 sink 序列的 Up。异常路径（序列里丢了 Up、画布
        // 输入被抢）会让代理窗一直跟着跑，这里按左键物理状态兜底。
        let lbutton_down = unsafe { GetAsyncKeyState(VK_LBUTTON) } & 0x8000u16 as i16 != 0;
        if !lbutton_down {
            log::warn!("[glass-dbg] 接管式移动未收到 UP（左键已抬起）→ 强制收尾");
            end_box_drag(hwnd);
            return;
        }
    }
    // 拖拽进行中不做 hover 轮询（代理窗正在被搬，高亮跟随没有意义）
    // 框选同理：真实/转发的 WM_MOUSEMOVE 已经是坐标源，这里再 Post 一发合成 move
    // 只会让选框在同一拍被写两次（多一次无谓重绘，还与定时器互相覆盖）。
    if data.menu.open || data.box_drag_active || data.marquee_active {
        return;
    }
    let Some((cx, cy)) = glass_cursor_pos() else {
        return;
    };
    let Some(wr) = get_grid_content_rect(hwnd) else {
        return;
    };
    let inside = cx >= wr.left && cx < wr.right && cy >= wr.top && cy < wr.bottom;
    if inside {
        data.hover_out_ticks = 0;
        let local_x = cx - wr.left;
        let local_y = cy - wr.top;
        let lparam = (((local_y << 16) | (local_x & 0xFFFF)) as i32) as isize;
        unsafe { PostMessageW(hwnd, WM_MOUSEMOVE, 0, lparam) };
    } else {
        data.hover_out_ticks = data.hover_out_ticks.saturating_add(1);
        if data.hover_out_ticks == 2
            && (data.hovered_item >= 0 || data.lock_hovered || data.more_hovered)
        {
            unsafe { PostMessageW(hwnd, WM_MOUSELEAVE, 0, 0) };
        }
    }
}

/// 库缩放会话的高频跟随 tick：玻璃块矩形是真值，内容窗口保持相对留白跟随。
///
/// 会话的开始由引擎事件回调（`WM_USER_GLASS_RESIZE`）触发；本函数只做
/// 两件事：①把内容窗口贴到块的当前矩形（物理像素换算，误差 ≤1 帧）；
/// ②确认会话结束（快照 `resizing` 回落后收尾：卸载定时器、恢复参考线、
/// 持久化布局）。库画布若被独占全屏让路等原因丢块，同样走收尾路径。
fn follow_glass_session(hwnd: HWND) {
    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return;
    };
    let box_id = data.config_id;
    // 催库出帧：块矩形来自快照。0.5.2 起库的契约是「块几何变化 => 同帧快照可见」，
    // 这里只唤醒渲染，不再需要 set_default_resize 那种催发布 hack。
    super::glass_engine::request_render();
    // 空闲态**绝不搬窗口**：块矩形与窗口矩形之间存在 1px 级舍入差是常态
    // （逻辑↔物理往返、库中心钳制），空闲跟随会与 WM_WINDOWPOSCHANGED 的
    // 反向 push 形成每 tick 拉锯 —— 表现为 hover 闪烁、点击抖动（实测根因）。
    // 跟随只发生在库缩放会话期间（ResizeStarted 事件置位后）。
    if !data.glass_session_active {
        return;
    }
    let Some(content) = super::glass_engine::block_content_rect(box_id) else {
        end_glass_resize_session(hwnd);
        return;
    };
    let padding = data.render_padding;
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let have_rect = unsafe { GetWindowRect(hwnd, &mut outer) } != 0;
    let want = RECT {
        left: content.left - padding,
        top: content.top - padding,
        right: content.right + padding,
        bottom: content.bottom + padding,
    };
    if have_rect
        && (outer.left != want.left
            || outer.top != want.top
            || outer.right - outer.left != want.right - want.left
            || outer.bottom - outer.top != want.bottom - want.top)
    {
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                want.left,
                want.top,
                want.right - want.left,
                want.bottom - want.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    super::glass_engine::note_session_rect([
        content.left,
        content.top,
        content.right,
        content.bottom,
    ]);
    super::glass_engine::maybe_log_session_stats();
    // 移动会话只有 moving、缩放会话只有 resizing：两者都要当作「会话还活着」，
    // 否则移动会话会被宽限期提前掐断。
    let active =
        super::glass_engine::block_resizing(box_id) || super::glass_engine::block_moving(box_id);
    let (seen, idle, should_end) = session_next(
        active,
        data.glass_session_seen_resizing,
        data.glass_session_idle_ticks,
    );
    data.glass_session_seen_resizing = seen;
    data.glass_session_idle_ticks = idle;
    if should_end {
        log::debug!("库会话收尾：moving/resizing 连续 {idle} 拍为 false（seen={seen}）");
        end_glass_resize_session(hwnd);
    }
}

/// 进入库缩放会话（由 `WM_USER_GLASS_RESIZE` wParam=1 触发）。
pub(crate) fn begin_glass_resize_session(hwnd: HWND) {
    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return;
    };
    if data.glass_session_active {
        return;
    }
    data.glass_session_active = true;
    data.glass_session_seen_resizing = false;
    data.glass_session_idle_ticks = 0;
    raise_timer_resolution();
    enter_glass_resize_session(hwnd);
    // 0.5.2 起 border 由库在**逐块 pass 内**发射（紧跟自己那块的玻璃、早于任何
    // 更高块，库 core::draw::border_emit_plan）：下层框的反馈白边会被上层框的玻璃
    // 正确遮挡，不再画到上层框身上，因此这里不再抬层 —— 0.5.1 时代的 raise_box
    // 绕法实测修不了该缺陷（border 是无条件盖在全部玻璃之上的独立 pass，抬层只
    // 改块序、改不了那条 pass）。「操作中的框整体在最前」仍由内容窗口激活链路
    // （wnd_proc 的 Z 序不变量 -> glass_engine::raise_box）负责。
    // 0.5.3 再补一刀：命中层序与绘制层序同判据（库 hit_resize 的 topmost_body_z_at 遮挡过滤），
    // 被上层框机身压住的那段边缘**根本不会再亮**，所以这条白边争议从「画在哪」变成「该不该亮」。
    super::alignment_guide::on_enter_size_move(hwnd);
    log::debug!("分类框进入库会话（移动/缩放）");
}

/// 退出库缩放会话（tick 确认快照 `resizing` 回落，或 wParam=0 事件兜底）。
pub(crate) fn end_glass_resize_session(hwnd: HWND) {
    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return;
    };
    if !data.glass_session_active {
        return;
    }
    data.glass_session_active = false;
    data.glass_session_seen_resizing = false;
    data.glass_session_idle_ticks = 0;
    data.content_last_render_ms = 0;
    lower_timer_resolution();
    exit_glass_resize_session(hwnd);
    // 最后做一次几何同步，避免内容窗口停在落后玻璃块不足一 tick 的位置。
    if let Some(content) = super::glass_engine::block_content_rect(data.config_id) {
        let padding = data.render_padding;
        let want = RECT {
            left: content.left - padding,
            top: content.top - padding,
            right: content.right + padding,
            bottom: content.bottom + padding,
        };
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                want.left,
                want.top,
                want.right - want.left,
                want.bottom - want.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    super::alignment_guide::on_exit_size_move(hwnd);
    super::drag_handler::save_config();
    log::debug!("分类框退出库缩放会话，布局已持久化");
}

/// 向所有分类框广播一条自定义消息（异步，不等待对方线程）。
///
/// 配置写入发生在 Tauri 命令线程，而分类框窗口属于平台渲染线程。
/// 直接跨线程 SetWindowPos / render_layered_window 会同步等待对方线程，
/// 曾经导致 WebView2 事件循环被卡住、窗口底部出现异常空白，因此统一走
/// PostMessageW，由窗口自己的消息循环执行。
fn post_to_all_grid_windows(message: u32) {
    let hwnds: Vec<HWND> = match GRID_WINDOWS.lock() {
        Ok(grids) => grids.iter().map(|grid| grid.hwnd).collect(),
        Err(error) => {
            log::warn!("无法锁定分类框列表，跳过广播: {error}");
            return;
        }
    };
    for hwnd in hwnds {
        unsafe { PostMessageW(hwnd, message, 0, 0) };
    }
}

/// 请求所有分类框异步重绘一帧（配置里只改了颜色/圆角等样式时使用）。
pub fn request_all_grid_windows_redraw() {
    post_to_all_grid_windows(WM_USER_GRID_REDRAW);
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

        // 读取用户可见内容矩形（排除透明外投影留白）。
        let Some(wr) = get_grid_content_rect(grid.hwnd) else {
            continue;
        };

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

    /// 引擎架构下分类框内容窗口不再预留任何外投影留白（恒为 0）。
    ///
    /// 外投影与玻璃底板由 liquid-verre-rs 画布渲染在块矩形之外，
    /// 内容窗口面积与留白彻底解耦；回退 D2D 背景同样不画投影。
    #[test]
    fn shadow_padding_is_always_zero_in_engine_architecture() {
        for dpi in [96u32, 120, 144, 192] {
            assert_eq!(
                shadow_padding_for_dpi(dpi),
                0,
                "引擎架构下内容窗口留白恒为 0"
            );
        }
    }

    /// 内容矩形换算必须自洽：外框按留白内缩，客户区原点映射到内容原点。
    #[test]
    fn content_rect_conversion_uses_given_padding() {
        for padding in [0i32, 128, 192] {
            let outer = RECT {
                left: 300,
                top: 400,
                right: 1300,
                bottom: 1200,
            };
            // RECT 没有 Copy，先取出四条边再把 outer 移动进函数
            let (ol, ot, orr, ob) = (outer.left, outer.top, outer.right, outer.bottom);
            let content = content_rect_with_padding(outer, padding);
            assert_eq!(content.left, ol + padding);
            assert_eq!(content.top, ot + padding);
            assert_eq!(content.right, orr - padding);
            assert_eq!(content.bottom, ob - padding);
            assert_eq!(content.right - content.left, (orr - ol) - padding * 2);
            assert_eq!(content.bottom - content.top, (ob - ot) - padding * 2);
            assert_eq!(
                client_to_content_point_with_padding(padding, padding, padding),
                (0, 0)
            );
        }
    }

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
    fn glass_session_does_not_end_on_first_stale_snapshot() {
        // 事件先到、快照后补：前若干拍 resizing=false 绝不能收尾。旧实现第一拍
        // 就结束会话，于是内容不跟尺寸、窗口旧矩形把块推回去（用户看到的
        // 「拖边缘内容不动，过一会儿自己复原大小」）。
        let mut seen = false;
        let mut idle = 0u8;
        for _ in 0..10 {
            let (s2, i2, end) = session_next(false, seen, idle);
            seen = s2;
            idle = i2;
            assert!(!end, "宽限期内不得结束会话");
        }
        let (s2, i2, end) = session_next(true, seen, idle);
        assert!(s2 && i2 == 0 && !end);
        assert!(!session_next(false, true, 0).2);
        assert!(!session_next(false, true, 1).2);
        assert!(session_next(false, true, 2).2, "连续 3 拍 false 才收尾");
        // 从未见过 true：宽限期到点仍要收尾，不能永久挂着会话。
        assert!(session_next(false, false, SESSION_ARM_TIMEOUT_TICKS - 1).2);
    }

    #[test]
    fn test_lock_btn_rect_at_144dpi() {
        let win_w = 600;
        let title_h = 60;
        let dpi = 144;
        let (btn_x, btn_y, btn_size) = lock_btn_rect(win_w, title_h, dpi);
        assert_eq!(btn_size, 39);
        let (more_x, _, _more_size) = more_btn_rect(win_w, title_h, dpi);
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
        let (more_x, _, _more_size) = more_btn_rect(win_w, title_h, dpi);
        assert!(btn_x < more_x, "锁定按钮应在更多按钮左侧");
        assert_eq!(btn_y, 14);
    }

    #[test]
    fn test_lock_btn_rect_narrow_window() {
        // 窄窗口：按钮不应超出窗口右边界
        let win_w = 100;
        let title_h = 40;
        let dpi = 96;
        let (btn_x, _btn_y, _btn_size) = lock_btn_rect(win_w, title_h, dpi);
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
}

#[cfg(test)]
mod liquid_glass_timer_tests {
    use super::LIQUID_GLASS_DRAG_INTERVAL_MS;

    /// 会话跟随档必须能支撑「至少 25 帧」的下限。
    ///
    /// Windows 定时器默认粒度约 15.6ms：请求值小于粒度时实际按粒度触发，
    /// 因此真实间隔上限是 `max(请求值, 15.6)` 再向上取一个粒度。
    /// 这里按最坏情况（实际间隔 = 请求值 + 一个粒度）校验仍有 ≥25fps。
    #[test]
    fn drag_interval_meets_25fps_floor() {
        let worst_case_ms = LIQUID_GLASS_DRAG_INTERVAL_MS as f64 + 15.6;
        let fps = 1000.0 / worst_case_ms;
        assert!(
            fps >= 25.0,
            "跟随档最坏情况 {fps:.1}fps 低于 25fps 下限（间隔 {worst_case_ms:.1}ms）"
        );
    }

    /// 跟随档不能快到把定时器打成忙等：请求值不低于 8ms。
    #[test]
    fn drag_interval_is_not_busy_loop() {
        assert!(
            LIQUID_GLASS_DRAG_INTERVAL_MS >= 8,
            "跟随档 {}ms 过短，会退化成忙等",
            LIQUID_GLASS_DRAG_INTERVAL_MS
        );
    }
}
