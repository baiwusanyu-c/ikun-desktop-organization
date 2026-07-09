//! macOS 图标提取
//!
//! 使用 NSWorkspace 提取文件/文件夹图标

use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use objc::{class, msg_send, sel, sel_impl};
use std::path::Path;

/// 从文件路径提取图标
pub fn extract_icon(path: &Path) -> Option<id> {
    unsafe {
        let path_str = NSString::alloc(nil).init_str(path.to_str()?);
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let icon: id = msg_send![workspace, iconForFile: path_str];

        if icon.is_null() {
            None
        } else {
            Some(icon)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_icon() {
        let path = Path::new("/Applications/Safari.app");
        let icon = extract_icon(path);
        assert!(icon.is_some());
    }
}
