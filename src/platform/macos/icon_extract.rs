//! macOS 图标提取
//!
//! 对于 Alias 文件（Finder 快捷方式），先解析目标路径再提取图标；
//! 对于普通文件，直接使用 NSWorkspace iconForFile:。

use cocoa::base::{id, nil};
use cocoa::foundation::{NSString, NSURL};
use objc::{class, msg_send, sel, sel_impl};
use std::path::Path;

/// 从文件路径提取图标
///
/// - Alias 文件：解析目标后提取目标图标
/// - 普通文件/文件夹：使用 NSWorkspace iconForFile:
pub fn extract_icon(path: &Path) -> Option<id> {
    let path_str = path.to_str()?;

    // 尝试解析 alias 文件，获取目标路径
    let resolved_path = resolve_alias(path_str).unwrap_or_else(|| path_str.to_string());

    unsafe {
        let ns_path = NSString::alloc(nil).init_str(&resolved_path);
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let icon: id = msg_send![workspace, iconForFile: ns_path];

        if icon.is_null() {
            None
        } else {
            Some(icon)
        }
    }
}

/// 解析 Alias 文件，返回目标路径
fn resolve_alias(path_str: &str) -> Option<String> {
    unsafe {
        let url: id = msg_send![class!(NSURL), fileURLWithPath: NSString::alloc(nil).init_str(path_str)];
        if url.is_null() {
            return None;
        }

        // URLByResolvingAliasFileAtURL:options:error:
        // kBookmarkResolutionWithoutUI = 1 << 8
        // kBookmarkResolutionWithoutMounting = 1 << 9
        // kBookmarkResolutionSecurityScope = 1 << 10
        let options: u64 = (1 << 8) | (1 << 9);
        let resolved_url: id = msg_send![
            class!(NSURL),
            URLByResolvingAliasFileAtURL: url
            options: options
            error: nil as id
        ];

        if resolved_url.is_null() {
            return None;
        }

        // 获取 resolved URL 的路径
        let resolved_path: id = msg_send![resolved_url, path];
        if resolved_path.is_null() {
            return None;
        }

        let utf8: *const std::os::raw::c_char = msg_send![resolved_path, UTF8String];
        if utf8.is_null() {
            return None;
        }

        let resolved = std::ffi::CStr::from_ptr(utf8)
            .to_string_lossy()
            .into_owned();

        if resolved.is_empty() || resolved == path_str {
            None
        } else {
            log::debug!("解析 alias: {} -> {}", path_str, resolved);
            Some(resolved)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_icon_app() {
        let path = Path::new("/Applications/Safari.app");
        let icon = extract_icon(path);
        assert!(icon.is_some());
    }

    #[test]
    fn test_resolve_alias_non_alias() {
        // 非 alias 文件应返回 None
        let result = resolve_alias("/Applications/Safari.app");
        assert!(result.is_none());
    }
}
