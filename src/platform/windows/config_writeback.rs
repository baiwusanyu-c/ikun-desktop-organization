//! 配置回写入口注入点（依赖倒置桥）
//!
//! 平台线程（拖拽收尾、覆盖层图标位置、菜单删除分类框）历史上各自
//! `AppConfig::load()` → 改字段 → `save()` 直写磁盘，绕开了
//! `SharedTauriRuntime` 的内存真源：Tauri 命令随后从陈旧内存整文件回写时，
//! 会把平台刚写盘的字段冲掉（2026-11-04 自检审计 P0：布局丢失、
//! 被删分类框复活）。现在统一经 [`write_platform_config`] 提交：
//! 运行时层在启动时注入「锁内改内存真源 + 落盘」钩子（平台层不得反向
//! 依赖 tauri_commands，模式与 `edit_box_callback` / `TrayActionWaker` 一致）。
//! 钩子未注入（启动早期/单元测试）时退回磁盘读改写，读取失败只跳过本次
//! 写入，**绝不**用默认配置覆盖用户配置。

use std::sync::{Mutex, OnceLock};

use crate::config::AppConfig;

/// 运行时层注入的钩子类型：接一份变更闭包，一次性完成「改内存真源 + 落盘」。
pub type WriteHook =
    Box<dyn Fn(&str, Box<dyn FnOnce(&mut AppConfig) + Send>) -> bool + Send + Sync>;

static WRITE_HOOK: OnceLock<Mutex<Option<WriteHook>>> = OnceLock::new();

/// 注入平台层配置写钩子（仅运行时初始化时调用一次；重复注入被忽略）。
pub fn set_write_hook(hook: WriteHook) {
    let slot = WRITE_HOOK.get_or_init(|| Mutex::new(None));
    match slot.lock() {
        Ok(mut guard) => {
            if guard.is_some() {
                log::warn!("配置写钩子已存在，忽略本次重复注入");
            } else {
                *guard = Some(hook);
                log::info!("配置写钩子已注入（平台写配置统一入口生效）");
            }
        }
        Err(_) => log::error!("配置写钩子注入失败：锁不可用"),
    }
}

/// 平台层唯一配置写入口。
///
/// - `what`：业务描述（失败日志用）；
/// - `update`：变更闭包，运行在内存真源（钩子未注入时为磁盘最新快照）上。
///
/// # Returns
/// - `true`：变更与落盘全部成功
/// - `false`：任一步失败（不做部分保留，绝不回写默认配置）
pub fn write_platform_config(
    what: &str,
    update: impl FnOnce(&mut AppConfig) + Send + 'static,
) -> bool {
    if let Some(slot) = WRITE_HOOK.get() {
        if let Ok(guard) = slot.lock() {
            if let Some(hook) = guard.as_ref() {
                return hook(what, Box::new(update));
            }
        }
    }
    // 回退路径：运行时钩子尚未注入。先读磁盘最新快照，失败即跳过本次写入。
    match AppConfig::load() {
        Ok(mut config) => {
            update(&mut config);
            config.last_modified = chrono::Utc::now();
            match config.save() {
                Ok(()) => true,
                Err(error) => {
                    log::warn!("保存配置失败（{what}）: {error}");
                    false
                }
            }
        }
        Err(error) => {
            log::warn!("加载配置失败（{what}）: {error}，跳过本次写入以免覆盖用户配置");
            false
        }
    }
}
