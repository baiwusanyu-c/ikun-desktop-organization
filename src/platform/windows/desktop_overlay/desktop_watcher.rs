//! 桌面文件监听
//!
//! 阶段 7 使用 `notify` 监听用户桌面和公共桌面目录。当桌面文件新增、删除、
//! 重命名或修改时，经过 debounce 后通知覆盖窗口重新枚举未管理图标。

use anyhow::{Context, Result};
use notify::event::AccessKind;
use notify::{recommended_watcher, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

const DEBOUNCE_MS: u64 = 500;

struct DesktopWatcherHandle {
    _watcher: RecommendedWatcher,
    stop_tx: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

static DESKTOP_WATCHER: Mutex<Option<DesktopWatcherHandle>> = Mutex::new(None);

/// 启动桌面文件监听。
///
/// 重复调用会复用已有监听器；监听失败只向调用方返回错误，由生命周期层记录并降级。
pub(crate) fn start_desktop_file_watcher() -> Result<()> {
    let mut guard = DESKTOP_WATCHER
        .lock()
        .map_err(|e| anyhow::anyhow!("桌面文件监听锁获取失败: {}", e))?;
    if guard.is_some() {
        log::debug!("桌面文件监听已运行，跳过重复启动");
        return Ok(());
    }

    let watch_paths = desktop_watch_paths();
    if watch_paths.is_empty() {
        anyhow::bail!("没有可监听的桌面目录");
    }

    let (event_tx, event_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let mut watcher = recommended_watcher(move |res| {
        if event_tx.send(res).is_err() {
            log::debug!("桌面文件监听事件接收端已关闭");
        }
    })
    .context("创建桌面文件监听器失败")?;

    for path in &watch_paths {
        watcher
            .watch(path, RecursiveMode::NonRecursive)
            .with_context(|| format!("监听桌面目录失败: {}", path.display()))?;
    }

    let thread = std::thread::spawn(move || {
        let debounce = Duration::from_millis(DEBOUNCE_MS);
        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }

            let mut pending_refresh = false;
            match event_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(event)) => {
                    if should_refresh_for_event(&event.kind) {
                        pending_refresh = true;
                    }
                }
                Ok(Err(e)) => {
                    log::warn!("桌面文件监听事件错误: {}", e);
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }

            if !pending_refresh {
                continue;
            }

            loop {
                if stop_rx.recv_timeout(debounce).is_ok() {
                    return;
                }

                let mut saw_refresh_during_drain = false;
                loop {
                    match event_rx.try_recv() {
                        Ok(Ok(event)) => {
                            if should_refresh_for_event(&event.kind) {
                                saw_refresh_during_drain = true;
                            }
                        }
                        Ok(Err(e)) => log::warn!("桌面文件监听事件错误: {}", e),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => return,
                    }
                }

                if !saw_refresh_during_drain {
                    break;
                }
            }

            super::overlay_window::request_desktop_overlay_refresh();
            log::info!("桌面文件变化已触发覆盖窗口刷新");
        }
        log::debug!("桌面文件监听线程已退出");
    });

    log::info!(
        "桌面文件监听已启动: {}",
        watch_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    );
    *guard = Some(DesktopWatcherHandle {
        _watcher: watcher,
        stop_tx,
        thread: Some(thread),
    });
    Ok(())
}

/// 停止桌面文件监听。
pub(crate) fn stop_desktop_file_watcher() {
    let handle = match DESKTOP_WATCHER.lock() {
        Ok(mut guard) => guard.take(),
        Err(e) => {
            log::warn!("桌面文件监听锁获取失败: {}", e);
            return;
        }
    };

    if let Some(mut handle) = handle {
        let _ = handle.stop_tx.send(());
        if let Some(thread) = handle.thread.take() {
            let _ = thread.join();
        }
        log::info!("桌面文件监听已停止");
    }
}

fn desktop_watch_paths() -> Vec<PathBuf> {
    desktop_watch_paths_from(
        dirs::desktop_dir(),
        PathBuf::from(r"C:\Users\Public\Desktop"),
    )
}

fn desktop_watch_paths_from(
    user_desktop: Option<PathBuf>,
    public_desktop: PathBuf,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = user_desktop {
        if path.exists() {
            paths.push(path);
        }
    }
    if public_desktop.exists() && !paths.iter().any(|p| p == &public_desktop) {
        paths.push(public_desktop);
    }
    paths
}

fn should_refresh_for_event(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(_)) => true,
        EventKind::Access(_) => false,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_) | EventKind::Any => true,
        EventKind::Other => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind};

    #[test]
    fn test_should_refresh_for_create_remove_modify_and_any() {
        assert!(should_refresh_for_event(&EventKind::Create(
            CreateKind::File
        )));
        assert!(should_refresh_for_event(&EventKind::Remove(
            RemoveKind::File
        )));
        assert!(should_refresh_for_event(&EventKind::Modify(
            ModifyKind::Name(notify::event::RenameMode::Both)
        )));
        assert!(should_refresh_for_event(&EventKind::Any));
    }

    #[test]
    fn test_should_ignore_read_only_access_events() {
        assert!(!should_refresh_for_event(&EventKind::Access(
            AccessKind::Read
        )));
        assert!(!should_refresh_for_event(&EventKind::Access(
            AccessKind::Open(AccessMode::Read)
        )));
    }

    #[test]
    fn test_desktop_watch_paths_from_deduplicates_public_path() {
        let existing = std::env::temp_dir();
        let paths = desktop_watch_paths_from(Some(existing.clone()), existing.clone());
        assert_eq!(paths, vec![existing]);
    }
}
