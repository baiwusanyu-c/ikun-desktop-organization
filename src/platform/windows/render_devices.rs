//! 渲染线程内共享的 Direct2D / DirectWrite / WIC 工厂与文字格式缓存。
//!
//! # 为什么必须共享
//!
//! Direct2D 1.1 起，每个 `ID2D1Factory` 内部都持有一个**独立的 D3D11 设备**。
//! 旧实现给每个分类框、桌面覆盖窗口、右键菜单、子菜单、对齐参考线和拖拽覆盖
//! 位图各自 `D2D1CreateFactory` 一次，进程内因此出现近十个 D3D11 设备；实测每
//! 个额外设备连同显卡用户态驱动的私有分配约占用 50MB，5 个分类框就把主进程
//! 私有内存从 18MB 抬到 299MB。
//!
//! 本模块把三个工厂收敛为**每个渲染线程各一份**。之所以不是全进程一份：
//! `D2D1_FACTORY_TYPE_SINGLE_THREADED` 工厂及其派生资源有线程亲和性，跨线程
//! 使用会被 D2D 运行期拒绝；按线程缓存既能拿到全部内存收益（分类框、覆盖窗口、
//! 菜单、参考线都在同一条平台渲染线程上创建），又完整保留了改造前的线程语义。
//!
//! `ID2D1DCRenderTarget` 仍按窗口创建（由共享工厂派生），因为它需要与各窗口
//! 自己的 DIB 绑定，多个窗口共用一个 target 会在并发 `BindDC` 时互相踩画面。
//!
//! # 文字格式缓存
//!
//! `IDWriteTextFormat` 只取决于「字体族 + 字号 + 字重 + 样式 + 对齐 + 换行」。
//! 旧实现每帧为标题、锁定按钮、更多按钮和**每个图标标签**都重新创建一次，
//! 分类框定时器每秒产生上百次 DirectWrite 对象分配。这里按完整属性组合缓存，
//! 缓存命中后返回的对象**不可再修改**（会被同线程的多处渲染共享）。

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateDevice, ID2D1Device, ID2D1DeviceContext, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice2, IDCompositionDesktopDevice,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteInlineObject, IDWriteTextFormat, IDWriteTextLayout,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE, DWRITE_FONT_WEIGHT,
    DWRITE_PARAGRAPH_ALIGNMENT, DWRITE_TEXT_ALIGNMENT, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIFactory2};
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

/// 文字格式缓存键。
///
/// 字号放大 100 倍取整存储，避免浮点数参与哈希；其余字段直接取
/// DirectWrite 枚举的底层整数值。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TextFormatSpec {
    /// 字体族名称（英文常量，避免运行期分配）
    pub family: &'static str,
    /// 字号 × 100
    pub size_centi: u32,
    /// `DWRITE_FONT_WEIGHT.0`
    pub weight: i32,
    /// `DWRITE_FONT_STYLE.0`
    pub style: i32,
    /// `DWRITE_TEXT_ALIGNMENT.0`
    pub text_align: i32,
    /// `DWRITE_PARAGRAPH_ALIGNMENT.0`
    pub para_align: i32,
    /// `DWRITE_WORD_WRAPPING.0`
    pub word_wrapping: i32,
}

impl TextFormatSpec {
    /// 由 DirectWrite 枚举构造缓存键。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        family: &'static str,
        size: f32,
        weight: DWRITE_FONT_WEIGHT,
        style: DWRITE_FONT_STYLE,
        text_align: DWRITE_TEXT_ALIGNMENT,
        para_align: DWRITE_PARAGRAPH_ALIGNMENT,
        word_wrapping: DWRITE_WORD_WRAPPING,
    ) -> Self {
        Self {
            family,
            // 字号最小步进为 0.01，量化后不会丢失实际使用到的精度
            size_centi: (size * 100.0).round() as u32,
            weight: weight.0,
            style: style.0,
            text_align: text_align.0,
            para_align: para_align.0,
            word_wrapping: word_wrapping.0,
        }
    }

    /// 还原真实字号（DirectWrite 需要 f32）。
    fn size(&self) -> f32 {
        self.size_centi as f32 / 100.0
    }
}

// 所有共享状态都是线程本地的：`ID2D1Factory` 有线程亲和性，缓存本身也不需要
// 跨线程可见，因此用 `RefCell` 而不是 `Mutex`，既省一次锁开销也不会锁中毒。
/// 纯色画刷缓存上限，见 `solid_brush_rgba`。
const SOLID_BRUSH_CACHE_LIMIT: usize = 512;

