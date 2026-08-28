//! Windows 平台桌面管理实现
//!
//! 使用文件系统读取桌面图标信息
//!
//! # 实现要点
//! - 桌面枚举：读取桌面文件夹中的文件
//! - 快捷方式检测：通过扩展名识别 .lnk 文件
//! - 快捷方式解析：使用 IShellLinkW 解析 .lnk 目标路径
//! - 虚拟 Shell 对象：回收站、此电脑等通过 CLSID 枚举
//! - 位置获取：由 DesktopIconMover 通过 SysListView32 提供

use crate::platform::types::DesktopIcon;
use crate::platform::DesktopManagerTrait;
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Windows 桌面管理器
pub struct DesktopManager;

impl Default for DesktopManager {
    fn default() -> Self {
        Self
    }
}

impl DesktopManager {
    /// Create a new DesktopManager instance
    pub fn new() -> Result<Self> {
        log::info!("Initializing Windows DesktopManager...");
        Ok(Self)
    }
}

impl DesktopManagerTrait for DesktopManager {
    fn get_desktop_icons(&self) -> Vec<DesktopIcon> {
        log::info!("Enumerating Windows desktop icons...");
        self.get_desktop_icons_internal().unwrap_or_else(|e| {
            log::warn!("Failed to get desktop icons: {}, returning empty list", e);
            Vec::new()
        })
    }

    fn watch_desktop(&self) -> Result<()> {
        log::info!("Starting desktop change monitoring...");
        Ok(())
    }

    fn extract_icon(&self, path: &str) -> anyhow::Result<crate::platform::types::IconData> {
        match super::icon_extract::extract_file_icon(path) {
            Some(info) => {
                let _ = info;
                anyhow::bail!("Pixel-level icon extraction not yet implemented via trait")
            }
            None => anyhow::bail!("Failed to extract icon for: {}", path),
        }
    }

    fn launch_file(&self, path: &str) -> anyhow::Result<()> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        let path_wide: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show_cmd: i32,
            ) -> isize;
        }

        let operation: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                path_wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };

        if result > 32 {
            Ok(())
        } else {
            anyhow::bail!("ShellExecuteW failed for '{}': error code {}", path, result)
        }
    }

    fn get_screen_resolution(&self) -> (i32, i32) {
        super::monitor_utils::get_screen_resolution()
    }

    fn get_primary_monitor_dpi(&self) -> u32 {
        super::monitor_utils::get_primary_monitor_dpi()
    }

    fn get_monitor_work_area(&self, _point: (i32, i32)) -> (i32, i32, i32, i32) {
        super::monitor_utils::get_primary_monitor_work_area_phys()
    }
}

/// 已知的 Windows 虚拟 Shell 对象
///
/// 这些对象出现在桌面 SysListView32 中但不是文件系统上的真实文件。
/// clsid_path 格式为 ::{CLSID}，可被 ShellExecuteW 打开。
struct VirtualShellObject {
    /// 显示名称
    display_name: &'static str,
    /// CLSID 路径，如 ::{645FF040-5081-101B-9F08-00AA002F954E}
    clsid_path: &'static str,
    /// 注册表键（用于检测该对象是否在桌面显示）
    /// 格式: (hive, subkey, value_name)
    registry_check: Option<(&'static str, &'static str, &'static str)>,
}

/// 常见的 Windows 桌面虚拟对象
const VIRTUAL_SHELL_OBJECTS: &[VirtualShellObject] = &[
    VirtualShellObject {
        display_name: "回收站",
        clsid_path: "::{645FF040-5081-101B-9F08-00AA002F954E}",
        registry_check: Some((
            "HKCU",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel",
            "{645FF040-5081-101B-9F08-00AA002F954E}",
        )),
    },
    VirtualShellObject {
        display_name: "此电脑",
        clsid_path: "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
        registry_check: Some((
            "HKCU",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel",
            "{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
        )),
    },
    VirtualShellObject {
        display_name: "网络",
        clsid_path: "::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}",
        registry_check: Some((
            "HKCU",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel",
            "{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}",
        )),
    },
    VirtualShellObject {
        display_name: "控制面板",
        clsid_path: "::{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}",
        registry_check: Some((
            "HKCU",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel",
            "{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}",
        )),
    },
];

/// 已知的 CLSID 路径到显示名称的映射
/// 根据 CLSID 路径获取虚拟 Shell 对象的显示名称
///
/// 使用大小写不敏感的比较，因为 CLSID 在不同场景下可能大小写不一致。
pub fn clsid_display_name(clsid_path: &str) -> Option<&'static str> {
    let clsid_lower = clsid_path.to_lowercase();
    VIRTUAL_SHELL_OBJECTS
        .iter()
        .find(|obj| obj.clsid_path.to_lowercase() == clsid_lower)
        .map(|obj| obj.display_name)
}

