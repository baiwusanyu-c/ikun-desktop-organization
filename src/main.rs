//! Desktop Organizer - 入口

// 使用 windows 子系统（无控制台窗口），日志写入文件
#![windows_subsystem = "windows"]

#[cfg(feature = "eframe-ui")]
mod app;
mod box_core;
mod classification;
mod config;
mod context;
#[allow(dead_code)]
mod ipc;
mod platform;
mod plugin;
#[cfg(feature = "eframe-ui")]
mod ui;
mod utils;

#[cfg(windows)]
mod gpui_app;

fn main() -> anyhow::Result<()> {
    utils::init_logger();
    log::info!("========================================");
    log::info!("启动 Desktop Organizer (重写版)");
    log::info!("========================================");

    // 设置 Per-Monitor DPI V2 感知（运行时设置，作为 manifest 的补充保障）
    // 防止 Windows 对窗口做 bitmap-stretch 导致文字和图标模糊
    #[cfg(target_os = "windows")]
    {
        set_per_monitor_dpi_v2();
        init_ole();
    }

    // 桌面右键菜单注册表项触发：通过信号文件与运行中的实例通信
    // 写入信号文件后立即退出，运行中的实例轮询检测到信号后执行对应操作
    if let Some(arg) = std::env::args().nth(1) {
        let signal_name = match arg.as_str() {
            "--create-box" => Some("ikun_create_box.signal"),
            "--open-panel" => Some("ikun_open_panel.signal"),
            "--ai-classify" => Some("ikun_ai_classify.signal"),
            "--quit" => Some("ikun_quit.signal"),
            _ => None,
        };
        if let Some(name) = signal_name {
            let signal = std::env::temp_dir().join(name);
            let _ = std::fs::write(&signal, "1");
            return Ok(());
        }
    }

    // Windows 平台使用 GPUI（默认）
    #[cfg(all(target_os = "windows", not(feature = "eframe-ui")))]
    {
        gpui_app::windows_main::run_gpui_app();
        return Ok(());
    }

    // eframe 回退（仅当启用 eframe-ui feature 时）
    #[cfg(feature = "eframe-ui")]
    {
        run_eframe_app()?;
        Ok(())
    }

    #[cfg(not(any(target_os = "windows", feature = "eframe-ui")))]
    {
        anyhow::bail!("当前平台不支持 GPUI，请使用 Windows");
    }
}

