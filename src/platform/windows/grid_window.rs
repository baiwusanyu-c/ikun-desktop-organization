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
    IDCompositionTarget, IDCompositionVisual, IDCompositionVisual2, IDCompositionVisual3,
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
    /// 存下来是为了让液态玻璃能**直接 memcpy 进主 DIB**：玻璃回读缓冲
    /// `glass_pixels` 与本 DIB 的尺寸、格式、行距逐字节一致，直写即可，
    /// 不必再上传成 D2D 位图做全窗软件混合。详见
    /// `d2d_render::render_glass_into_dib`。
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
    /// 留白随液态玻璃开关变化，渲染、布局与命中测试都必须读这个字段而不是
    /// 每次按 DPI 重新推算：否则玻璃刚关闭时窗口外框还没收缩，渲染却已经
    /// 按新留白计算，会出现整框错位一帧。
    pub(crate) render_padding: i32,

    /// 液态玻璃像素回读缓冲，跨帧复用。
    ///
    /// 玻璃开启时每帧要接收一张 1~5MB 的 GPU 回读表面，
    /// 旧实现每帧新分配一块再丢掉；5 个框 × 5fps 就是每秒
    /// 上百 MB 的大对象申请/归还，堆的已提交容量会单调上涨。
    /// 注意：D2D 位图不能同样跨帧缓存，原因见
    /// `draw_liquid_glass_background` 的注释。
    pub(crate) glass_pixels: Vec<Pixel32>,
    /// 玻璃表面对应的 D2D 位图（尺寸变化时重建，
    /// 否则 CopyFromMemory 原地更新）
    pub(crate) glass_bitmap: Option<ID2D1Bitmap>,
    pub(crate) glass_bitmap_w: u32,
    pub(crate) glass_bitmap_h: u32,

    // ===== 周期重绘的变化检测状态 =====
    //
    // 液态玻璃不是时间驱动的动画，它只响应三件事：桌面内容变化、
    // 光标位置（决定高光与弹性形变）、以及分类框自身状态
    // （后者一律走显式重绘）。下面四个字段让 200ms 定时器
    // 能判断「这一帧与上一帧完全一样」从而整帧跳过。
    /// 上一次实际渲染时的光标屏幕坐标
    pub(crate) last_cursor_x: i32,
    pub(crate) last_cursor_y: i32,
    /// 光标离开本框后经过的 tick 数。
    ///
    /// 用来决定「光标移动还要不要触发本框重绘」：着色器的高光/形变
    /// 只对本框内的光标有反应，光标在别处移动时本框画出来逐像素相同。
    pub(crate) cursor_left_ticks: u32,
    /// 光标静止后经过的 tick 数。
    ///
    /// 光标停下后 `light_direction` 仍在按 0.14 的系数逐帧插值收敛，
    /// 收敛窗口内必须继续渲染，否则形变会停在中途。
    pub(crate) cursor_idle_ticks: u32,
    /// 距上一次实际渲染经过的 tick 数（心跳兜底用）
    pub(crate) ticks_since_render: u32,
    /// 上一次实际渲染时记录的桌面代号
    pub(crate) last_desktop_generation: u64,

    // ===== 遮挡判定状态 =====
    /// 上次判定结果：被其他应用窗口完全盖住，可以跳过渲染
    pub(crate) occluded: bool,
    /// 上次判定时刻。窗口枚举不便宜，按间隔限流
    pub(crate) occlusion_checked_at: std::time::Instant,
    /// 被判定遮挡后已跳过的 tick 数（驱动自愈心跳）
    pub(crate) occluded_ticks: u32,
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
            glass_pixels: Vec::new(),
            glass_bitmap: None,
            glass_bitmap_w: 0,
            glass_bitmap_h: 0,
            last_cursor_x: 0,
            last_cursor_y: 0,
            cursor_left_ticks: 0,
            cursor_idle_ticks: 0,
            ticks_since_render: 0,
            last_desktop_generation: 0,
            occluded: false,
            occlusion_checked_at: std::time::Instant::now(),
            occluded_ticks: 0,
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

/// 液态玻璃外投影在分类框四周预留的逻辑像素。
///
/// 128px 可覆盖最强亮区模式下约 3 倍高斯标准差加垂直偏移，避免投影在
/// `WS_EX_LAYERED` 窗口边界被矩形裁切。
pub(crate) const LIQUID_GLASS_SHADOW_PADDING: i32 = 48;
// 旧值 128 逻辑像素。2026-09-14 对 d09da5b1 参照帧的 alpha 剖面实测：
// 外投影在内容边缘外 ~96 物理像素处 alpha 已 ≤ 1/255，
// 128 逻辑（192 DPI 下 256 物理）的留白绝大部分是全透明死区，
// 却让每帧 ULW 提交面/DIB/暂存按 (content+512)^2 付钱。
// 收到 48 逻辑（96 物理）后提交面积 -40%（DWM 带宽与拖拽跟手感直接相关），
// 视觉上裁掉的只是 alpha≤1/255 的不可见部分。
/// 液态玻璃渲染定时器间隔（毫秒）
///
/// 从原始的 33ms (~30 FPS) 降低到 200ms (~10 FPS)，
/// 大幅减少 GPU 负载和与 GPUI 渲染帧的 RefCell 冲突频率。
/// 视觉差异极小（液态玻璃效果变化较慢），但 GPU 开销降低约 70%。
pub(crate) const LIQUID_GLASS_RENDER_INTERVAL_MS: u32 = 40;