thread_local! {
    /// (渲染目标裸指针, 打包 RGBA) → 画刷
    static SOLID_BRUSHES: RefCell<HashMap<(usize, u32), ID2D1SolidColorBrush>> =
        RefCell::new(HashMap::new());
    static D2D_FACTORY: RefCell<Option<ID2D1Factory>> = RefCell::new(None);
    static DW_FACTORY: RefCell<Option<IDWriteFactory>> = RefCell::new(None);
    static WIC_FACTORY: RefCell<Option<IWICImagingFactory>> = RefCell::new(None);
    static TEXT_FORMATS: RefCell<HashMap<TextFormatSpec, IDWriteTextFormat>> =
        RefCell::new(HashMap::new());
    static ELLIPSIS_SIGNS: RefCell<HashMap<TextFormatSpec, IDWriteInlineObject>> =
        RefCell::new(HashMap::new());
    /// 文字布局缓存。用 Vec + 线性扫描而不是 HashMap：
    /// 条目数天然收敛到几百条，而命中时能直接比较借用的 `&[u16]`，
    /// 不用为了查一次而先克隆一个 owned key。
    static TEXT_LAYOUTS: RefCell<Vec<TextLayoutEntry>> = RefCell::new(Vec::new());
    /// 渲染线程共享的 GPU 设备组（见 `SharedGpu`）
    static SHARED_GPU: RefCell<Option<SharedGpu>> = const { RefCell::new(None) };
    /// 渲染线程共享的 DirectComposition 设备。
    ///
    /// 必须是 `IDCompositionDesktopDevice` 而不是 `IDCompositionDevice2`：
    /// `CreateTargetForHwnd` 只在前者上，拿 v2 接口编译期就找不到方法。
    static DCOMP_DEVICE: RefCell<Option<IDCompositionDesktopDevice>> = const { RefCell::new(None) };
}

/// 取（必要时创建）当前渲染线程共享的 DirectComposition 设备。
///
/// 建在**共享的那个** D3D11 设备上（`shared_gpu().dxgi_device`）：
/// DComp 视觉树里的合成交换链必须与渲染它的 D3D11 设备同源，
/// 否则 `Present` 会失败或退化成一次跨设备拷贝。
///
/// 返回 `None` 表示这台机器/这次会话用不了 DComp（远程桌面、
/// 极旧系统等），调用方必须退回 `WS_EX_LAYERED` + `UpdateLayeredWindow`。
pub(crate) fn dcomp_device() -> Option<IDCompositionDesktopDevice> {
    if let Some(existing) = DCOMP_DEVICE.with(|cell| cell.borrow().clone()) {
        return Some(existing);
    }
    let gpu = shared_gpu()?;
    unsafe {
        // windows 0.62 的签名是 `DCompositionCreateDevice2<P0, T>(renderingdevice) -> Result<T>`，
        // IID 与出参由泛型 T 推导，不要手写 &mut raw。
        let device: IDCompositionDesktopDevice = match DCompositionCreateDevice2(&gpu.dxgi_device) {
            Ok(device) => device,
            Err(error) => {
                log::error!("创建 DirectComposition 设备失败，分类框退回分层窗口呈现: {error:?}");
                return None;
            }
        };
        log::info!("线程共享 DirectComposition 设备已创建");
        DCOMP_DEVICE.with(|cell| *cell.borrow_mut() = Some(device.clone()));
        Some(device)
    }
}

/// 渲染线程内共享的一组 GPU 设备。
///
/// # 为什么全线程必须只有一个 D3D11 设备
///
/// 旧实现里进程内同时存在**两个** D3D11 设备：一个由
/// 由渲染线程按需创建（历史：自研液态玻璃设备；现在仅内容层使用），
/// 另一个由 `D2D1CreateFactory` 内部隐式创建（分类框合成用的 DC 渲染目标）。
///
/// D3D11 纹理**不能跨设备直接共享**，于是玻璃每帧都要：
/// GPU 渲染 → `CopySubresourceRegion` 到 staging → `Map` 阻塞等 GPU 排空
/// → 逐行 memcpy 到 CPU → `CopyFromMemory` 上传给**另一个**设备 → 软件光栅。
/// 实测这条往返占单帧 20ms 里的 16ms，而真正的模糊 + 着色只有 0.44ms ——
/// 也就是说 97% 的时间花在搬像素，3% 花在画画。
///
/// 统一到一个设备之后，玻璃的输出纹理可以用
/// `ID2D1DeviceContext::CreateBitmapFromDxgiSurface` **零拷贝**包成 D2D 位图
/// 直接参与合成，整条 GPU→CPU→GPU 往返彻底消失。
///
/// 顺带也省掉一个设备本身：每个额外 D3D11 设备连同显卡用户态驱动的
/// 私有分配约占 50MB（见本模块开头对旧「每窗口一个工厂」的实测记录）。
///
/// # 为什么仍是线程本地而不是全进程一份
///
/// `ID3D11DeviceContext` 不是线程安全的（immidate context 明确要求单线程
/// 串行使用），而分类框、桌面覆盖、菜单、对齐参考线都在同一条平台渲染
/// 线程上创建。按线程缓存既拿到全部收益，又不引入跨线程同步。
#[derive(Clone)]
pub(crate) struct SharedGpu {
    pub device: ID3D11Device,
    pub dxgi_device: IDXGIDevice,
    /// 建 composition swap chain 用（`CreateSwapChainForComposition` 需要 factory2）
    pub factory2: IDXGIFactory2,
    pub d2d_device: ID2D1Device,
}