impl DesktopManager {
    /// 获取桌面文件夹路径
    fn get_desktop_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // 用户桌面（可无管理员权限管理图标隐藏/显示）
        if let Some(desktop) = dirs::desktop_dir() {
            paths.push(desktop);
        }

        paths
    }

    /// 获取所有桌面路径（含公共桌面，仅用于只读枚举）
    fn get_all_desktop_paths() -> Vec<PathBuf> {
        let mut paths = Self::get_desktop_paths();
        let public_desktop = PathBuf::from(r"C:\Users\Public\Desktop");
        if public_desktop.exists() {
            paths.push(public_desktop);
        }
        paths
    }

    /// 检查虚拟 Shell 对象是否在桌面可见
    ///
    /// Windows 通过注册表 HideDesktopIcons\NewStartPanel 控制虚拟对象显示。
    /// 如果值为 1，表示隐藏；如果值不存在或不为 1，表示显示。
    fn is_virtual_object_visible(obj: &VirtualShellObject) -> bool {
        if let Some((_hive, subkey, value_name)) = obj.registry_check {
            // 使用 Win32 API 读取注册表
            use std::ffi::OsStr;
            use std::os::windows::ffi::OsStrExt;

            extern "system" {
                fn RegOpenKeyExW(
                    hkey: isize,
                    lpSubKey: *const u16,
                    ulOptions: u32,
                    samDesired: u32,
                    phkResult: *mut isize,
                ) -> i32;
                fn RegQueryValueExW(
                    hkey: isize,
                    lpValueName: *const u16,
                    lpReserved: *mut u32,
                    lpType: *mut u32,
                    lpData: *mut u8,
                    lpcbData: *mut u32,
                ) -> i32;
                fn RegCloseKey(hkey: isize) -> i32;
            }

            const HKEY_CURRENT_USER: isize = 0x80000001u32 as isize;
            const KEY_READ: u32 = 0x20019;

            let subkey_wide: Vec<u16> = OsStr::new(subkey)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let value_wide: Vec<u16> = OsStr::new(value_name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let mut hkey: isize = 0;
            let result = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    subkey_wide.as_ptr(),
                    0,
                    KEY_READ,
                    &mut hkey,
                )
            };

            if result != 0 {
                // 注册表键不存在 → 默认显示
                return true;
            }

            let mut data: u32 = 0;
            let mut data_size: u32 = 4;
            let mut value_type: u32 = 0;
            let result = unsafe {
                RegQueryValueExW(
                    hkey,
                    value_wide.as_ptr(),
                    std::ptr::null_mut(),
                    &mut value_type,
                    &mut data as *mut u32 as *mut u8,
                    &mut data_size,
                )
            };

            unsafe { RegCloseKey(hkey) };

            if result != 0 {
                // 值不存在 → 默认显示
                return true;
            }

            // 值为 1 表示隐藏，其他值表示显示
            data != 1
        } else {
            // 没有注册表检查 → 默认显示
            true
        }
    }

    /// 枚举桌面虚拟 Shell 对象（回收站、此电脑等）
    fn enumerate_virtual_shell_objects() -> Vec<DesktopIcon> {
        let mut icons = Vec::new();

        for obj in VIRTUAL_SHELL_OBJECTS {
            if !Self::is_virtual_object_visible(obj) {
                log::debug!("虚拟对象 '{}' 已隐藏，跳过", obj.display_name);
                continue;
            }

            icons.push(DesktopIcon {
                id: uuid::Uuid::new_v4(),
                display_name: obj.display_name.to_string(),
                path: PathBuf::from(obj.clsid_path),
                target_path: None, // 虚拟对象没有文件目标
                icon_path: None,
                position: (0, 0),
                is_shortcut: false,
            });

            log::debug!("添加虚拟对象: {} -> {}", obj.display_name, obj.clsid_path);
        }

        icons
    }

    fn get_desktop_icons_internal(&self) -> Result<Vec<DesktopIcon>> {
        let desktop_paths = Self::get_all_desktop_paths();

        if desktop_paths.is_empty() {
            anyhow::bail!("无法获取桌面路径");
        }

        let mut icons = Vec::new();
        // 使用完整路径做去重 key，避免同名快捷方式在用户/公共桌面被误杀
        let mut seen_paths = std::collections::HashSet::new();

        for desktop_path in &desktop_paths {
            log::info!("扫描桌面路径: {}", desktop_path.display());

            if !desktop_path.exists() {
                continue;
            }

            for entry in std::fs::read_dir(desktop_path).context("无法读取桌面文件夹")? {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        log::warn!("读取桌面条目失败: {}", e);
                        continue;
                    }
                };

                let path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();

                // 跳过系统保留文件
                if file_name.starts_with('.') || file_name == "desktop.ini" {
                    continue;
                }

                let is_shortcut = path
                    .extension()
                    .map(|ext| ext == "lnk" || ext == "url")
                    .unwrap_or(false);

                // 显示名称（去掉扩展名，模拟 Windows 桌面显示）
                let display_name = if is_shortcut {
                    path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or(file_name.clone())
                } else if path.is_dir() {
                    file_name.clone()
                } else {
                    path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or(file_name.clone())
                };

                // 使用规范化完整路径去重（替代 display_name 去重）
                let path_key = path.to_string_lossy().replace('/', "\\").to_lowercase();
                if !seen_paths.insert(path_key) {
                    continue;
                }

                // 解析快捷方式目标路径
                let target_path = if is_shortcut {
                    let path_str = path.to_string_lossy();
                    let resolved =
                        super::icon_extract::resolve_shortcut_target(&path_str).map(PathBuf::from);
                    if let Some(ref target) = resolved {
                        log::debug!("快捷方式 '{}' -> 目标 '{}'", display_name, target.display());
                    }
                    resolved
                } else {
                    Some(path.clone())
                };

                log::debug!("桌面项目: {} -> {:?}", display_name, path);

                icons.push(DesktopIcon {
                    id: uuid::Uuid::new_v4(),
                    display_name,
                    path: path.clone(),
                    target_path,
                    icon_path: None,
                    position: (0, 0), // 位置由 DesktopIconMover 提供
                    is_shortcut,
                });
            }
        }

        // 枚举虚拟 Shell 对象（回收站、此电脑、网络等）
        let virtual_icons = Self::enumerate_virtual_shell_objects();
        for vicon in virtual_icons {
            let path_key = vicon.path.to_string_lossy().to_lowercase();
            if seen_paths.insert(path_key) {
                icons.push(vicon);
            }
        }

        // 从 SysListView32 枚举所有可见桌面图标，发现不在文件系统中的虚拟对象
        // （如企业微信联系人卡片等 Shell 命名空间扩展对象）

        log::info!("找到 {} 个桌面图标（含虚拟对象）", icons.len());
        Ok(icons)
    }
}

