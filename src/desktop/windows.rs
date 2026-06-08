//! Windows 平台桌面管理实现
//!
//! 使用 Windows Shell API 读取桌面图标信息
//!
//! # 实现要点
//! - COM 初始化：使用 `CoInitializeEx` 初始化 COM 环境
//! - 桌面枚举：使用 `IShellFolder` 和 `IEnumIDList` 枚举桌面项目
//! - 快捷方式解析：使用 `IShellLink` 解析 `.lnk` 文件
//! - 位置获取：从桌面文件夹视图中获取图标位置

use crate::desktop::{DesktopIcon, DesktopManagerTrait};
use anyhow::{Context, Result};
use std::path::PathBuf;

use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;

/// Windows 桌面管理器
///
/// 使用 Windows Shell API 实现桌面图标读取
pub struct DesktopManager {
    /// COM 是否已初始化
    com_initialized: bool,
}

impl DesktopManagerTrait for DesktopManager {
    fn new() -> Result<Self> {
        log::info!("初始化 Windows DesktopManager...");

        // 初始化 COM（使用 STA 模式，适合 GUI 应用）
        // COINIT_APARTMENTTHUNKED = STA (Single Threaded Apartment)
        CoInitializeEx(None, COINIT_APARTMENTTHUNKED).context("COM 初始化失败")?;

        log::info!("COM 初始化成功");

        Ok(Self {
            com_initialized: true,
        })
    }

    fn get_desktop_icons(&self) -> Vec<DesktopIcon> {
        log::info!("开始获取 Windows 桌面图标...");

        // 尝试获取图标，失败时返回空列表并记录警告
        self.get_desktop_icons_internal().unwrap_or_else(|e| {
            log::warn!("获取桌面图标失败: {}, 返回空列表", e);
            Vec::new()
        })
    }

    fn watch_desktop(&self) -> Result<()> {
        // TODO: 实现桌面变化监听
        // 使用 notify crate 监听桌面文件夹变化
        log::info!("开始监听桌面变化...");
        Ok(())
    }
}

impl DesktopManager {
    /// 内部方法：获取桌面图标
    ///
    /// 实现完整的 Windows Shell API 调用流程
    fn get_desktop_icons_internal(&self) -> Result<Vec<DesktopIcon>> {
        // 1. 获取桌面文件夹的 IShellFolder 接口
        let desktop_folder = self.get_desktop_folder().context("无法获取桌面文件夹")?;

        // 2. 枚举桌面项目
        let items = self
            .enumerate_desktop_items(&desktop_folder)
            .context("无法枚举桌面项目")?;

        log::info!("找到 {} 个桌面项目", items.len());

        // 3. 解析每个项目的信息
        let icons: Vec<DesktopIcon> = items
            .into_iter()
            .filter_map(|pidl| {
                // 解析单个项目，失败时记录警告并跳过
                self.parse_item_info(&desktop_folder, pidl)
                    .map_err(|e| {
                        log::warn!("解析项目失败: {}", e);
                        e
                    })
                    .ok()
            })
            .collect();

        log::info!("成功解析 {} 个桌面图标", icons.len());
        Ok(icons)
    }

    /// 获取桌面文件夹的 IShellFolder 接口
    fn get_desktop_folder(&self) -> Result<IShellFolder> {
        // SHGetDesktopFolder 返回桌面的 IShellFolder 接口
        let mut desktop_folder: Option<IShellFolder> = None;

        SHGetDesktopFolder(&mut desktop_folder).context("SHGetDesktopFolder 调用失败")?;

        desktop_folder
            .ok_or_else(|| anyhow::anyhow!("获取桌面文件夹失败：返回值为空"))
            .context("桌面文件夹接口获取失败")
    }

    /// 枚举桌面上的所有项目
    ///
    /// 返回 PIDL（Pointer to ID List）列表
    fn enumerate_desktop_items(&self, folder: &IShellFolder) -> Result<Vec<PIDL>> {
        // 创建枚举器
        // SHCONTF_FOLDERS | SHCONTF_NONFOLDERS = 枚举文件夹和非文件夹项目
        let mut enum_id_list: Option<IEnumIDList> = None;

        folder
            .EnumObjects(
                HWND(0), // 使用默认窗口
                SHCONTF_FOLDERS | SHCONTF_NONFOLDERS | SHCONTF_INCLUDEHIDDEN,
                &mut enum_id_list,
            )
            .context("无法创建桌面项目枚举器")?;

        let enum_id_list =
            enum_id_list.ok_or_else(|| anyhow::anyhow!("枚举器创建失败：返回值为空"))?;

        // 枚举所有项目
        let mut items: Vec<PIDL> = Vec::new();
        let mut pidl: PIDL = PIDL::default();
        let mut fetched: u32 = 0;

        // 循环获取每个项目
        loop {
            let result = enum_id_list.Next(1, &mut pidl, &mut fetched);

            // S_OK 表示成功获取，S_FALSE 表示枚举结束
            if result != S_OK || fetched == 0 {
                break;
            }

            items.push(pidl);
        }

        Ok(items)
    }

