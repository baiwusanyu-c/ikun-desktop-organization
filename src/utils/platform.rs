//! 平台特定工具函数

#![allow(dead_code)] // 平台工具函数，部分为架构预留

/// 注册表 Run 键路径
const RUN_KEY_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// 注册表值名称
const RUN_VALUE_NAME: &str = "IkunOrganizer";
/// Windows 启动批准标记键路径（任务管理器/启动影响评估/安全软件会写入此键）
const STARTUP_APPROVED_KEY_PATH: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
/// 启动批准标记：启用（REG_BINARY 首字节 0x02；0x03 表示禁用）
const STARTUP_APPROVED_ENABLED: [u8; 12] = [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

#[cfg(windows)]
mod registry {
    //! 开机自启相关注册表操作的 FFI 声明与常量。
    use std::ffi::c_void;

    extern "system" {
        pub fn RegCreateKeyExW(
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
        pub fn RegSetValueExW(
            hKey: isize,
            lpValueName: *const u16,
            Reserved: u32,
            dwType: u32,
            lpData: *const u8,
            cbData: u32,
        ) -> i32;
        pub fn RegDeleteValueW(hKey: isize, lpValueName: *const u16) -> i32;
        pub fn RegOpenKeyExW(
            hKey: isize,
            lpSubKey: *const u16,
            ulOptions: u32,
            samDesired: u32,
            phkResult: *mut isize,
        ) -> i32;
        pub fn RegQueryValueExW(
            hKey: isize,
            lpValueName: *const u16,
            lpReserved: *mut u32,
            lpType: *mut u32,
            lpData: *mut u8,
            lpcbData: *mut u32,
        ) -> i32;
        pub fn RegCloseKey(hKey: isize) -> i32;
    }

    pub const HKEY_CURRENT_USER: isize = 0x80000001u32 as isize;
    pub const KEY_WRITE: u32 = 0x20006;
    pub const KEY_READ: u32 = 0x20019;
    pub const REG_SZ: u32 = 1;
    pub const REG_BINARY: u32 = 3;
    pub const REG_OPTION_NON_VOLATILE: u32 = 0;
    pub const ERROR_SUCCESS: i32 = 0;
    pub const ERROR_FILE_NOT_FOUND: i32 = 2;

    /// 将字符串转换为以 NUL 结尾的 UTF-16 宽字符序列。
    pub fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

/// 写入/删除 Windows 启动批准标记。
///
/// Windows 通过 `HKCU\\...\\Explorer\\StartupApproved\\Run` 下的 REG_BINARY 值
/// 记录自启条目是否被禁用（任务管理器、启动影响评估或安全软件都可能写入 0x03 禁用标记）。
/// 开启自启时主动写入启用标记，防止条目被静默禁用导致开机不启动；
/// 关闭自启时删除标记，避免残留。
#[cfg(windows)]
fn set_startup_approved(enabled: bool) -> anyhow::Result<()> {
    use registry::*;

    let key_path = to_wide(STARTUP_APPROVED_KEY_PATH);
    let value_name = to_wide(RUN_VALUE_NAME);

    unsafe {
        if enabled {
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
                anyhow::bail!("打开 StartupApproved 注册表键失败: error={}", result);
            }

            let result = RegSetValueExW(
                hkey,
                value_name.as_ptr(),
                0,
                REG_BINARY,
                STARTUP_APPROVED_ENABLED.as_ptr(),
                STARTUP_APPROVED_ENABLED.len() as u32,
            );
            RegCloseKey(hkey);

            if result != ERROR_SUCCESS {
                anyhow::bail!("写入 StartupApproved 启用标记失败: error={}", result);
            }
            log::debug!("StartupApproved 启用标记已写入");
        } else {
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
                return Ok(());
            }

            let result = RegDeleteValueW(hkey, value_name.as_ptr());
            RegCloseKey(hkey);

            if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND {
                anyhow::bail!("删除 StartupApproved 标记失败: error={}", result);
            }
            log::debug!("StartupApproved 标记已删除");
        }
    }

    Ok(())
}

/// 设置 Windows 开机自启。
///
/// 通过注册表 `HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run` 实现。
/// 开启时写入带双引号的当前可执行文件路径（防止路径含空格导致启动失败），
/// 并写入 StartupApproved 启用标记；关闭时删除注册表值与批准标记。
#[cfg(windows)]
pub fn set_auto_start(enabled: bool) -> anyhow::Result<()> {
    use registry::*;

    if enabled {
        // 获取当前可执行文件路径
        let exe_path = std::env::current_exe()
            .map_err(|e| anyhow::anyhow!("无法获取当前可执行文件路径: {}", e))?;
        // 双引号包裹路径：Run 键值按命令行解析，含空格的路径必须加引号
        let exe_str = format!("\"{}\"", exe_path.to_string_lossy());
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

        // 清除/覆盖 Windows 的“启动批准”禁用标记，防止条目被静默禁用
        if let Err(error) = set_startup_approved(true) {
            log::warn!("写入开机自启批准标记失败（不影响 Run 键）: {error}");
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

            if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND {
                // error 2 = FILE_NOT_FOUND, 值不存在
                anyhow::bail!("删除注册表值失败: error={}", result);
            }
        }

        // 一并清理启动批准标记，避免残留禁用状态
        if let Err(error) = set_startup_approved(false) {
            log::warn!("清理开机自启批准标记失败（不影响 Run 键）: {error}");
        }

        log::info!("开机自启已关闭");
    }

    Ok(())
}

/// 查询当前开机自启状态。
///
/// 检查注册表 `HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run` 中是否存在本程序条目。
#[cfg(windows)]
pub fn is_auto_start_enabled() -> bool {
    use registry::*;

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

    /// 串行化所有操作真实注册表的测试，避免并行竞争导致断言失败
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 读取 Run 键中本程序条目的原始值（测试后原样恢复，避免污染用户环境）
    fn read_run_value() -> Option<String> {
        use registry::*;
        use std::os::windows::ffi::OsStringExt;

        let key_path = to_wide(RUN_KEY_PATH);
        let value_name = to_wide(RUN_VALUE_NAME);
        unsafe {
            let mut hkey: isize = 0;
            if RegOpenKeyExW(HKEY_CURRENT_USER, key_path.as_ptr(), 0, KEY_READ, &mut hkey)
                != ERROR_SUCCESS
            {
                return None;
            }
            let mut size: u32 = 0;
            if RegQueryValueExW(
                hkey,
                value_name.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            ) != ERROR_SUCCESS
            {
                RegCloseKey(hkey);
                return None;
            }
            let mut buf: Vec<u16> = vec![0; size as usize / 2 + 1];
            let result = RegQueryValueExW(
                hkey,
                value_name.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut u8,
                &mut size,
            );
            RegCloseKey(hkey);
            if result != ERROR_SUCCESS {
                return None;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(std::ffi::OsString::from_wide(&buf[..len]).to_string_lossy().into_owned())
        }
    }

    /// 原样写回 Run 键条目
    fn write_run_value_raw(value: &str) {
        use registry::*;

        let key_path = to_wide(RUN_KEY_PATH);
        let value_name = to_wide(RUN_VALUE_NAME);
        let wide = to_wide(value);
        unsafe {
            let mut hkey: isize = 0;
            let mut disposition: u32 = 0;
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key_path.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null_mut(),
                &mut hkey,
                &mut disposition,
            ) != ERROR_SUCCESS
            {
                return;
            }
            RegSetValueExW(
                hkey,
                value_name.as_ptr(),
                0,
                REG_SZ,
                wide.as_ptr() as *const u8,
                (wide.len() * 2) as u32,
            );
            RegCloseKey(hkey);
        }
    }

    /// 删除 Run 键条目
    fn delete_run_value() {
        use registry::*;

        let key_path = to_wide(RUN_KEY_PATH);
        let value_name = to_wide(RUN_VALUE_NAME);
        unsafe {
            let mut hkey: isize = 0;
            if RegOpenKeyExW(HKEY_CURRENT_USER, key_path.as_ptr(), 0, KEY_WRITE, &mut hkey)
                != ERROR_SUCCESS
            {
                return;
            }
            RegDeleteValueW(hkey, value_name.as_ptr());
            RegCloseKey(hkey);
        }
    }

    /// 测试结束后把 Run 键恢复到测试前的原始状态
    fn restore_run_value(original: Option<&str>) {
        match original {
            Some(value) => write_run_value_raw(value),
            None => delete_run_value(),
        }
    }

    #[test]
    fn test_set_auto_start_toggle() {
        let _guard = test_lock();
        let original = read_run_value();

        // Get initial state
        let initial = is_auto_start_enabled();

        // Toggle to opposite
        let new_state = !initial;
        set_auto_start(new_state).unwrap();
        assert_eq!(is_auto_start_enabled(), new_state);

        // Toggle back
        set_auto_start(initial).unwrap();
        assert_eq!(is_auto_start_enabled(), initial);

        restore_run_value(original.as_deref());
    }

    #[test]
    fn test_set_auto_start_idempotent() {
        let _guard = test_lock();
        let original = read_run_value();

        let state = is_auto_start_enabled();

        // Setting same state twice should succeed
        set_auto_start(state).unwrap();
        set_auto_start(state).unwrap();
        assert_eq!(is_auto_start_enabled(), state);

        restore_run_value(original.as_deref());
    }

    #[test]
    fn test_startup_approved_follows_auto_start() {
        let _guard = test_lock();
        // 开启自启后 StartupApproved 标记写入应成功（不 panic）
        assert!(set_startup_approved(true).is_ok());
        // 删除标记也应成功（幂等）
        assert!(set_startup_approved(false).is_ok());
        assert!(set_startup_approved(false).is_ok());
    }
}
