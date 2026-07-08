//! macOS FFI 绑定
//!
//! 提供 Core Foundation、Core Graphics、Cocoa 等原生 API 的类型和函数声明。

// =============================================================================
// Core Foundation 类型重导出
// =============================================================================

pub use core_foundation::base::{CFRelease, CFTypeRef};
pub use core_foundation::string::CFString;

// =============================================================================
// Core Graphics 类型重导出
// =============================================================================

pub use core_graphics::base::CGFloat;
pub use core_graphics::color_space::CGColorSpace;
pub use core_graphics::context::CGContext;
pub use core_graphics::geometry::{CGPoint, CGRect, CGSize};
pub use core_graphics::image::CGImage;

// =============================================================================
// Cocoa 类型重导出
// =============================================================================

pub use cocoa::appkit::{NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
pub use cocoa::base::{id, nil, BOOL, NO, YES};
pub use cocoa::foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

// =============================================================================
// Objective-C 运行时
// =============================================================================

pub use objc::runtime::{Class, Object, Sel};
pub use objc::{class, msg_send, sel, sel_impl};

// =============================================================================
// ForeignType trait (core_graphics 需要)
// =============================================================================

pub use foreign_types_shared::ForeignType;

// =============================================================================
// NSWindow Level 常量
// =============================================================================

/// 桌面图标层级（Finder 桌面图标使用的层级）
pub const NS_WINDOW_LEVEL_DESKTOP_ICON: i64 = 20;

/// 桌面壁纸层级
pub const NS_WINDOW_LEVEL_DESKTOP: i64 = -2147483603;

/// 普通窗口层级
pub const NS_WINDOW_LEVEL_NORMAL: i64 = 0;

/// 浮动窗口层级（分类框使用）
pub const NS_WINDOW_LEVEL_FLOATING: i64 = 3;

// =============================================================================
// NSWindow 样式常量补充
// =============================================================================

/// 无边框窗口（补充 cocoa crate 未提供的常量）
pub const NS_BORDERLESS_WINDOW_MASK: u64 = 0;

/// 非激活面板（窗口不会成为 key window）
pub const NS_NONACTIVATING_PANEL: u64 = 128;

// =============================================================================
// CGImage 相关常量
// =============================================================================

/// 预乘 Alpha（渲染图标时使用）
pub const K_CG_IMAGE_ALPHA_PREMULTIPLIED_LAST: u32 = 3;

/// RGB + Alpha 位图信息
pub const K_CG_BITMAP_BYTE_ORDER_DEFAULT: u32 = 0;

// =============================================================================
// 辅助函数
// =============================================================================

/// 将 Rust &str 转换为 NSString
pub fn str_to_nsstring(s: &str) -> id {
    unsafe { NSString::alloc(nil).init_str(s) }
}

/// 将 NSString 转换为 Rust String
pub fn nsstring_to_string(ns_string: id) -> String {
    unsafe {
        let c_str: *const std::os::raw::c_char = msg_send![ns_string, UTF8String];
        if c_str.is_null() {
            return String::new();
        }
        std::ffi::CStr::from_ptr(c_str)
            .to_string_lossy()
            .into_owned()
    }
}

/// 创建 NSRect
pub fn make_nsrect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_level_constants() {
        assert_eq!(NS_WINDOW_LEVEL_DESKTOP_ICON, 20);
        assert_eq!(NS_WINDOW_LEVEL_FLOATING, 3);
        assert_eq!(NS_WINDOW_LEVEL_NORMAL, 0);
    }

    #[test]
    fn test_make_nsrect() {
        let rect = make_nsrect(10.0, 20.0, 100.0, 200.0);
        assert_eq!(rect.origin.x, 10.0);
        assert_eq!(rect.origin.y, 20.0);
        assert_eq!(rect.size.width, 100.0);
        assert_eq!(rect.size.height, 200.0);
    }
}