    /// 解析单个桌面项目的信息
    ///
    /// 从 PIDL 提取名称、路径、快捷方式目标等信息
    fn parse_item_info(&self, folder: &IShellFolder, pidl: PIDL) -> Result<DesktopIcon> {
        // 1. 获取显示名称
        let display_name = self
            .get_display_name(folder, pidl)
            .context("无法获取显示名称")?;

        // 2. 获取文件系统路径
        let path = self
            .get_file_path(folder, pidl)
            .context("无法获取文件路径")?;

        log::debug!("解析项目: {} -> {}", display_name, path.display());

        // 3. 判断是否是快捷方式（.lnk 文件）
        let is_shortcut = path.extension().map(|ext| ext == "lnk").unwrap_or(false);

        // 4. 解析快捷方式目标路径
        let target_path = if is_shortcut {
            self.parse_shortcut_target(&path)
                .context("无法解析快捷方式目标")
                .ok()
        } else {
            Some(path.clone())
        };

        // 5. 获取图标位置（从桌面视图）
        let position = self.get_icon_position(folder, pidl).unwrap_or((0, 0)); // 失败时使用默认位置

        Ok(DesktopIcon {
            id: uuid::Uuid::new_v4(),
            display_name,
            path,
            target_path,
            icon_path: None, // TODO: 提取图标图片
            position,
            is_shortcut,
        })
    }

    /// 获取项目的显示名称
    fn get_display_name(&self, folder: &IShellFolder, pidl: PIDL) -> Result<String> {
        let mut str_ret = STRRET::default();

        folder
            .GetDisplayNameOf(pidl, SHGDN_INFOLDER | SHGDN_FORPARSING, &mut str_ret)
            .context("GetDisplayNameOf 调用失败")?;

        // 从 STRRET 结构体中提取字符串
        // STRRET 有三种类型：STRRET_WSTR、STRRET_OFFSET、STRRET_CSTR
        match str_ret.uType {
            STRRET_WSTR => {
                // Unicode 字符串（pOleStr）
                // 注意：需要释放分配的内存
                if str_ret.pOleStr.is_null() {
                    return Err(anyhow::anyhow!("STRRET_WSTR: pOleStr 为空"));
                }
                unsafe {
                    let wide_str = windows::core::PCWSTR(str_ret.pOleStr);
                    let name = wide_str.to_string().context("无法转换 Unicode 字符串")?;
                    // 释放分配的内存（Shell 分配的字符串需要用 CoTaskMemFree 释放）
                    CoTaskMemFree(Some(str_ret.pOleStr.as_ptr() as *const _));
                    Ok(name)
                }
            }
            STRRET_CSTR => {
                // ANSI 字符串（cStr）
                let bytes: Vec<u8> = str_ret.cStr.iter().take_while(|&b| b != 0).collect();
                String::from_utf8(bytes).context("无法转换 ANSI 字符串")
            }
            STRRET_OFFSET => {
                // 基于偏移的字符串（通常不用于桌面项目）
                Err(anyhow::anyhow!("STRRET_OFFSET 类型不支持"))
            }
            _ => Err(anyhow::anyhow!("未知的 STRRET 类型: {}", str_ret.uType)),
        }
    }

    /// 获取项目的文件系统路径
    fn get_file_path(&self, folder: &IShellFolder, pidl: PIDL) -> Result<PathBuf> {
        // 使用 SHGDN_FORPARSING 获取可解析的路径
        let mut str_ret = STRRET::default();

        folder
            .GetDisplayNameOf(pidl, SHGDN_FORPARSING, &mut str_ret)
            .context("无法获取解析路径")?;

        // 提取路径字符串
        let path_str = match str_ret.uType {
            STRRET_WSTR => {
                if str_ret.pOleStr.is_null() {
                    return Err(anyhow::anyhow!("STRRET_WSTR: pOleStr 为空"));
                }
                unsafe {
                    let wide_str = windows::core::PCWSTR(str_ret.pOleStr);
                    let path = wide_str.to_string().context("无法转换路径字符串")?;
                    CoTaskMemFree(Some(str_ret.pOleStr.as_ptr() as *const _));
                    path
                }
            }
            STRRET_CSTR => {
                let bytes: Vec<u8> = str_ret.cStr.iter().take_while(|&b| b != 0).collect();
                String::from_utf8(bytes).context("无法转换路径字符串")?
            }
            _ => return Err(anyhow::anyhow!("不支持 STRRET 类型")),
        };

        Ok(PathBuf::from(path_str))
    }