/// 拖拽 / 缩放分类框期间的重绘间隔（毫秒）。
///
/// # 为什么必须单独给拖拽一条高频路径
///
/// 静止时用 200ms（5fps）是划算的：桌面没变，画了也看不出差别。
/// 但拖拽时**窗口位置每帧都在变**，玻璃要重新采样新位置背后的桌面，
/// 5fps 的上限与渲染快慢无关 —— 即使单帧耗时降到 0，拖拽照样只有
/// 5 帧。用户看到的「移动分类框时玻璃卡顿」根因就在这里，而不在
/// 单帧耗时。
///
/// 16ms ≈ 60Hz，与 DWM 合成节奏对齐；Windows 定时器默认粒度约
/// 15.6ms，因此实际 tick 落在 15.6~31.2ms，即 32~64fps，
/// 稳超「至少 25 帧」的下限。
///
/// # 为什么不会把 CPU 打满
///
/// 只有**正在被拖动的那一个框**会走到渲染：拖拽时光标必然落在它
/// 内部（`cursor_relevant = true`），`decide_tick_action` 判为
/// `Render(Cursor)`；其余框光标不相关、桌面也没变，判为
/// `AdvanceLightOnly`，只推进一次高光插值就返回，不进管线。
/// 鼠标按住不动时 `cursor_moved` 转假，超过 `GLASS_CURSOR_SETTLE_TICKS`
/// 后连被拖的框也转 `SkipIdle`，不会空转。
pub(crate) const LIQUID_GLASS_DRAG_INTERVAL_MS: u32 = 16;

/// 光标静止后继续渲染的 tick 数，用于等待高光/形变插值收敛。
///
/// `light_direction` 每帧按 `LIGHT_BLEND = 0.14` 向目标靠拢，残差为
/// `0.86^n`；n=25 时剩 2.2%，肉眼已看不出差别，再渲染只是白白浪费。
pub(crate) const GLASS_CURSOR_SETTLE_TICKS: u32 = 25;

/// 周期重绘的心跳兜底间隔（tick 数）。
///
/// 200ms 一个 tick，50 个 = 10 秒。
///
/// # 为什么从 2 秒放宽到 10 秒
///
/// 心跳是**兜底**：桌面脏矩形、光标移动、光标静止后的插值收敛、
/// 分类框自身状态变化都各有独立且更灵敏的触发路径。静止桌面上
/// 心跳渲染出来的那一帧与上一帧逐像素相同，纯属白干。
///
/// 但每一次白干都要走完整条管线：DXGI 取帧 → D2D 模糊 → D3D 着色 →
/// 暂存纹理回读 → `CopyFromMemory` → 整窗 DIB 重画 → `UpdateLayeredWindow`。
/// 实测（`[内存分解]` 日志）这条管线在显卡用户态驱动里留下的已提交内存
/// 以 **~1.2MB/分** 缓慢爬升，而同一时刻 Rust 堆活跃字节恒定在 6.4MB、
/// 块数一动不动 —— 涨的部分完全与渲染次数成正比。
/// 5 个框 × 每 2 秒一次 = 每秒 2.5 次白干，一天 21 万次。
///
/// 放宽到 10 秒后兜底渲染降到 1/5。代价只存在于「所有变化检测同时漏报」
/// 的假想场景：那时画面滞后从 2 秒变成 10 秒。正常使用中一帧都不会多等。
pub(crate) const GLASS_FORCE_RENDER_TICKS: u32 = 250;

/// 单位是 tick（LIQUID_GLASS_RENDER_INTERVAL_MS = 40ms）：250 tick = 10 秒。
/// 不要把它改回 1：那是饱和压测值，会让静止/遮挡桌面每 40ms
/// 白烧一整条渲染管线，驱动提交内存按渲染次数正比爬升。

/// 被判定为遮挡后仍然强制渲染的心跳间隔（tick 数）。
///
/// 遮挡判定是启发式的，万一误报（比如某个窗口报告可见
/// 但实际没画出来），5 秒一次的自愈心跳能保证分类框
/// 不会永久冻在旧画面上。
pub(crate) const GLASS_OCCLUDED_HEARTBEAT_TICKS: u32 = 125;

/// 单位是 tick：125 tick = 5 秒。同上，不要改回 1。

/// 光标离开本框后还要继续渲染的 tick 数。
///
/// 着色器里 `mouse_offset` 与 `elasticity` 都只在「光标在本框内」时
/// 才取 `light_direction`，光标一离开，输出立刻变成无高光版本 ——
/// 所以只需要再画一帧就能把残留的高光/形变覆盖掉。给 3 个 tick 的
/// 余量是为了容忍光标在框边缘来回抖动。
///
/// 注意这与 `GLASS_CURSOR_SETTLE_TICKS`（25）不是一回事：后者是
/// 「光标停在框内不动时，等高光插值收敛完」，那期间**必须**持续渲染。
pub(crate) const CURSOR_LEAVE_SETTLE_TICKS: u32 = 3;

/// 返回当前 DPI 下液态玻璃外投影所需的物理像素留白（纯 DPI 换算，不看开关）。
pub(crate) fn glass_shadow_padding_at_dpi(dpi: u32) -> i32 {
    scale_for_dpi(LIQUID_GLASS_SHADOW_PADDING, dpi.max(1))
}

/// 返回当前 DPI 下分类框实际应使用的外投影留白。
///
/// 液态玻璃关闭时返回 0：外投影完全由玻璃着色器画在内容矩形之外，关闭后
/// 继续保留 128 逻辑像素留白会把每帧 DIB、DWM 重定向表面和命中测试区域
/// 一起放大到内容面积的 3 倍以上（200% DPI 下 5 个分类框每帧 23MB → 7MB），
/// 还会在窗口四周多出一大圈能吃鼠标事件的透明区域。
pub(crate) fn shadow_padding_for_dpi(dpi: u32) -> i32 {
    if super::liquid_glass::is_liquid_glass_enabled() {
        glass_shadow_padding_at_dpi(dpi)
    } else {
        0
    }
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
    let dcomp_pre = if dcomp_enabled() {
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
        glass_pixels: Vec::new(),
        glass_bitmap: None,
        glass_bitmap_w: 0,
        glass_bitmap_h: 0,
        last_cursor_x: 0,
        last_cursor_y: 0,
        cursor_left_ticks: 0,
        cursor_idle_ticks: 0,
        ticks_since_render: 0,
        last_desktop_generation: 0,
        occluded: false,
        occlusion_checked_at: std::time::Instant::now(),
        occluded_ticks: 0,
    });
    let data_ptr = Box::into_raw(data);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
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
    unsafe {
        // 先完成首帧再显示窗口，避免桌面捕获尚未就绪时短暂出现黑/透明窗口。
        ShowWindow(hwnd, SW_SHOW);
    }
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
    teardown_dcomp_state(state);
}