/// 运行 eframe 应用（回退 UI 运行时）
#[cfg(feature = "eframe-ui")]
fn run_eframe_app() -> anyhow::Result<()> {
    use eframe::egui;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(app::CONTROL_PANEL_TITLE)
            .with_inner_size([320.0, 380.0])
            .with_position([-32000.0, -32000.0])
            .with_decorations(true)
            .with_always_on_top()
            .with_resizable(true)
            .with_visible(true)
            .with_taskbar(false),
        ..Default::default()
    };

    eframe::run_native(
        "Desktop Organizer",
        options,
        Box::new(|cc| {
            setup_chinese_fonts(&cc.egui_ctx);
            Box::new(app::MyApp::new(cc))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe 运行失败: {}", e))?;

    Ok(())
}

#[cfg(feature = "eframe-ui")]
fn setup_chinese_fonts(ctx: &eframe::egui::Context) {
    let mut fonts = eframe::egui::FontDefinitions::default();

    let font_paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyhbd.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];

    for path in &font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            log::info!("加载中文字体: {}", path);
            fonts.font_data.insert(
                "chinese".to_owned(),
                eframe::egui::FontData::from_owned(font_data),
            );
            if let Some(f) = fonts
                .families
                .get_mut(&eframe::egui::FontFamily::Proportional)
            {
                f.insert(0, "chinese".to_owned());
            }
            if let Some(f) = fonts.families.get_mut(&eframe::egui::FontFamily::Monospace) {
                f.insert(0, "chinese".to_owned());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
    log::warn!("未找到中文字体");
}

/// 设置 Per-Monitor DPI V2 感知（运行时调用，作为 manifest 的补充保障）
///
/// 确保 Windows 不对格子窗口做 bitmap-stretch，保持文字和图标清晰。
/// DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
#[cfg(target_os = "windows")]
fn set_per_monitor_dpi_v2() {
    use std::ffi::c_void;

    #[allow(non_camel_case_types)]
    type DPI_AWARENESS_CONTEXT = *mut c_void;
    const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: DPI_AWARENESS_CONTEXT =
        -4isize as *mut c_void;

    extern "system" {
        fn SetProcessDpiAwarenessContext(value: DPI_AWARENESS_CONTEXT) -> i32;
    }

    let result =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if result != 0 {
        log::info!("已设置 Per-Monitor DPI V2 感知（运行时）");
    } else {
        log::warn!("设置 Per-Monitor DPI V2 失败（可能系统版本不支持），依赖 manifest 配置");
    }
}

/// 初始化 OLE（拖拽功能必需）
///
/// OleInitialize 初始化 COM 单线程单元（STA）并注册 OLE 剪贴板和拖拽子系统。
/// 必须在 DoDragDrop / RegisterDragDrop 之前调用。
#[cfg(target_os = "windows")]
fn init_ole() {
    extern "system" {
        fn OleInitialize(pMalloc: *mut std::ffi::c_void) -> i32;
    }
    let hr = unsafe { OleInitialize(std::ptr::null_mut()) };
    if hr == 0 || hr == 1 {
        log::info!("OLE 初始化成功");
    } else {
        log::warn!(
            "OleInitialize 失败: hr={:#010X}，拖拽功能可能不可用",
            hr as u32
        );
    }

    // 检查进程完整性级别（High = 管理员，Medium = 普通用户）
    // OLE 拖拽到桌面需要与 Explorer.exe 相同的完整性级别
    check_integrity_level();
}

/// 检查进程完整性级别
///
/// 如果应用以管理员权限运行（High Integrity），OLE 拖拽到桌面会失败，
/// 因为 Explorer.exe 运行在 Medium Integrity，Windows UIPI 阻止跨级别通信。
#[cfg(target_os = "windows")]
fn check_integrity_level() {
    use std::ffi::c_void;

    type HANDLE = *mut c_void;
    type PSID = *mut c_void;

    extern "system" {
        fn GetCurrentProcess() -> HANDLE;
        fn OpenProcessToken(
            process_handle: HANDLE,
            desired_access: u32,
            token_handle: *mut HANDLE,
        ) -> i32;
        fn GetTokenInformation(
            token_handle: HANDLE,
            token_information_class: u32,
            token_information: *mut c_void,
            token_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: HANDLE) -> i32;
    }

    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_INTEGRITY_LEVEL: u32 = 25;
    const SECURITY_MANDATORY_HIGH_RID: u32 = 0x00003000;

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            log::warn!("无法获取进程令牌");
            return;
        }

        // 第一次调用获取所需大小
        let mut needed: u32 = 0;
        GetTokenInformation(
            token,
            TOKEN_INTEGRITY_LEVEL,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );

        if needed > 0 {
            let mut buffer = vec![0u8; needed as usize];
            if GetTokenInformation(
                token,
                TOKEN_INTEGRITY_LEVEL,
                buffer.as_mut_ptr() as *mut c_void,
                needed,
                &mut needed,
            ) != 0
            {
                // TOKEN_MANDATORY_LABEL 结构: { PSID Label; DWORD Attributes; }
                // PSID 是指针（8 bytes on 64-bit），后面跟 Attributes
                let label_ptr = *(buffer.as_ptr() as *const PSID);
                if !label_ptr.is_null() {
                    // SID 的最后一个 DWORD 是 RID（完整性级别）
                    extern "system" {
                        fn GetSidSubAuthorityCount(pSid: PSID) -> *mut u8;
                        fn GetSidSubAuthority(pSid: PSID, nSubAuthority: u32) -> *mut u32;
                    }
                    let count = *GetSidSubAuthorityCount(label_ptr) as u32;
                    if count > 0 {
                        let rid = *GetSidSubAuthority(label_ptr, count - 1);
                        if rid >= SECURITY_MANDATORY_HIGH_RID {
                            log::warn!("⚠️ 进程运行在高完整性级别（管理员权限, RID={:#X}）", rid);
                            log::warn!("⚠️ OLE 拖拽到桌面将被 Windows UIPI 阻止！");
                            log::warn!("⚠️ 请关闭管理员权限重新启动应用，或从普通终端运行");
                        } else {
                            log::info!("进程完整性级别: 正常 (RID={:#X})", rid);
                        }
                    }
                }
            }
        }

        CloseHandle(token);
    }
}