/// 取（必要时创建）当前渲染线程共享的 GPU 设备组。
///
/// 任一步失败都记日志并返回 `None`，由调用方退回原有路径；
/// 绝不 panic —— 建不出设备只意味着玻璃不可用，不该让整个应用挂掉。
pub(crate) fn shared_gpu() -> Option<SharedGpu> {
    if let Some(existing) = SHARED_GPU.with(|cell| cell.borrow().clone()) {
        return Some(existing);
    }
    let built = unsafe { build_shared_gpu() };
    match built {
        Some(gpu) => {
            log::info!("线程共享 GPU 设备组已创建（D3D11 + DXGI + D2D1 单一设备）");
            SHARED_GPU.with(|cell| *cell.borrow_mut() = Some(gpu.clone()));
            Some(gpu)
        }
        None => {
            log::error!("创建线程共享 GPU 设备组失败，液态玻璃将退回半透明背景");
            None
        }
    }
}

/// 实际创建共享设备组。任一步失败返回 `None`（已就地记日志）。
unsafe fn build_shared_gpu() -> Option<SharedGpu> {
    let levels = [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0];
    let mut device: Option<ID3D11Device> = None;
    let mut chosen = D3D_FEATURE_LEVEL::default();
    if let Err(error) = D3D11CreateDevice(
        None,
        D3D_DRIVER_TYPE_HARDWARE,
        HMODULE::default(),
        // BGRA 支持是 D2D1 的硬性要求，缺了 CreateDeviceContext 会直接失败
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        Some(&levels),
        D3D11_SDK_VERSION,
        Some(&mut device),
        Some(&mut chosen),
        // 立即上下文无人消费，不再请求（原字段存而不用，属重构残留）。
        None,
    ) {
        log::error!("创建共享 D3D11 设备失败: {error:?}");
        return None;
    }
    let device = device?;

    let dxgi_device: IDXGIDevice = match device.cast() {
        Ok(d) => d,
        Err(error) => {
            log::error!("D3D11 设备转 IDXGIDevice 失败: {error:?}");
            return None;
        }
    };
    let adapter = match dxgi_device.GetAdapter() {
        Ok(a) => a,
        Err(error) => {
            log::error!("取 DXGI 适配器失败: {error:?}");
            return None;
        }
    };
    let factory2: IDXGIFactory2 = match adapter.GetParent() {
        Ok(f) => f,
        Err(error) => {
            log::error!("取 IDXGIFactory2 失败: {error:?}");
            return None;
        }
    };
    let d2d_device = match D2D1CreateDevice(&dxgi_device, None) {
        Ok(d) => d,
        Err(error) => {
            log::error!("从共享 D3D11 设备创建 D2D1 Device 失败: {error:?}");
            return None;
        }
    };
    Some(SharedGpu {
        device,
        dxgi_device,
        factory2,
        d2d_device,
    })
}

/// 在共享设备上创建一个新的 D2D1 设备上下文。
///
/// 每个窗口一个：`ID2D1DeviceContext` 同一时刻只能 `SetTarget` 一个目标，
/// 多窗口共用会在并发渲染时互相踩画面（与 DC 渲染目标必须按窗口持有
/// 是同一个理由）。上下文本身很轻，重的是它背后的设备 —— 那个是共享的。
pub(crate) fn new_d2d_context() -> Option<ID2D1DeviceContext> {
    let gpu = shared_gpu()?;
    match unsafe {
        gpu.d2d_device
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
    } {
        Ok(context) => Some(context),
        Err(error) => {
            log::error!("创建共享设备上的 D2D1 DeviceContext 失败: {error:?}");
            None
        }
    }
}