/// 把一份 DComp 状态从 DWM 合成树上摘下来。
///
/// WM_DESTROY 兑底路径拿到的是 GridWindowData 里的所有权，不经 hwnd 查表，
/// 所以抆树动作单独抽出来两边共用。
pub(crate) fn teardown_dcomp_state(state: Box<DcompState>) {
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
        let dpi = unsafe { GetDpiForWindow(hwnd) };
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
        if let Err(error) = unsafe { root.SetTransform2(&scale_matrix) } {
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
        D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
        ID3D11Texture2D,
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
        let mapped = unsafe { mapped.assume_init() };
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

/// 按当前液态玻璃开关装配 / 卸载分类框的周期重绘定时器。
///
/// 玻璃关闭时画面是一张静态位图，所有会改变它的操作都走显式重绘路径，
/// 200ms 定时器只会白白唤醒消息循环（实测约 9% CPU）并持续制造内存抖动。
pub(crate) fn sync_grid_refresh_timer(hwnd: HWND) {
    set_grid_refresh_interval(hwnd, LIQUID_GLASS_RENDER_INTERVAL_MS);
}

/// 进入拖拽 / 缩放模态循环：把周期重绘切到高频档。
///
/// 只在 `WM_ENTERSIZEMOVE` 调用。玻璃关闭时不装定时器 —— 那时画面
/// 是一张静态位图，窗口移动由 DWM 直接搬运，不需要我们重画。
pub(crate) fn enter_drag_refresh(hwnd: HWND) {
    set_grid_refresh_interval(hwnd, LIQUID_GLASS_DRAG_INTERVAL_MS);
}

/// 退出拖拽 / 缩放模态循环：恢复静止档间隔。
///
/// 必须与 `enter_drag_refresh` 严格配对。漏掉这一步会让分类框
/// 永久以 60Hz 重绘 —— 画面看不出问题，但 CPU 与驱动内存常驻高水位
/// 都下不来，而且没有任何报错。
pub(crate) fn exit_drag_refresh(hwnd: HWND) {
    sync_grid_refresh_timer(hwnd);
}

/// 按指定间隔重装周期重绘定时器；`interval_ms` 为静止档或拖拽档。
///
/// 玻璃关闭时一律不装：所有会改变画面的操作都走显式重绘路径，
/// 定时器只会白白唤醒消息循环（实测约 9% CPU）并持续制造内存抖动。
fn set_grid_refresh_interval(hwnd: HWND, interval_ms: u32) {
    unsafe { KillTimer(hwnd, TIMER_ID_LIQUID_GLASS) };
    if !super::liquid_glass::is_liquid_glass_enabled() {
        log::warn!("[timerdbg] set_interval {} skipped: glass disabled", interval_ms);
        return;
    }
    let id = unsafe {
        SetTimer(
            hwnd,
            TIMER_ID_LIQUID_GLASS,
            interval_ms,
            std::ptr::null_mut(),
        )
    };
    log::warn!("[timerdbg] set_interval {}ms -> timer id {}", interval_ms, id);
}

/// 读取当前光标的屏幕坐标（失败时返回 None）。
fn cursor_screen_pos() -> Option<(i32, i32)> {
    let mut cursor = windows::Win32::Foundation::POINT { x: 0, y: 0 };
    // SAFETY: 只往本地栈上的 POINT 里写
    let ok = unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut cursor) }.is_ok();
    ok.then_some((cursor.x, cursor.y))
}

/// 记录一次实际渲染，供下一个 tick 判断画面是否会变。
///
/// 无论是定时器还是显式调用触发的渲染都要记一笔，
/// 否则下一个 tick 会误以为画面还没更新过。
pub(crate) fn note_grid_frame_rendered(data: &mut GridWindowData) {
    data.ticks_since_render = 0;
    data.last_desktop_generation = super::liquid_glass::desktop_generation();
    if let Some((x, y)) = cursor_screen_pos() {
        data.last_cursor_x = x;
        data.last_cursor_y = y;
    }
}

/// 200ms 周期 tick：只有画面真的会变才渲染。
///
/// 旧实现每个 tick 无条件重绘，5 个框 × 5fps = 每秒 25 帧，
/// 每帧新建上百个 DirectWrite / D2D 小对象（标题、锁定、更多、
/// 每个标签各一个 TextLayout）。这些对象都会释放，但高频
/// 申请/归还会让 Windows 堆的已提交容量单调上涨：
/// 实测 +36MB/分，并伴随 ~28MB 的堆段提交台阶。
///
/// 判据全部保守：任一条件成立就渲染，并且有 2 秒心跳兜底。
/// 分类框自身状态变化（hover / 选中 / 拖拽 / 滚动 / 重命名 /
/// 配置变更）一律走显式 `render_layered_window`，不经过本函数。
/// 周期 tick 的判定结果。
/// 一个 tick 的动作。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TickAction {
    /// 整帧重绘
    Render(RenderReason),
    /// 只推进共享的光标高光插值，不重绘。
    ///
    /// 旧逻辑里「光标动了」会让**所有**分类框重绘，而光标只影响它
    /// 所在的那一个框。但 `light_direction` 是渲染器全局共享的，
    /// 直接跳过重绘会改变它的插值序列、让高光突然变亮约 3.8 倍，
    /// 所以这些框仍要按原节奏推进插值 —— 只是不必再跑
    /// DXGI 取帧 → D2D 模糊 → D3D 着色 → GDI 回读 → 提交这条管线。
    AdvanceLightOnly,
    /// 桌面与光标都没动，整帧省掉
    SkipIdle,
    /// 被其他应用窗口完全盖住，整条管线（含 DXGI 采样）都不跑
    SkipOccluded,
}