    /// 解析快捷方式的目标路径
    ///
    /// 使用 IShellLink 接口解析 .lnk 文件
    fn parse_shortcut_target(&self, shortcut_path: &PathBuf) -> Result<PathBuf> {
        log::debug!("解析快捷方式: {}", shortcut_path.display());

        // 创建 IShellLink 对象
        let mut shell_link: Option<IShellLinkW> = None;

        CoCreateInstance(
            &CLSID_ShellLink as *const _ as *const _,
            None,
            CLSCTX_INPROC_SERVER,
            &IShellLinkW::IID as *const _ as *const _,
            &mut shell_link as *mut _ as *mut _,
        )
        .context("无法创建 IShellLink 对象")?;

        let shell_link = shell_link.ok_or_else(|| anyhow::anyhow!("IShellLink 创建失败"))?;

        // 加载快捷方式文件
        let path_wide: Vec<u16> = shortcut_path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let path_pcwstr = windows::core::PCWSTR(path_wide.as_ptr());

        shell_link
            .Load(path_pcwstr, STGM_READ)
            .context("无法加载快捷方式文件")?;

        // 获取目标路径
        let mut target_path_buf = [0u16; windows::Win32::Storage::FileSystem::MAX_PATH as usize];
        let target_pcwstr = windows::core::PWSTR(target_path_buf.as_mut_ptr());

        shell_link
            .GetPath(
                target_pcwstr,
                target_path_buf.len() as i32,
                None,
                SLGP_SHORTPATH,
            )
            .context("无法获取快捷方式目标路径")?;

        // 转换为字符串
        let target_wide = windows::core::PCWSTR(target_path_buf.as_ptr());
        let target_str = target_wide.to_string().context("无法转换目标路径字符串")?;

        if target_str.is_empty() {
            return Err(anyhow::anyhow!("快捷方式目标路径为空"));
        }

        log::debug!("快捷方式目标: {}", target_str);
        Ok(PathBuf::from(target_str))
    }

    /// 获取图标在桌面上的位置
    ///
    /// 尝试从桌面视图获取图标位置
    fn get_icon_position(&self, _folder: &IShellFolder, _pidl: PIDL) -> Result<(i32, i32)> {
        // TODO: 实现图标位置获取
        // 这需要获取 IShellView 并调用 GetItemPosition
        // 或者读取注册表中的位置信息
        //
        // 暂时返回默认位置 (0, 0)
        // 实际实现需要：
        // 1. 获取桌面窗口的 IShellView
        // 2. 调用 GetItemPosition 获取每个图标的位置
        // 3. 处理多显示器情况

        // 当前返回模拟位置（后续完善）
        Ok((100, 100))
    }
}

impl Drop for DesktopManager {
    fn drop(&mut self) {
        if self.com_initialized {
            // 释放 COM
            CoUninitialize();
            log::info!("COM 已释放");
        }
    }
}

// ===== 单元测试 =====

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试 COM 初始化和清理
    #[test]
    #[cfg(windows)]
    fn test_com_initialization() {
        // 测试初始化
        let manager = DesktopManager::new();
        assert!(manager.is_ok(), "COM 初始化应该成功");

        let manager = manager.unwrap();
        assert!(manager.com_initialized, "COM 应标记为已初始化");

        // Drop 会自动调用 CoUninitialize
    }

    /// 测试桌面图标枚举
    #[test]
    #[cfg(windows)]
    fn test_get_desktop_icons() {
        let manager = DesktopManager::new().expect("初始化失败");

        let icons = manager.get_desktop_icons();

        // 桌面应该至少有一些图标（除非是全新的 Windows）
        // 这个测试可能在不同环境下表现不同
        log::info!("找到 {} 个桌面图标", icons.len());

        // 验证图标数据结构
        for icon in &icons {
            assert!(!icon.display_name.is_empty(), "图标名称不应为空");
            assert!(!icon.path.as_os_str().is_empty(), "图标路径不应为空");
        }
    }

    /// 测试快捷方式解析逻辑（跨平台）
    #[test]
    fn test_shortcut_detection_logic() {
        // 测试 .lnk 文件检测逻辑
        let lnk_path = PathBuf::from("C:\\Users\\Desktop\\Test.lnk");
        let is_shortcut = lnk_path
            .extension()
            .map(|ext| ext == "lnk")
            .unwrap_or(false);
        assert!(is_shortcut, ".lnk 文件应被识别为快捷方式");

        // 测试非快捷方式
        let exe_path = PathBuf::from("C:\\Program Files\\App.exe");
        let is_shortcut = exe_path
            .extension()
            .map(|ext| ext == "lnk")
            .unwrap_or(false);
        assert!(!is_shortcut, ".exe 文件不应被识别为快捷方式");
    }

    /// 测试错误处理
    #[test]
    #[cfg(windows)]
    fn test_error_handling() {
        // 测试初始化两次（应该失败）
        let manager1 = DesktopManager::new().expect("第一次初始化应该成功");

        // COM 在同一线程重复初始化应该返回 S_FALSE（但仍成功）
        let manager2 = DesktopManager::new();
        // 根据 COM 规范，重复初始化在同一线程应该返回 S_FALSE
        // CoInitializeEx 会返回 S_FALSE 但仍然增加引用计数
        assert!(manager2.is_ok(), "重复初始化应该成功（返回 S_FALSE）");
    }
}