/// 获取（必要时创建）当前渲染线程共享的 Direct2D 工厂。
pub(crate) fn d2d_factory() -> Option<ID2D1Factory> {
    if let Some(existing) = D2D_FACTORY.with(|cell| cell.borrow().clone()) {
        return Some(existing);
    }
    match unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) } {
        Ok(factory) => {
            log::info!("线程共享 D2D Factory 已创建");
            D2D_FACTORY.with(|cell| *cell.borrow_mut() = Some(factory.clone()));
            Some(factory)
        }
        Err(error) => {
            log::error!("创建线程共享 D2D Factory 失败: {error:?}");
            None
        }
    }
}

/// 获取（必要时创建）当前渲染线程共享的 DirectWrite 工厂。
pub(crate) fn dw_factory() -> Option<IDWriteFactory> {
    if let Some(existing) = DW_FACTORY.with(|cell| cell.borrow().clone()) {
        return Some(existing);
    }
    match unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) } {
        Ok(factory) => {
            log::info!("线程共享 DirectWrite Factory 已创建");
            DW_FACTORY.with(|cell| *cell.borrow_mut() = Some(factory.clone()));
            Some(factory)
        }
        Err(error) => {
            log::error!("创建线程共享 DirectWrite Factory 失败: {error:?}");
            None
        }
    }
}

/// 获取（必要时创建）当前渲染线程共享的 WIC 工厂。
pub(crate) fn wic_factory() -> Option<IWICImagingFactory> {
    if let Some(existing) = WIC_FACTORY.with(|cell| cell.borrow().clone()) {
        return Some(existing);
    }
    match unsafe {
        CoCreateInstance::<_, IWICImagingFactory>(
            &CLSID_WICImagingFactory,
            None,
            CLSCTX_INPROC_SERVER,
        )
    } {
        Ok(factory) => {
            log::info!("线程共享 WIC Factory 已创建");
            WIC_FACTORY.with(|cell| *cell.borrow_mut() = Some(factory.clone()));
            Some(factory)
        }
        Err(error) => {
            log::error!("创建线程共享 WIC Factory 失败: {error:?}");
            None
        }
    }
}

/// 纯色画刷缓存键：(渲染目标裸指针, 打包后的 RGBA)。
///
/// 单独抽出来是为了可测：键算错会让两个不同颜色命中同一条目，
/// 画出来的东西就错了，而这种错误没有任何报错。
fn brush_cache_key(target_ptr: usize, r: u8, g: u8, b: u8, a: u8) -> (usize, u32) {
    (target_ptr, u32::from_be_bytes([r, g, b, a]))
}