/// 一帧被判定为「要渲染」的原因。
///
/// # 为什么必须分开计数
///
/// 只报一个「实际渲染 N 次」是看不出脏矩形方案有没有真在工作的：
/// N 偏大时，可能是光标在动（正常）、可能是心跳（正常）、
/// 也可能是 `region_changed_since` 退化成了「桌面任何一处变了都返回
/// true」（**按框判定失效**）。这三种情况的处理方式完全不同，
/// 混在一个数字里就只能靠猜。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RenderReason {
    /// 光标移动过
    Cursor,
    /// 这个框自己覆盖的桌面区域变了（脏矩形命中）
    DesktopChanged,
    /// 光标刚停下，高光/形变插值还在收敛
    Settle,
    /// 兜底心跳
    Heartbeat,
}

/// 决定这个 tick 干什么：重绘 / 只推进光标插值 / 完全跳过。
///
/// 与「旧规则」的唯一差别是 `cursor_driven && !cursor_relevant && !must_render`
/// 这一种组合：旧规则重绘，新规则只推进插值。其余所有组合的
/// 渲染与否、以及上报的原因，都与旧规则逐条一致。
fn decide_tick_action(
    cursor_moved: bool,
    desktop_changed: bool,
    cursor_idle_ticks: u32,
    ticks_since_render: u32,
    cursor_relevant: bool,
) -> TickAction {
    let cursor_driven = cursor_moved || cursor_idle_ticks < GLASS_CURSOR_SETTLE_TICKS;
    let must_render = desktop_changed || ticks_since_render >= GLASS_FORCE_RENDER_TICKS;
    if cursor_driven && !cursor_relevant && !must_render {
        return TickAction::AdvanceLightOnly;
    }
    match pick_render_reason(
        cursor_moved,
        desktop_changed,
        cursor_idle_ticks,
        ticks_since_render,
    ) {
        Some(reason) => TickAction::Render(reason),
        None => TickAction::SkipIdle,
    }
}

/// 按与判定完全一致的优先级挑出渲染原因；`None` 表示这一帧可以省掉。
///
/// 纯函数，便于单测：优先级排错会让统计说谎，而统计是判断
/// 脏矩形过滤有没有退化的唯一依据。
fn pick_render_reason(
    cursor_moved: bool,
    desktop_changed: bool,
    cursor_idle_ticks: u32,
    ticks_since_render: u32,
) -> Option<RenderReason> {
    if cursor_moved {
        Some(RenderReason::Cursor)
    } else if desktop_changed {
        Some(RenderReason::DesktopChanged)
    } else if cursor_idle_ticks < GLASS_CURSOR_SETTLE_TICKS {
        Some(RenderReason::Settle)
    } else if ticks_since_render >= GLASS_FORCE_RENDER_TICKS {
        Some(RenderReason::Heartbeat)
    } else {
        None
    }
}

/// 渲染统计的输出间隔。
///
/// 每 30 秒一条，用于确认帧跳过真的在生效——否则
/// 「内存不涨了」可能只是因为根本没在渲染（画面冻住了）。
const TICK_STATS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// 一个统计窗口内的渲染计数。
#[derive(Clone, Copy)]
struct TickStats {
    ticks: u64,
    rendered: u64,
    /// 只推进了光标插值、没重绘的次数（光标在别处移动时的省帧量）
    light_only: u64,
    occluded: u64,
    by_cursor: u64,
    by_desktop: u64,
    by_settle: u64,
    by_heartbeat: u64,
    last_report: Option<std::time::Instant>,
}

const fn new_tick_stats() -> TickStats {
    TickStats {
        ticks: 0,
        rendered: 0,
        light_only: 0,
        occluded: 0,
        by_cursor: 0,
        by_desktop: 0,
        by_settle: 0,
        by_heartbeat: 0,
        last_report: None,
    }
}

thread_local! {
    static TICK_STATS: std::cell::RefCell<TickStats> =
        const { std::cell::RefCell::new(new_tick_stats()) };
}

fn record_tick_stats(decision: TickAction) {
    TICK_STATS.with(|cell| {
        let snapshot = {
            let mut stats = cell.borrow_mut();
            stats.ticks += 1;
            match decision {
                TickAction::Render(reason) => {
                    stats.rendered += 1;
                    match reason {
                        RenderReason::Cursor => stats.by_cursor += 1,
                        RenderReason::DesktopChanged => stats.by_desktop += 1,
                        RenderReason::Settle => stats.by_settle += 1,
                        RenderReason::Heartbeat => stats.by_heartbeat += 1,
                    }
                }
                TickAction::AdvanceLightOnly => stats.light_only += 1,
                TickAction::SkipOccluded => stats.occluded += 1,
                TickAction::SkipIdle => {}
            }
            let now = std::time::Instant::now();
            let due = match stats.last_report {
                Some(last) => now.duration_since(last) >= TICK_STATS_INTERVAL,
                None => true,
            };
            if !due {
                return;
            }
            let snapshot = *stats;
            let keep = stats.last_report;
            *stats = new_tick_stats();
            stats.last_report = Some(now);
            let _ = keep;
            snapshot
        };
        let skipped = snapshot
            .ticks
            .saturating_sub(snapshot.rendered)
            .saturating_sub(snapshot.occluded)
            .saturating_sub(snapshot.light_only);
        log::info!(
            "[分类框渲染] 近 {} tick → 实际渲染 {}（光标 {} / 桌面变化 {} / 收敛 {} / 心跳 {}），\
             仅推进光标插值 {}，静止跳过 {}，遮挡跳过 {}",
            snapshot.ticks,
            snapshot.rendered,
            snapshot.by_cursor,
            snapshot.by_desktop,
            snapshot.by_settle,
            snapshot.by_heartbeat,
            snapshot.light_only,
            skipped,
            snapshot.occluded
        );
        // 内存分解单独节流（60 秒一条），与渲染统计错开，
        // 便于把「渲染量」与「内存去向」两条曲线对齐看。
        if super::mem_probe::due() {
            super::mem_probe::log_breakdown();
        }
    });
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
    /// 玻璃像素回读缓冲合计字节
    pub glass_pixels_bytes: u64,
    /// 玻璃 D2D 位图合计字节（GPU 侧）
    pub glass_bitmap_bytes: u64,
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
        stats.glass_pixels_bytes +=
            data.glass_pixels.len() as u64 * std::mem::size_of::<Pixel32>() as u64;
        if data.glass_bitmap.is_some() {
            stats.glass_bitmap_bytes += data.glass_bitmap_w as u64 * data.glass_bitmap_h as u64 * 4;
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
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRST: AtomicBool = AtomicBool::new(false);
        if !FIRST.swap(true, Ordering::Relaxed) {
            log::warn!("[tickdbg] first tick hwnd={:x}", hwnd as usize);
        }
    }
    // 判定与渲染分两步：decide 里持有 data 的可变借用，
    // 必须先释放才能调 render_layered_window（它会重新取一次 data）。
    let action = decide_grid_refresh(hwnd);
    record_tick_stats(action);
    if matches!(action, TickAction::Render(_)) {
        // 周期 tick 走「内容层可复用」的入口：拖拽分类框、动态壁纸这些
        // 高频场景里内容一个像素都没变，变的只有玻璃背景，没必要每帧
        // 重跑二十几次字形光栅化（实测占整帧 27.6ms 里的 17.4ms）。
        // 所有显式重绘入口仍然走 render_layered_window（全量重画），
        // 所以任何状态变化都不会因为复用而漏掉。
        let dragging = super::wnd_proc::in_modal_move_or_resize() && super::wnd_proc::drag_hwnd() == hwnd;
        let t0 = std::time::Instant::now();
        super::d2d_render::render_layered_window_glass_only(hwnd);
        super::mem_probe::record_drag_frame(hwnd as usize, dragging, t0, t0.elapsed());
    }
}

