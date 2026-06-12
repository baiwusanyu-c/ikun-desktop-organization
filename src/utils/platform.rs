//! 平台特定工具函数

/// 设置开机自启
#[cfg(windows)]
pub fn set_auto_start(enabled: bool) -> anyhow::Result<()> {
    // TODO: Windows 注册表实现
    // use winreg::RegKey;
    // use winreg::enums::*;

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

/// 获取配置目录
pub fn get_config_dir() -> anyhow::Result<std::path::PathBuf> {
    dirs::config_dir()
        .map(|p| p.join("ikun-desktop-organization"))
        .ok_or_else(|| anyhow::anyhow!("无法获取配置目录"))
}

/// 获取桌面路径
pub fn get_desktop_path() -> anyhow::Result<std::path::PathBuf> {
    dirs::desktop_dir().ok_or_else(|| anyhow::anyhow!("无法获取桌面路径"))
}