/// 取（必要时创建并缓存）指定渲染目标上的纯色画刷。
///
/// # 为什么要缓存
///
/// `ID2D1SolidColorBrush` 是 COM 对象，每次 `CreateSolidColorBrush`
/// 都从 Windows 进程堆上分配。分类框一帧要在 16 处各建一个，
/// 加上每个图标标签的阴影与正文两把，5 个框 × 每秒 2.5 帧就是
/// **每秒七百多次 COM 对象的申请/归还**。
///
/// 这些对象都会释放，Rust 侧分配器实测活跃字节恒定在 6.4MB 一动不动，
/// 但进程私有字节仍以 ~1MB/分 爬升 —— 涨的是 Windows 堆为这类
/// 高频申请/归还保留的已提交容量。按「渲染目标 × 颜色」缓存之后，
/// 稳态下一次都不再创建。
///
/// # 为什么按渲染目标分开缓存
///
/// 画刷归属于创建它的渲染目标，跨目标复用会被 D2D 运行期拒绝。
pub(crate) fn solid_brush_rgba(
    target: &ID2D1DCRenderTarget,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> Option<ID2D1SolidColorBrush> {
    let key = brush_cache_key(target.as_raw() as usize, r, g, b, a);
    if let Some(hit) = SOLID_BRUSHES.with(|cell| cell.borrow().get(&key).cloned()) {
        return Some(hit);
    }
    let created =
        unsafe { target.CreateSolidColorBrush(&super::ffi_types::d2d_color(r, g, b, a), None) }
            .ok()?;
    SOLID_BRUSHES.with(|cell| {
        let mut cache = cell.borrow_mut();
        // 颜色绝大多数是编译期常量，正常十几个分类框也就一两百条。
        // 上限只防「配置里出现逐帧变化的动态颜色」导致无界增长，
        // 触顶直接清空重建，代价是下一帧重新创建十几把画刷。
        if cache.len() >= SOLID_BRUSH_CACHE_LIMIT {
            log::debug!("纯色画刷缓存触顶 {} 条，清空重建", cache.len());
            cache.clear();
        }
        cache.insert(key, created.clone());
    });
    Some(created)
}

/// 丢弃当前渲染线程缓存的全部纯色画刷。
///
/// 画刷归属于创建它的渲染目标，目标销毁后缓存里会留下悬空条目；
/// 而 COM 对象地址会被系统复用，新目标可能拿到与旧目标**相同的地址**，
/// 那时命中悬空条目就会画出错误内容甚至直接被 D2D 拒绝，而且没有任何报错。
///
/// 所有 DC 渲染目标都由 `create_dc_render_target` 创建，因此在新目标
/// 诞生时整体清空即可保证：任何地址被复用的那一刻，缓存里一定没有
/// 它的旧条目。
pub(crate) fn clear_solid_brush_cache() {
    SOLID_BRUSHES.with(|cell| cell.borrow_mut().clear());
}

/// 用共享工厂创建一个 GDI DC 渲染目标（预乘 alpha，BGRA）。
///
/// `dpi` 传 `0.0` 表示沿用系统 DPI（分类框与覆盖窗口的既有行为）；
/// 拖拽覆盖位图沿用旧的固定 96。渲染目标仍按窗口持有，
/// 避免多个窗口并发绑定同一个 target。
pub(crate) fn create_dc_render_target(dpi: f32) -> Option<ID2D1DCRenderTarget> {
    let factory = d2d_factory()?;
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: dpi,
        dpiY: dpi,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    match unsafe { factory.CreateDCRenderTarget(&props) } {
        Ok(target) => {
            // 新目标可能复用了已销毁目标的地址，先清掉旧画刷条目
            clear_solid_brush_cache();
            // 整套渲染代码都把 D2D 坐标当**物理像素**用（几何取自
            // GetWindowRect，字号自己乘 dpi_scale），这只有在渲染目标
            // DPI == 96 时才成立。这里把真实 DPI 打出来做一次性核对：
            // 若哪天不是 96，画面会整体缩放，而这条日志就是唯一线索。
            let mut dpi_x = 0.0f32;
            let mut dpi_y = 0.0f32;
            unsafe { target.GetDpi(&mut dpi_x, &mut dpi_y) };
            log::info!(
                "DC RenderTarget 已创建: 请求 dpi={dpi:.1} → 实际 dpiX={dpi_x:.1} dpiY={dpi_y:.1}",
                dpi = dpi
            );
            Some(target)
        }
        Err(error) => {
            log::error!("创建共享工厂的 DC RenderTarget 失败: {error:?}");
            None
        }
    }
}

fn build_text_format(spec: &TextFormatSpec) -> Option<IDWriteTextFormat> {
    let factory = dw_factory()?;
    let family: Vec<u16> = spec
        .family
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let locale: Vec<u16> = "zh-cn\0".encode_utf16().collect();
    let format = unsafe {
        factory
            .CreateTextFormat::<_, _, _>(
                windows::core::PCWSTR(family.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT(spec.weight),
                DWRITE_FONT_STYLE(spec.style),
                DWRITE_FONT_STRETCH_NORMAL,
                spec.size(),
                windows::core::PCWSTR(locale.as_ptr()),
            )
            .ok()?
    };
    // 所有可变属性在入缓存前一次性设定；命中缓存的对象不得再被修改。
    let _ = unsafe { format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT(spec.text_align)) };
    let _ = unsafe { format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT(spec.para_align)) };
    let _ = unsafe { format.SetWordWrapping(DWRITE_WORD_WRAPPING(spec.word_wrapping)) };
    Some(format)
}

/// 获取（必要时创建并缓存）指定属性的 DirectWrite 文字格式。
pub(crate) fn text_format(spec: &TextFormatSpec) -> Option<IDWriteTextFormat> {
    if let Some(cached) = TEXT_FORMATS.with(|cell| cell.borrow().get(spec).cloned()) {
        return Some(cached);
    }
    let created = build_text_format(spec)?;
    TEXT_FORMATS.with(|cell| cell.borrow_mut().insert(spec.clone(), created.clone()));
    Some(created)
}

/// 获取（必要时创建并缓存）与文字格式配套的字符级省略号裁剪对象。
pub(crate) fn ellipsis_sign(spec: &TextFormatSpec) -> Option<IDWriteInlineObject> {
    if let Some(cached) = ELLIPSIS_SIGNS.with(|cell| cell.borrow().get(spec).cloned()) {
        return Some(cached);
    }
    let format = text_format(spec)?;
    let factory = dw_factory()?;
    let sign = unsafe { factory.CreateEllipsisTrimmingSign(&format).ok()? };
    ELLIPSIS_SIGNS.with(|cell| cell.borrow_mut().insert(spec.clone(), sign.clone()));
    Some(sign)
}