/// 判断这个 tick 是否真的需要重绘。
///
/// 三层判据，从便宜到昂贵：
/// 1. **遮挡**：被其他应用窗口完全盖住 → 整条管线都不用跑；
/// 2. **光标**：移动过或刚停下不久（高光/形变还在插值收敛）；
/// 3. **桌面脏矩形**：只看这个框自己覆盖的那块桌面变了没。
///    任务栏时钟、别处光标闪烁都不会再让所有框陪绘。
///
/// 另外有两层兜底心跳：正常 10 秒一次，遮挡态下 5 秒一次。
///
/// 光标移动只对**光标所在的那个框**构成重绘理由；其余框这一帧画出来
/// 逐像素相同，只补一次共享高光插值（见 `TickAction::AdvanceLightOnly`）。
fn decide_grid_refresh(hwnd: HWND) -> TickAction {
    let Some(data) = (unsafe { get_data(hwnd) }) else {
        return TickAction::SkipIdle;
    };
    data.ticks_since_render = data.ticks_since_render.saturating_add(1);

    // 外框矩形取一次，遮挡判定与脏矩形判定都用它
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let have_rect = unsafe { GetWindowRect(hwnd, &mut outer) } != 0;
    // RECT 不是 Copy，后面 `content_rect_with_padding(outer, ..)` 会把它移走，
    // 而补光标插值时还要用外框坐标，所以在这里先抄一份到函数作用域
    let (outer_left, outer_top, outer_right, outer_bottom) =
        (outer.left, outer.top, outer.right, outer.bottom);

    // ===== 1. 全屏遮挡 =====
    let now = std::time::Instant::now();
    if now.duration_since(data.occlusion_checked_at) >= super::occlusion::OCCLUSION_CHECK_INTERVAL {
        data.occlusion_checked_at = now;
        let occluded = have_rect && super::occlusion::is_window_occluded(hwnd, &outer);
        if occluded != data.occluded {
            log::info!(
                "分类框遮挡状态切换: {} → {}（上方候选窗口已逐个排除自己/不可见/cloaked）",
                data.occluded,
                occluded
            );
        }
        data.occluded = occluded;
    }
    if data.occluded {
        data.occluded_ticks = data.occluded_ticks.saturating_add(1);
        if data.occluded_ticks < GLASS_OCCLUDED_HEARTBEAT_TICKS {
            return TickAction::SkipOccluded;
        }
        data.occluded_ticks = 0;
    } else {
        data.occluded_ticks = 0;
    }

    // ===== 2. 光标 =====
    let cursor_pos = cursor_screen_pos();
    let cursor_moved = match cursor_pos {
        Some((x, y)) => x != data.last_cursor_x || y != data.last_cursor_y,
        None => false,
    };
    if cursor_moved {
        data.cursor_idle_ticks = 0;
    } else {
        data.cursor_idle_ticks = data.cursor_idle_ticks.saturating_add(1);
    }
    // 光标是否落在**本框**外框内（含外投影留白：高光与弹性形变作用在整块玻璃上）。
    // 判定口径必须与 `GlassRenderer::advance_light` 里的 cursor_in_window 一致，
    // 否则会出现「该画的不画」或「不该画的白画」。
    let cursor_inside = match (cursor_pos, have_rect) {
        (Some((cx, cy)), true) => {
            cx >= outer.left && cx < outer.right && cy >= outer.top && cy < outer.bottom
        }
        _ => false,
    };
    if cursor_inside {
        data.cursor_left_ticks = 0;
    } else {
        data.cursor_left_ticks = data.cursor_left_ticks.saturating_add(1);
    }
    // 光标在框内、或刚离开不久（还要把残留高光那一帧覆盖掉）时，
    // 光标移动才与本框有关；否则本框画出来逐像素相同。
    let cursor_relevant = cursor_inside || data.cursor_left_ticks < CURSOR_LEAVE_SETTLE_TICKS;

    // ===== 3. 桌面脏矩形：只看这个框自己覆盖的区域 =====
    // 不能调 get_grid_content_rect：它内部会再取一次 data，
    // 与此处的可变借用冲突，所以直接用 render_padding 算。
    // 自拖期间桌面背景必变（框在移动），跳过脏矩形+像素指纹两级判定：
    // 后者每帧一次 GPU 子区域拷贝 + Map 同步，实测是偶发 30ms+ 卡顿的来源之一。
    let self_dragging =
        super::wnd_proc::in_modal_move_or_resize() && super::wnd_proc::drag_hwnd() == hwnd;
    // 自拖期间渲染完全交给 WM_MOVING 的 8ms 节流同步渲染：
    // tick 再叠一路渲染会让 GPU 提交队列饱和（实测间隔退化到 74-114ms）。
    let desktop_changed = if self_dragging {
        false
    } else if have_rect {
        // 先把新帧取回来，否则不渲染就永远不知道桌面变了
        super::liquid_glass::poll_desktop_changes();
        let content = content_rect_with_padding(outer, data.render_padding);
        let region = windows::Win32::Foundation::RECT {
            left: content.left,
            top: content.top,
            right: content.right,
            bottom: content.bottom,
        };
        // 两级判定，从便宜到贵：
        //   1) 脏矩形队列里有没有矩形碰到这个框（纯 CPU，几微秒）；
        //   2) 碰到了再比这块桌面的**像素**指纹（一次 GPU 子区域拷贝 + Map）。
        //
        // 为什么必须有第二级：本机驱动把「桌面每秒整块重绘一次」报成
        // 一个覆盖整个输出的矩形，第一级于是永远命中，5 个框每秒陪绘
        // 5 次。实测把渲染频率从 5 次/秒拉到 24.5 次/秒，取帧率纹丝不动，
        // 说明那个整屏 present 来自外部、驱动给不出空间粒度 ——
        // 只有比像素才能拿到真正的按框粒度。
        //
        // 两级任一不确定都保守放行（`region_changed_since` 的兜底、
        // `desktop_region_changed` 失败时返回 true），再加上周期心跳，
        // 最坏情况只是多画一帧或滞后一个心跳，绝不会永久停在旧画面。
        super::liquid_glass::region_changed_since(&region, data.last_desktop_generation)
            && super::liquid_glass::desktop_region_changed(&region)
    } else {
        // 拿不到矩形就不猜，照常渲染
        true
    };

    let action = decide_tick_action(
        if self_dragging { false } else { cursor_moved },
        desktop_changed,
        data.cursor_idle_ticks,
        data.ticks_since_render,
        cursor_relevant,
    );
    if let TickAction::AdvanceLightOnly = action {
        // 旧逻辑在这一帧会重绘，而重绘会顺带把全局共享的 light_direction
        // 朝 0 拉一次。直接跳过会改变插值序列、让光标所在框的高光
        // 突然变亮约 3.8 倍，所以这里补一次「只推进插值、不重绘」。
        let padding = data.render_padding;
        let content_w = (outer_right - outer_left) - padding * 2;
        let content_h = (outer_bottom - outer_top) - padding * 2;
        super::liquid_glass::advance_light_direction(
            outer_left, outer_top, content_w, content_h, padding,
        );
    }
    action
}

