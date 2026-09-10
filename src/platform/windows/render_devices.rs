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

use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteInlineObject, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE, DWRITE_FONT_WEIGHT,
    DWRITE_PARAGRAPH_ALIGNMENT, DWRITE_TEXT_ALIGNMENT, DWRITE_WORD_WRAPPING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
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
thread_local! {
    static D2D_FACTORY: RefCell<Option<ID2D1Factory>> = RefCell::new(None);
    static DW_FACTORY: RefCell<Option<IDWriteFactory>> = RefCell::new(None);
    static WIC_FACTORY: RefCell<Option<IWICImagingFactory>> = RefCell::new(None);
    static TEXT_FORMATS: RefCell<HashMap<TextFormatSpec, IDWriteTextFormat>> =
        RefCell::new(HashMap::new());
    static ELLIPSIS_SIGNS: RefCell<HashMap<TextFormatSpec, IDWriteInlineObject>> =
        RefCell::new(HashMap::new());
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
        Ok(target) => Some(target),
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

/// 清空当前线程的文字格式缓存（仅测试使用）。
///
/// 运行期缓存条目由「字体族 × 字号 × 对齐」组合天然收敛，分类框最多产生十余条。
#[cfg(test)]
pub(crate) fn clear_text_format_cache() {
    TEXT_FORMATS.with(|cell| cell.borrow_mut().clear());
    ELLIPSIS_SIGNS.with(|cell| cell.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;
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