// ===== 单元测试 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shortcut_detection_logic() {
        let lnk_path = PathBuf::from("C:\\Users\\Desktop\\Test.lnk");
        let is_shortcut = lnk_path
            .extension()
            .map(|ext| ext == "lnk")
            .unwrap_or(false);
        assert!(is_shortcut, ".lnk 文件应被识别为快捷方式");

        let exe_path = PathBuf::from("C:\\Program Files\\App.exe");
        let is_shortcut = exe_path
            .extension()
            .map(|ext| ext == "lnk")
            .unwrap_or(false);
        assert!(!is_shortcut, ".exe 文件不应被识别为快捷方式");
    }

    #[test]
    fn test_virtual_shell_objects_have_valid_clsids() {
        for obj in VIRTUAL_SHELL_OBJECTS {
            assert!(
                obj.clsid_path.starts_with("::{"),
                "CLSID 路径应以 '::{{' 开头: {}",
                obj.clsid_path
            );
            assert!(
                obj.clsid_path.ends_with('}'),
                "CLSID 路径应以 '}}' 结尾: {}",
                obj.clsid_path
            );
            assert!(!obj.display_name.is_empty(), "显示名称不能为空");
        }
    }

    #[test]
    fn test_clsid_display_name_known_clsids() {
        // 回收站
        assert_eq!(
            clsid_display_name("::{645FF040-5081-101B-9F08-00AA002F954E}"),
            Some("回收站")
        );
        // 此电脑
        assert_eq!(
            clsid_display_name("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"),
            Some("此电脑")
        );
        // 网络
        assert_eq!(
            clsid_display_name("::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}"),
            Some("网络")
        );
        // 控制面板
        assert_eq!(
            clsid_display_name("::{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}"),
            Some("控制面板")
        );
    }

    #[test]
    fn test_clsid_display_name_case_insensitive() {
        // 小写 CLSID 应该也能匹配
        assert_eq!(
            clsid_display_name("::{645ff040-5081-101b-9f08-00aa002f954e}"),
            Some("回收站")
        );
        assert_eq!(
            clsid_display_name("::{20d04fe0-3aea-1069-a2d8-08002b30309d}"),
            Some("此电脑")
        );
        // 混合大小写
        assert_eq!(
            clsid_display_name("::{5399e694-6CE5-4d6c-8FCE-1d8870fdcba0}"),
            Some("控制面板")
        );
    }

    #[test]
    fn test_clsid_display_name_unknown() {
        // 未知 CLSID 返回 None
        assert_eq!(
            clsid_display_name("::{00000000-0000-0000-0000-000000000000}"),
            None
        );
        assert_eq!(
            clsid_display_name("::{DEADBEEF-DEAD-BEEF-DEAD-BEEFDEADBEEF}"),
            None
        );
    }

    #[test]
    fn test_clsid_display_name_non_clsid_path() {
        // 非 CLSID 路径返回 None
        assert_eq!(clsid_display_name("C:\\Users\\Desktop\\test.lnk"), None);
        assert_eq!(clsid_display_name("::{invalid}"), None);
        assert_eq!(clsid_display_name(""), None);
    }
}