/// 保持分类框内容矩形不动，按当前液态玻璃开关重设分层窗口外框。
///
/// 玻璃关闭后 128 逻辑像素的外投影留白失去意义，必须同步收缩窗口外框，
/// 否则每帧 DIB、DWM 重定向表面与命中测试区域仍按放大 3 倍的面积分配。
/// 内容矩形保持不变 = 用户看到的分类框位置尺寸完全不受开关影响。
pub(crate) fn sync_grid_window_padding(hwnd: HWND) {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi > 0 { dpi } else { 96 };

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return;
    }

    let old_padding = unsafe { get_data(hwnd) }
        .map(|data| data.render_padding)
        .unwrap_or(0);
    let new_padding = shadow_padding_for_dpi(dpi);
    if new_padding == old_padding {
        return;
    }

    let content = content_rect_with_padding(rect, old_padding);
    // 先写回新留白：SetWindowPos 会同步派发 WM_SIZE，
    // 处理函数按 render_padding 反算内容矩形，必须已经是新值。
    if let Some(data) = unsafe { get_data(hwnd) } {
        data.render_padding = new_padding;
    }
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            content.left - new_padding,
            content.top - new_padding,
            (content.right - content.left) + new_padding * 2,
            (content.bottom - content.top) + new_padding * 2,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    log::info!(
        "分类框外投影留白切换: {} → {} (dpi={})",
        old_padding,
        new_padding,
        dpi
    );
    // 留白变了 → 分层位图尺寸变了 → 立即重绘一帧，避免残留旧画面
    render_layered_window(hwnd);
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

