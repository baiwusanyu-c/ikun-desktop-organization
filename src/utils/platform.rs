//! 平台特定工具函数

#![allow(dead_code)] // 平台工具函数，部分为架构预留

/// 注册表 Run 键路径
const RUN_KEY_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// 注册表值名称
const RUN_VALUE_NAME: &str = "IkunOrganizer";

/// 设置 Windows 开机自启。
///
/// 通过注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 实现。
/// 开启时写入当前可执行文件路径，关闭时删除注册表值。
#[cfg(windows)]
pub fn set_auto_start(enabled: bool) -> anyhow::Result<()> {
    use std::ffi::c_void;

    extern "system" {
        fn RegCreateKeyExW(
            hKey: isize,
            lpSubKey: *const u16,
            Reserved: u32,
            lpClass: *const u16,
            dwOptions: u32,
            samDesired: u32,
            lpSecurityAttributes: *mut c_void,
            phkResult: *mut isize,
            lpdwDisposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            hKey: isize,
            lpValueName: *const u16,
            Reserved: u32,
            dwType: u32,
            lpData: *const u8,
            cbData: u32,
        ) -> i32;
        fn RegDeleteValueW(hKey: isize, lpValueName: *const u16) -> i32;
        fn RegOpenKeyExW(
            hKey: isize,
            lpSubKey: *const u16,
            ulOptions: u32,
            samDesired: u32,
            phkResult: *mut isize,
        ) -> i32;
        fn RegCloseKey(hKey: isize) -> i32;
    }

    const HKEY_CURRENT_USER: isize = 0x80000001u32 as isize;
    const KEY_WRITE: u32 = 0x20006;
    const KEY_READ: u32 = 0x20019;
    const REG_SZ: u32 = 1;
    const REG_OPTION_NON_VOLATILE: u32 = 0;
    const ERROR_SUCCESS: i32 = 0;

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    if enabled {
        // 获取当前可执行文件路径
        let exe_path = std::env::current_exe()
            .map_err(|e| anyhow::anyhow!("无法获取当前可执行文件路径: {}", e))?;
        let exe_str = exe_path.to_string_lossy().to_string();
        let exe_wide = to_wide(&exe_str);
        let key_path = to_wide(RUN_KEY_PATH);
        let value_name = to_wide(RUN_VALUE_NAME);

        unsafe {
            let mut hkey: isize = 0;
            let mut disposition: u32 = 0;
            let result = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key_path.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null_mut(),
                &mut hkey,
                &mut disposition,
            );
            if result != ERROR_SUCCESS {
                anyhow::bail!("打开注册表键失败: error={}", result);
            }

            let byte_len = (exe_wide.len() * 2) as u32;
            let result = RegSetValueExW(
                hkey,
                value_name.as_ptr(),
                0,
                REG_SZ,
                exe_wide.as_ptr() as *const u8,
                byte_len,
            );
            RegCloseKey(hkey);

            if result != ERROR_SUCCESS {
                anyhow::bail!("写入注册表值失败: error={}", result);
            }
        }

        log::info!("开机自启已开启: {}", exe_str);
    } else {
        // 删除注册表值
        let key_path = to_wide(RUN_KEY_PATH);
        let value_name = to_wide(RUN_VALUE_NAME);

        unsafe {
            let mut hkey: isize = 0;
            let result = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                key_path.as_ptr(),
                0,
                KEY_WRITE,
                &mut hkey,
            );
            if result != ERROR_SUCCESS {
                // 键不存在，无需删除
                log::info!("开机自启注册表键不存在，无需删除");
                return Ok(());
            }

            let result = RegDeleteValueW(hkey, value_name.as_ptr());
            RegCloseKey(hkey);

            if result != ERROR_SUCCESS && result != 2 {
                // error 2 = FILE_NOT_FOUND, 值不存在
                anyhow::bail!("删除注册表值失败: error={}", result);
            }
        }

        log::info!("开机自启已关闭");
    }

    Ok(())
}

/// 查询当前开机自启状态。
///
/// 检查注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 中是否存在本程序条目。
#[cfg(windows)]
pub fn is_auto_start_enabled() -> bool {
    extern "system" {
        fn RegOpenKeyExW(
            hKey: isize,
            lpSubKey: *const u16,
            ulOptions: u32,
            samDesired: u32,
            phkResult: *mut isize,
        ) -> i32;
        fn RegQueryValueExW(
            hKey: isize,
            lpValueName: *const u16,
            lpReserved: *mut u32,
            lpType: *mut u32,
            lpData: *mut u8,
            lpcbData: *mut u32,
        ) -> i32;
        fn RegCloseKey(hKey: isize) -> i32;
    }

    const HKEY_CURRENT_USER: isize = 0x80000001u32 as isize;
    const KEY_READ: u32 = 0x20019;
    const ERROR_SUCCESS: i32 = 0;

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let key_path = to_wide(RUN_KEY_PATH);
    let value_name = to_wide(RUN_VALUE_NAME);

    unsafe {
        let mut hkey: isize = 0;
        let result = RegOpenKeyExW(HKEY_CURRENT_USER, key_path.as_ptr(), 0, KEY_READ, &mut hkey);
        if result != ERROR_SUCCESS {
            return false;
        }

        let mut data_size: u32 = 0;
        let result = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut data_size,
        );
        RegCloseKey(hkey);

        result == ERROR_SUCCESS
    }
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

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;

    #[test]
    fn test_set_auto_start_toggle() {
        // Get initial state
        let initial = is_auto_start_enabled();

        // Toggle to opposite
        let new_state = !initial;
        set_auto_start(new_state).unwrap();
        assert_eq!(is_auto_start_enabled(), new_state);

        // Toggle back
        set_auto_start(initial).unwrap();
        assert_eq!(is_auto_start_enabled(), initial);
    }

    #[test]
    fn test_set_auto_start_idempotent() {
        let state = is_auto_start_enabled();

        // Setting same state twice should succeed
        set_auto_start(state).unwrap();
        set_auto_start(state).unwrap();
        assert_eq!(is_auto_start_enabled(), state);
    }
}