/// 文字布局缓存条目。
///
/// 键 = 文本 × 文字格式 × 换行尺寸 × 对齐 × 是否省略号。
/// 尺寸乘 100 取整参与比较，避免浮点相等陷阱。
struct TextLayoutEntry {
    text: Vec<u16>,
    spec: TextFormatSpec,
    max_width_centi: i32,
    max_height_centi: i32,
    text_align: i32,
    para_align: i32,
    ellipsis: bool,
    layout: IDWriteTextLayout,
}

/// 文字布局缓存的容量上限。
///
/// 条目数天然收敛到「不同文本 × 不同换行宽度」，几十个分类框
/// 也就几百条。上限只是防极端情况（不停重命名、不停拖拽改宽度）
/// 下无界增长，触顶直接清空重来。
pub(crate) const TEXT_LAYOUT_CACHE_LIMIT: usize = 2048;

/// 获取（必要时创建并缓存）DirectWrite 文字布局。
///
/// 旧实现每帧为标题、锁定按钮、更多按钮和**每个图标标签**
/// 都重新 `CreateTextLayout` 一次。DirectWrite 每次都要跑完整的
/// 文本分析（脚本切分、字形映射、簇表、断行）并分配若干内部缓冲。
/// 开销随「分类框数 × 每框文字段数 × 帧率」相乘：10 个框就是
/// 每秒 400+ 次，是玻璃开启时堆已提交容量持续抬升的主因之一。
///
/// 布局对象在入缓存前就把对齐与省略号裁剪全部设定完毕，
/// 之后**不得再修改**（会被同线程多处渲染共享）。
///
/// 注意：`text_align` / `para_align` 不能从 `spec` 里推——标题的
/// 文字格式声明的是 NEAR，但布局上覆盖为 CENTER，两者不一致。
#[allow(clippy::too_many_arguments)]
pub(crate) fn text_layout(
    text: &[u16],
    spec: &TextFormatSpec,
    max_width: f32,
    max_height: f32,
    text_align: DWRITE_TEXT_ALIGNMENT,
    para_align: DWRITE_PARAGRAPH_ALIGNMENT,
    ellipsis: bool,
) -> Option<IDWriteTextLayout> {
    let w = (max_width * 100.0).round() as i32;
    let h = (max_height * 100.0).round() as i32;
    let hit = TEXT_LAYOUTS.with(|cell| {
        cell.borrow()
            .iter()
            .find(|entry| {
                entry.max_width_centi == w
                    && entry.max_height_centi == h
                    && entry.text_align == text_align.0
                    && entry.para_align == para_align.0
                    && entry.ellipsis == ellipsis
                    && entry.spec == *spec
                    && entry.text == text
            })
            .map(|entry| entry.layout.clone())
    });
    if let Some(layout) = hit {
        return Some(layout);
    }

    let format = text_format(spec)?;
    let factory = dw_factory()?;
    let layout = unsafe {
        factory
            .CreateTextLayout(text, &format, max_width, max_height)
            .ok()?
    };
    let _ = unsafe { layout.SetTextAlignment(text_align) };
    let _ = unsafe { layout.SetParagraphAlignment(para_align) };
    if ellipsis {
        if let Some(sign) = ellipsis_sign(spec) {
            let trimming = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            let _ = unsafe { layout.SetTrimming(&trimming, &sign) };
        }
    }

    TEXT_LAYOUTS.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if borrow.len() >= TEXT_LAYOUT_CACHE_LIMIT {
            log::debug!("文字布局缓存触顶 {} 条，清空重建", borrow.len());
            borrow.clear();
        }
        borrow.push(TextLayoutEntry {
            text: text.to_vec(),
            spec: spec.clone(),
            max_width_centi: w,
            max_height_centi: h,
            text_align: text_align.0,
            para_align: para_align.0,
            ellipsis,
            layout: layout.clone(),
        });
    });
    Some(layout)
}

/// 当前渲染线程三级文字缓存的条目数（只读诊断，供 `mem_probe` 输出）。
///
/// 返回 (文字格式, 省略号裁剪对象, 文字布局)。
///
/// 布局缓存是运行期唯一可能持续变大的文字缓存：键里含文本与换行尺寸，
/// 只要标签文字或列宽抖动就会不断产生新条目，直到 2048 条触顶清空，
/// 表现为内存缓慢爬升后突然掉一截。把条目数打进日志就能一眼看出
/// 是不是它在涨。
pub(crate) fn text_cache_sizes() -> (usize, usize, usize) {
    (
        TEXT_FORMATS.with(|cell| cell.borrow().len()),
        ELLIPSIS_SIGNS.with(|cell| cell.borrow().len()),
        TEXT_LAYOUTS.with(|cell| cell.borrow().len()),
    )
}