/// 液态玻璃开关变化后，同步所有分类框的留白与周期重绘定时器。
pub fn sync_all_grid_windows_to_glass_state() {
    post_to_all_grid_windows(WM_USER_GRID_SYNC_GLASS);
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

    /// 四个条件的优先级必须与判定一致：光标 > 桌面变化 > 收敛 > 心跳。
    ///
    /// 排错顺序会让统计把「脏矩形退化」误报成「光标在动」，
    /// 那就再也看不出按框判定有没有失效了。
    #[test]
    fn render_reason_priority_matches_decision() {
        // 全部命中时取最靠前的
        assert_eq!(
            pick_render_reason(true, true, 0, GLASS_FORCE_RENDER_TICKS),
            Some(RenderReason::Cursor)
        );
        // 光标没动，才轮到桌面变化
        assert_eq!(
            pick_render_reason(false, true, 0, GLASS_FORCE_RENDER_TICKS),
            Some(RenderReason::DesktopChanged)
        );
        assert_eq!(
            pick_render_reason(false, false, 0, GLASS_FORCE_RENDER_TICKS),
            Some(RenderReason::Settle)
        );
        assert_eq!(
            pick_render_reason(
                false,
                false,
                GLASS_CURSOR_SETTLE_TICKS,
                GLASS_FORCE_RENDER_TICKS
            ),
            Some(RenderReason::Heartbeat)
        );
    }

    /// # 穷举证明：新判定与旧判定的**渲染集合**只差一种组合
    ///
    /// 这条是「少画 4/5 帧但不改变画面」的全部依据。旧规则是
    /// `cursor_moved || desktop_changed || 光标收敛中 || 心跳到期`
    /// 四个条件的或；新规则只允许在
    /// 「仅因光标而触发 && 光标与本框无关 && 没有别的理由」这一种组合下
    /// 把重绘降级成"只推进共享高光插值"，其余任何组合都必须与旧规则一致。
    /// 32 种组合全部枚举，任何一种越界都会当场失败。
    #[test]
    fn decide_tick_action_only_diverges_on_irrelevant_cursor() {
        for cursor_moved in [false, true] {
            for desktop_changed in [false, true] {
                for idle in [0u32, GLASS_CURSOR_SETTLE_TICKS] {
                    for since in [0u32, GLASS_FORCE_RENDER_TICKS] {
                        for relevant in [false, true] {
                            let action = decide_tick_action(
                                cursor_moved,
                                desktop_changed,
                                idle,
                                since,
                                relevant,
                            );
                            let cursor_driven = cursor_moved || idle < GLASS_CURSOR_SETTLE_TICKS;
                            let must_render = desktop_changed || since >= GLASS_FORCE_RENDER_TICKS;
                            let old_would_render = cursor_driven || must_render;
                            let ctx = format!(
                                "cursor_moved={cursor_moved} desktop_changed={desktop_changed} \
                                 idle={idle} since={since} relevant={relevant}"
                            );
                            match action {
                                TickAction::Render(_) => {
                                    assert!(old_would_render, "旧规则不会渲染却渲染了: {ctx}");
                                }
                                TickAction::AdvanceLightOnly => {
                                    assert!(
                                        old_would_render,
                                        "只推进插值的前提是旧规则本来会渲染: {ctx}"
                                    );
                                    assert!(
                                        cursor_driven && !relevant && !must_render,
                                        "只有「仅光标触发且与本框无关」才允许降级: {ctx}"
                                    );
                                }
                                TickAction::SkipIdle => {
                                    assert!(!old_would_render, "旧规则会渲染却整帧跳过了: {ctx}");
                                }
                                TickAction::SkipOccluded => {
                                    panic!("decide_tick_action 不该产出遮挡跳过: {ctx}")
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// 光标在别处移动时：本框不重绘，但必须仍推进一次共享高光插值。
    ///
    /// 少推进就会改变 `light_direction` 的插值序列，光标所在框的高光
    /// 会突然变亮约 3.8 倍 —— 那是肉眼可见的画面变化，不是优化。
    #[test]
    fn cursor_elsewhere_advances_light_without_rendering() {
        let action = decide_tick_action(
            true,  // 光标动了
            false, // 桌面没变
            GLASS_CURSOR_SETTLE_TICKS,
            0,
            false, // 光标与本框无关
        );
        assert_eq!(action, TickAction::AdvanceLightOnly);
        // 同样的输入，只要光标与本框有关就必须重绘
        assert_eq!(
            decide_tick_action(true, false, GLASS_CURSOR_SETTLE_TICKS, 0, true),
            TickAction::Render(RenderReason::Cursor)
        );
    }

    /// 桌面变化与心跳到期时，即使光标与本框无关也**必须重绘**：
    /// 这两条是「背景真的变了」和「兜底自愈」，降级成只推进插值就会漏更新。
    /// 这里只断言"渲染了"，不钉死具体原因 —— 原因的优先级由
    /// `pick_render_reason` 自己的测试保证。
    #[test]
    fn desktop_change_and_heartbeat_win_over_cursor_filter() {
        // 桌面变了 + 光标在别处动 → 必须重绘
        assert!(
            matches!(
                decide_tick_action(true, true, GLASS_CURSOR_SETTLE_TICKS, 0, false),
                TickAction::Render(_)
            ),
            "桌面变化时必须重绘，不能降级成只推进插值"
        );
        // 桌面变了 + 光标没动 → 必须重绘
        assert!(
            matches!(
                decide_tick_action(false, true, GLASS_CURSOR_SETTLE_TICKS, 0, false),
                TickAction::Render(_)
            ),
            "桌面变化时必须重绘（光标没动也一样）"
        );
        // 只有心跳到期 → 必须重绘
        assert!(
            matches!(
                decide_tick_action(
                    false,
                    false,
                    GLASS_CURSOR_SETTLE_TICKS,
                    GLASS_FORCE_RENDER_TICKS,
                    false
                ),
                TickAction::Render(_)
            ),
            "心跳到期时必须重绘，这是漏更新的最后兜底"
        );
        // 心跳没到期、桌面没变、光标在别处 → 这才是唯一允许降级的组合
        assert_eq!(
            decide_tick_action(false, false, GLASS_CURSOR_SETTLE_TICKS, 0, false),
            TickAction::SkipIdle,
            "四条都不成立时必须整帧跳过"
        );
    }

    /// 四条都不成立时必须能整帧省掉 —— 这是脏矩形方案省渲染的落点。
    #[test]
    fn render_reason_is_none_when_nothing_changed() {
        assert_eq!(
            pick_render_reason(false, false, GLASS_CURSOR_SETTLE_TICKS, 0),
            None
        );
        assert_eq!(
            pick_render_reason(
                false,
                false,
                GLASS_CURSOR_SETTLE_TICKS,
                GLASS_FORCE_RENDER_TICKS - 1
            ),
            None
        );
    }

    /// 静止桌面上渲染次数必须只由心跳决定：5 个框、10 秒心跳、30 秒窗口
    /// = 15 次。这条是把实测数字钉成断言，回归时一眼就能看出退化。
    #[test]
    fn idle_desktop_renders_only_heartbeat() {
        const BOXES: u64 = 5;
        const WINDOW_SECS: u64 = 30;
        const TICK_MS: u64 = 40;
        let mut heartbeat = 0u64;
        for _ in 0..BOXES {
            let ticks = WINDOW_SECS * 1000 / TICK_MS;
            //  faithfully 复刻 decide_grid_refresh：计数器每 tick 先自增，
            //  渲染后由 note_grid_frame_rendered 清零
            let mut ticks_since_render = 0u32;
            for _ in 0..ticks {
                ticks_since_render = ticks_since_render.saturating_add(1);
                if pick_render_reason(false, false, GLASS_CURSOR_SETTLE_TICKS, ticks_since_render)
                    == Some(RenderReason::Heartbeat)
                {
                    heartbeat += 1;
                    ticks_since_render = 0;
                }
            }
        }
        assert_eq!(
            heartbeat,
            BOXES * (WINDOW_SECS * 1000 / TICK_MS) / GLASS_FORCE_RENDER_TICKS as u64,
            "静止桌面的渲染次数应当正好等于心跳次数"
        );
    }

    #[test]
    fn glass_shadow_padding_preserves_content_rect_at_96_dpi() {
        let outer = RECT {
            left: 72,
            top: 172,
            right: 728,
            bottom: 828,
        };
        let content = content_rect_with_padding(outer, glass_shadow_padding_at_dpi(96));

        // padding = 48 逻辑像素（2026-09-14 收缩：参照帧 alpha 剖面
        // 证明 96 物理像素之外的外投影 alpha ≤ 1/255 不可见）。
        assert_eq!(content.left, 120);
        assert_eq!(content.top, 220);
        assert_eq!(content.right, 680);
        assert_eq!(content.bottom, 780);
        assert_eq!(client_to_content_point_with_padding(48, 48, 48), (0, 0));
        assert_eq!(
            client_to_content_point_with_padding(48, 448, 448),
            (400, 400)
        );
    }

    #[test]
    fn glass_shadow_padding_scales_with_dpi() {
        assert_eq!(glass_shadow_padding_at_dpi(96), 48);
        assert_eq!(glass_shadow_padding_at_dpi(144), 72);
        assert_eq!(glass_shadow_padding_at_dpi(192), 96);
        let content = content_rect_with_padding(
            RECT {
                left: -92,
                top: 8,
                right: 692,
                bottom: 792,
            },
            glass_shadow_padding_at_dpi(144),
        );

        assert_eq!(content.left, -20);
        assert_eq!(content.top, 80);
        assert_eq!(content.right, 620);
        assert_eq!(content.bottom, 720);
        assert_eq!(client_to_content_point_with_padding(72, 72, 72), (0, 0));
    }

    /// 液态玻璃关闭时不得预留任何外投影留白。
    ///
    /// 留白只服务于玻璃着色器画到内容矩形之外的外投影；关闭后继续保留会把
    /// 每帧 DIB、DWM 重定向表面和命中测试区域一起放大到内容面积的 3 倍以上
    /// （200% DPI 下 5 个分类框每帧 23MB → 7MB）。
    #[test]
    fn shadow_padding_follows_liquid_glass_switch() {
        let enabled = super::super::liquid_glass::get_liquid_glass_config().enabled;
        for dpi in [96u32, 120, 144, 192] {
            if enabled {
                assert_eq!(
                    shadow_padding_for_dpi(dpi),
                    glass_shadow_padding_at_dpi(dpi),
                    "玻璃开启时留白应等于 DPI 换算值"
                );
            } else {
                assert_eq!(shadow_padding_for_dpi(dpi), 0, "玻璃关闭时留白必须为 0");
            }
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
    use super::{LIQUID_GLASS_DRAG_INTERVAL_MS, LIQUID_GLASS_RENDER_INTERVAL_MS};

    /// 验证液态玻璃渲染定时器间隔已从 33ms 提升到 100ms
    /// 静止档 40ms = 25fps：用户要求的液态玻璃背景帧率下限；
    /// 拖拽档另有 16ms 高频路径。真正的省电手段是心跳
    /// GLASS_FORCE_RENDER_TICKS=250 tick（10 秒），而不是拉长档位间隔。
    #[test]
    fn test_liquid_glass_render_interval() {
        // 静止档 40ms = 25fps：用户要求的液态玻璃背景帧率下限。
        assert_eq!(
            LIQUID_GLASS_RENDER_INTERVAL_MS, 40,
            "静止档渲染间隔应为 40ms（25fps 下限）"
        );
    }

    /// 验证渲染间隔不低于合理下限（避免过于频繁的渲染）
    #[test]
    fn test_liquid_glass_render_interval_minimum() {
        assert!(
            LIQUID_GLASS_RENDER_INTERVAL_MS >= 33,
            "静止档不应快于 ~30fps，避免空转 GPU"
        );
    }

    /// 验证渲染间隔不超过合理上限（避免明显卡顿）
    #[test]
    fn test_liquid_glass_render_interval_maximum() {
        assert!(
            LIQUID_GLASS_RENDER_INTERVAL_MS <= 200,
            "渲染间隔不应超过 200ms，避免视觉卡顿"
        );
    }

    /// 拖拽档必须显著快于静止档，否则「拖拽走高频路径」等于没做。
    #[test]
    fn drag_interval_is_faster_than_idle_interval() {
        assert!(
            LIQUID_GLASS_DRAG_INTERVAL_MS < LIQUID_GLASS_RENDER_INTERVAL_MS,
            "拖拽档 {}ms 必须小于静止档 {}ms",
            LIQUID_GLASS_DRAG_INTERVAL_MS,
            LIQUID_GLASS_RENDER_INTERVAL_MS
        );
    }

    /// 拖拽档必须能支撑「至少 25 帧」的下限。
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
            "拖拽档最坏情况 {fps:.1}fps 低于 25fps 下限（间隔 {worst_case_ms:.1}ms）"
        );
    }

    /// 拖拽档不能快到把定时器打成忙等：请求值不低于 8ms。
    #[test]
    fn drag_interval_is_not_busy_loop() {
        assert!(
            LIQUID_GLASS_DRAG_INTERVAL_MS >= 8,
            "拖拽档 {}ms 过短，会退化成忙等",
            LIQUID_GLASS_DRAG_INTERVAL_MS
        );
    }
}
