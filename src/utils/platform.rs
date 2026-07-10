//! 平台特定工具函数

#![allow(dead_code)] // 平台工具函数，部分为架构预留

/// 设置开机自启
#[cfg(windows)]
pub fn set_auto_start(enabled: bool) -> anyhow::Result<()> {
    // TODO: Windows 注册表实现
    log::info!("Windows 开机自启设置: {}", enabled);
    Ok(())
}

/// 设置开机自启
#[cfg(target_os = "macos")]
pub fn set_auto_start(enabled: bool) -> anyhow::Result<()> {
    // TODO: macOS LaunchAgent 实现
    log::info!("macOS 开机自启设置: {}", enabled);
    Ok(())
}

/// 获取用户主目录
///
/// 配置文件 `~/.ikun_organizer.json` 存放在此目录。
pub fn get_home_dir() -> anyhow::Result<std::path::PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("无法获取用户主目录"))
}

/// 获取配置文件路径
///
/// 返回 `~/.ikun_organizer.json`
pub fn get_config_file_path() -> anyhow::Result<std::path::PathBuf> {
    get_home_dir().map(|p| p.join(".ikun_organizer.json"))
}

/// 获取桌面路径
pub fn get_desktop_path() -> anyhow::Result<std::path::PathBuf> {
    dirs::desktop_dir().ok_or_else(|| anyhow::anyhow!("无法获取桌面路径"))
}