/// 清空当前线程的文字格式缓存（仅测试使用）。
///
/// 运行期缓存条目由「字体族 × 字号 × 对齐」组合天然收敛，分类框最多产生十余条。
#[cfg(test)]
pub(crate) fn clear_text_format_cache() {
    TEXT_FORMATS.with(|cell| cell.borrow_mut().clear());
    ELLIPSIS_SIGNS.with(|cell| cell.borrow_mut().clear());
    TEXT_LAYOUTS.with(|cell| cell.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 画刷缓存键必须同时区分渲染目标与颜色：
    /// 键碰撞会让两个不同颜色拿到同一把画刷，画面直接画错且无报错。
    #[test]
    fn brush_key_separates_target_and_color() {
        let a = brush_cache_key(0x1000, 255, 255, 255, 255);
        let b = brush_cache_key(0x1000, 255, 255, 255, 254);
        let c = brush_cache_key(0x2000, 255, 255, 255, 255);
        assert_ne!(a, b, "alpha 差 1 也必须是不同的键");
        assert_ne!(a, c, "不同渲染目标必须是不同的键");
        assert_eq!(
            a,
            brush_cache_key(0x1000, 255, 255, 255, 255),
            "同参数必须稳定"
        );
    }

    /// RGBA 四个分量都要进键，不能被移位吃掉。
    #[test]
    fn brush_key_uses_every_channel() {
        let base = brush_cache_key(1, 10, 20, 30, 40);
        assert_ne!(base, brush_cache_key(1, 20, 10, 30, 40), "红绿互换必须不同");
        assert_ne!(
            base,
            brush_cache_key(1, 10, 20, 40, 30),
            "蓝 alpha 互换必须不同"
        );
        assert_ne!(base, brush_cache_key(1, 40, 30, 20, 10), "整体反序必须不同");
    }
    use windows::Win32::Graphics::DirectWrite::{
        DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
        DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP,
        DWRITE_WORD_WRAPPING_WRAP,
    };

    fn spec(size: f32, weight: DWRITE_FONT_WEIGHT) -> TextFormatSpec {
        TextFormatSpec::new(
            "Microsoft YaHei UI",
            size,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            DWRITE_WORD_WRAPPING_WRAP,
        )
    }

    #[test]
    fn test_text_format_spec_quantizes_size() {
        let key = spec(19.5, DWRITE_FONT_WEIGHT_BOLD);
        assert_eq!(key.size_centi, 1950);
        assert!((key.size() - 19.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_text_format_spec_distinguishes_every_property() {
        let base = spec(20.0, DWRITE_FONT_WEIGHT_NORMAL);
        assert_ne!(
            base,
            spec(21.0, DWRITE_FONT_WEIGHT_NORMAL),
            "字号必须参与键"
        );
        assert_ne!(base, spec(20.0, DWRITE_FONT_WEIGHT_BOLD), "字重必须参与键");
        assert_ne!(
            base,
            TextFormatSpec::new(
                "Segoe UI Symbol",
                20.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
                DWRITE_WORD_WRAPPING_WRAP,
            ),
            "字体族必须参与键"
        );
        assert_ne!(
            base,
            TextFormatSpec::new(
                "Microsoft YaHei UI",
                20.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
                DWRITE_WORD_WRAPPING_WRAP,
            ),
            "水平对齐必须参与键"
        );
        assert_ne!(
            base,
            TextFormatSpec::new(
                "Microsoft YaHei UI",
                20.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                DWRITE_WORD_WRAPPING_WRAP,
            ),
            "段落对齐必须参与键"
        );
        assert_ne!(
            base,
            TextFormatSpec::new(
                "Microsoft YaHei UI",
                20.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
                DWRITE_WORD_WRAPPING_NO_WRAP,
            ),
            "换行模式必须参与键"
        );
    }

    #[test]
    fn test_text_format_cache_returns_same_object() {
        // 缓存命中路径必须返回同一个 COM 对象；无 DirectWrite 环境时允许
        // 创建失败，但两次调用的结果必须一致（要么都命中要么都失败）。
        let key = spec(15.0, DWRITE_FONT_WEIGHT_BOLD);
        let first = text_format(&key);
        let second = text_format(&key);
        assert_eq!(first.is_some(), second.is_some());
        if let (Some(a), Some(b)) = (&first, &second) {
            assert!(
                windows::core::Interface::as_raw(a) == windows::core::Interface::as_raw(b),
                "同一缓存键必须复用同一个 IDWriteTextFormat"
            );
        }
        clear_text_format_cache();
        // 清缓存后重新创建，仍然应成功（或同样失败），且不与旧对象同址
        let third = text_format(&key);
        assert_eq!(first.is_some(), third.is_some());
        clear_text_format_cache();
    }

    /// 文字布局缓存必须同键复用、异键分离。
    ///
    /// 键写错会直接画出错误文字（比如把带省略号的布局用到
    /// 不该裁剪的地方，或把标题的 CENTER 段落对齐用到标签上），
    /// 所以每一个键分量都要单独验证。
    #[test]
    fn test_text_layout_cache_keys_are_exact() {
        let key = spec(13.0, DWRITE_FONT_WEIGHT_NORMAL);
        let text: Vec<u16> = "开发工具".encode_utf16().collect();
        let other: Vec<u16> = "系统工具".encode_utf16().collect();

        let base = text_layout(
            &text,
            &key,
            80.0,
            32.0,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            true,
        );
        let again = text_layout(
            &text,
            &key,
            80.0,
            32.0,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            true,
        );
        assert_eq!(base.is_some(), again.is_some());
        let (Some(a), Some(b)) = (&base, &again) else {
            // 无 DirectWrite 环境下允许创建失败，但两次必须一致
            clear_text_format_cache();
            return;
        };
        assert_eq!(
            windows::core::Interface::as_raw(a),
            windows::core::Interface::as_raw(b),
            "同一个键必须复用同一个 IDWriteTextLayout"
        );

        let differs = |t: &[u16],
                       w: f32,
                       h: f32,
                       ta: DWRITE_TEXT_ALIGNMENT,
                       pa: DWRITE_PARAGRAPH_ALIGNMENT,
                       el: bool| {
            let got = text_layout(t, &key, w, h, ta, pa, el);
            assert_eq!(got.is_some(), base.is_some());
            assert_ne!(
                got.map(|l| windows::core::Interface::as_raw(&l)),
                Some(windows::core::Interface::as_raw(a)),
                "键分量变了就必须是不同的布局对象"
            );
        };
        differs(
            &other,
            80.0,
            32.0,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            true,
        );
        differs(
            &text,
            81.0,
            32.0,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            true,
        );
        differs(
            &text,
            80.0,
            33.0,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            true,
        );
        differs(
            &text,
            80.0,
            32.0,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            true,
        );
        differs(
            &text,
            80.0,
            32.0,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
            true,
        );
        differs(
            &text,
            80.0,
            32.0,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            false,
        );

        // 字号也必须参与键（随 spec 一起）
        let bigger = spec(14.0, DWRITE_FONT_WEIGHT_NORMAL);
        let got = text_layout(
            &text,
            &bigger,
            80.0,
            32.0,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            true,
        );
        assert_ne!(
            got.map(|l| windows::core::Interface::as_raw(&l)),
            Some(windows::core::Interface::as_raw(a)),
            "字号不同必须是不同的布局"
        );
        clear_text_format_cache();
    }

    #[test]
    fn test_ellipsis_sign_follows_text_format_availability() {
        let key = spec(13.0, DWRITE_FONT_WEIGHT_NORMAL);
        let format_ok = text_format(&key).is_some();
        let sign_ok = ellipsis_sign(&key).is_some();
        assert!(
            !sign_ok || format_ok,
            "省略号对象依赖文字格式，格式不可用时不得凭空成功"
        );
        clear_text_format_cache();
    }

    #[test]
    fn test_shared_factories_are_stable_across_calls() {
        // 同一线程内重复获取必须拿到同一个实例，否则共享就失去意义
        let a = d2d_factory();
        let b = d2d_factory();
        assert_eq!(a.is_some(), b.is_some());
        if let (Some(x), Some(y)) = (&a, &b) {
            assert_eq!(
                windows::core::Interface::as_raw(x),
                windows::core::Interface::as_raw(y),
                "D2D 工厂必须线程内唯一"
            );
        }
        let c = dw_factory();
        let d = dw_factory();
        if let (Some(x), Some(y)) = (&c, &d) {
            assert_eq!(
                windows::core::Interface::as_raw(x),
                windows::core::Interface::as_raw(y),
                "DirectWrite 工厂必须线程内唯一"
            );
        }
    }
}
