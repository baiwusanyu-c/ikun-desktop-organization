//! AI 分类命令
//!
//! 提供 AI 一键分类功能的前端接口

use tauri::{Emitter, State};

use crate::tauri_commands::platform_runtime::SharedTauriRuntime;
use crate::tauri_commands::window_cmd::{show_managed_window, ManagedWindow};

/// 启动 AI 分类对话框
#[tauri::command]
pub fn show_ai_classify_dialog(
    runtime: State<'_, SharedTauriRuntime>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let rt = runtime
        .lock()
        .map_err(|e| format!("运行时状态不可用: {e}"))?;

    // 检查 AI 配置
    let config = &rt.ctx.config;
    if config.ai.base_url.is_empty() || config.ai.api_key.is_empty() || config.ai.model.is_empty() {
        return Err("请先在设置中配置 AI 分类参数".to_string());
    }

    // 显示对话框
    show_managed_window(&app, ManagedWindow::AiClassify)
        .map_err(|e| format!("显示 AI 分类对话框失败: {e}"))?;

    Ok(())
}

/// 开始 AI 分类（异步）
#[tauri::command]
pub async fn start_ai_classification(
    runtime: State<'_, SharedTauriRuntime>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    log::info!("[AI] start_ai_classification 已调用");

    let config = {
        let rt = runtime
            .lock()
            .map_err(|e| format!("运行时状态不可用: {e}"))?;
        log::info!(
            "[AI] 获取配置: base_url={}, model={}",
            rt.ctx.config.ai.base_url,
            rt.ctx.config.ai.model
        );
        rt.ctx.config.clone()
    };

    // 在后台线程执行 AI 分类
    let app_clone = app.clone();
    let runtime_clone = runtime.inner().clone();

    log::info!("[AI] 准备启动 tokio 任务");
    tokio::spawn(async move {
        log::info!("[AI] tokio 任务已启动");
        log::info!("[AI] 开始调用 perform_ai_classification_async");
        match perform_ai_classification_async(config).await {
            Ok(groups) => {
                log::info!("[AI] AI 分类成功，共 {} 个分类", groups.len());
                // 在主线程创建分类框
                let app_for_create = app_clone.clone();
                let runtime_for_create = runtime_clone.clone();

                if let Err(e) = app_clone.run_on_main_thread(move || {
                    if let Err(e) =
                        create_classified_boxes(&app_for_create, &runtime_for_create, &groups)
                    {
                        log::error!("创建分类框失败: {e}");
                        let _ = app_for_create
                            .emit("ai-classification-error", format!("创建分类框失败: {e}"));
                    } else {
                        let icon_count: usize = groups.values().map(|v| v.len()).sum();
                        let box_count = groups.len();
                        let _ = app_for_create.emit(
                            "ai-classification-success",
                            serde_json::json!({
                                "icon_count": icon_count,
                                "box_count": box_count
                            }),
                        );
                    }
                }) {
                    log::error!("调度到主线程失败: {e}");
                }
            }
            Err(e) => {
                log::error!("[AI] AI 分类失败: {}", e);
                let _ = app_clone.emit("ai-classification-error", e.to_string());
            }
        }
    });

    Ok(())
}

/// 异步执行 AI 分类
async fn perform_ai_classification_async(
    config: crate::config::AppConfig,
) -> Result<std::collections::HashMap<String, Vec<crate::platform::DesktopIcon>>, String> {
    use std::collections::HashMap;

    // 获取桌面图标
    // 桌面图标枚举无共享状态，直接使用平台实现
    let icons = crate::platform::windows::DesktopManager.get_desktop_icons();

    if icons.is_empty() {
        return Err("桌面没有图标".to_string());
    }

    // 调用 AI 分类
    let classifier = crate::classification::AiClassifier::new(config.ai.clone())
        .map_err(|e| format!("创建 AI 分类器失败: {e}"))?;

    let results = classifier
        .classify(&icons)
        .await
        .map_err(|e| format!("AI 分类失败: {e}"))?;

    // 按分类分组
    let mut groups: HashMap<String, Vec<crate::platform::DesktopIcon>> = HashMap::new();
    for icon in &icons {
        let category = results
            .iter()
            .find(|(name, _)| name == &icon.display_name)
            .map(|(_, cat)| cat.clone())
            .unwrap_or_else(|| "其他".to_string());
        groups.entry(category).or_default().push(icon.clone());
    }

    Ok(groups)
}

/// 在主线程创建分类框
fn create_classified_boxes(
    app: &tauri::AppHandle,
    runtime: &SharedTauriRuntime,
    groups: &std::collections::HashMap<String, Vec<crate::platform::DesktopIcon>>,
) -> Result<(), String> {
    use crate::platform::{
        create_grid_window, destroy_all_grid_windows, extract_file_icon, get_primary_monitor_dpi,
        get_screen_resolution, restore_all_hidden_icons, GridCreateParams, GridFileItem,
    };

    let config = {
        let rt = runtime
            .lock()
            .map_err(|e| format!("运行时状态不可用: {e}"))?;
        rt.ctx.config.clone()
    };

    // ★ 先销毁所有分类框
    destroy_all_grid_windows();

    // ★ 恢复所有图标到桌面
    restore_all_hidden_icons();

    log::info!("[AI] 已恢复所有图标到桌面，准备创建新分类框");

    // ★ 按照拖拽逻辑的顺序处理图标
    // 1. 对于 public 图标：先隐藏（检查权限），失败则跳过
    // 2. 对于 non-public 图标：先不隐藏，等创建完分类框后再隐藏
    let mut filtered_groups: std::collections::HashMap<
        String,
        Vec<(crate::platform::DesktopIcon, bool)>,
    > = std::collections::HashMap::new();
    for (category, icons) in groups {
        let mut valid_icons = Vec::new();
        for icon in icons {
            let path_str = icon.path.to_string_lossy().to_string();
            let is_public = crate::platform::windows::icon_hider::is_public_desktop_path(&path_str);

            if is_public {
                // public 图标需要授权，先隐藏（作为 preflight），失败则跳过
                let success = crate::platform::windows::drag_drop::hide_single_desktop_icon(
                    &path_str,
                    &icon.display_name,
                );
                if !success {
                    log::warn!("[AI] public 图标授权失败，跳过: {}", icon.display_name);
                    continue;
                }
                valid_icons.push((icon.clone(), true)); // true = is_public
            } else {
                // non-public 图标先不隐藏，等创建完分类框后再隐藏
                valid_icons.push((icon.clone(), false)); // false = not public
            }
        }
        if !valid_icons.is_empty() {
            filtered_groups.insert(category.clone(), valid_icons);
        }
    }

    // 计算屏幕尺寸
    let (screen_w, screen_h) = get_screen_resolution();
    let dpi = get_primary_monitor_dpi();
    let scale = dpi as f32 / 96.0;
    let logical_w = (screen_w as f32 / scale) as i32;
    let logical_h = (screen_h as f32 / scale) as i32;

    // 计算每个分类框的尺寸和位置
    let box_count = filtered_groups.len();
    let gap = 20;
    let padding = 40;
    let available_width = logical_w - padding * 2;

    // 计算每行放多少个框子
    let cols = if box_count <= 3 { box_count as i32 } else { 3 };
    let rows = (box_count as i32 + cols - 1) / cols;

    let box_width = (available_width - gap * (cols - 1)) / cols;
    let box_height = (logical_h - padding * 2 - gap * (rows - 1)) / rows;
    let box_height = box_height.min(600); // 最大高度

    let start_x = padding;
    let start_y = padding;

    // 创建分类框
    let mut created = 0;
    for (index, (category_name, group_icons)) in filtered_groups.iter().enumerate() {
        if group_icons.is_empty() {
            continue;
        }

        let row = index as i32 / cols;
        let col = index as i32 % cols;
        let x = start_x + col * (box_width + gap);
        let y = start_y + row * (box_height + gap);

        // 提取图标
        let items: Vec<GridFileItem> = group_icons
            .iter()
            .map(|(icon, _is_public)| {
                // ★ 使用原始路径（快捷方式路径），而不是 target_path（exe 路径）
                let path = icon.path.to_string_lossy().to_string();
                // 解析实际读取路径（图标可能已被隐藏到仓库）
                let read_path =
                    crate::platform::windows::drag_drop::resolve_drag_item_read_path(&path);
                let read_path_str = read_path.to_string_lossy().to_string();

                let hicon = extract_file_icon(&read_path_str)
                    .map(|info| info.hicon)
                    .unwrap_or_else(|| {
                        log::warn!("[AI] 无法提取图标: {} (解析后: {})", path, read_path_str);
                        std::ptr::null_mut()
                    });
                GridFileItem {
                    file_path: path,
                    display_name: icon.display_name.clone(),
                    icon_handle: hicon,
                    grid_col: -1,
                    grid_row: -1,
                }
            })
            .collect();

        let params = GridCreateParams {
            id: index,
            title: category_name.clone(),
            x,
            y,
            width: box_width,
            height: box_height,
            items,
            view_mode: config.box_defaults.view_mode.clone(),
            locked: false,
            sort_mode: config.box_defaults.sort_mode.clone(),
            sort_ascending: config.box_defaults.sort_ascending,
            group_by: config.box_defaults.group_by.clone(),
            config_id: uuid::Uuid::new_v4(),
            edit_box_callback: Some(crate::tauri_commands::box_cmd::edit_box_callback(
                app.clone(),
            )),
        };

        if let Err(e) = create_grid_window(params) {
            log::warn!("创建分类框 '{}'失败: {e}", category_name);
        } else {
            created += 1;
        }
    }

    log::info!("创建了 {}/{} 个分类框", created, box_count);

    // 重建后必须把新布局经运行时锁回写「内存真源 + 磁盘」：
    // 旧实现只改窗口不落配置，后续任何 Tauri 配置命令都会拿 AI 分类前的
    // 陈旧布局整文件回写，把新框冲掉（2026-11-04 自检审计 P1）。
    {
        let layouts = crate::platform::collect_grid_layout();
        match runtime.lock() {
            Ok(mut rt) => {
                rt.ctx.config.layout.boxes = layouts;
                rt.ctx.config.last_modified = chrono::Utc::now();
                if let Err(e) = rt.ctx.config.save() {
                    log::warn!("[AI] 分类框新布局落盘失败: {e}");
                } else {
                    log::info!("[AI] 分类框新布局已落盘（内存与磁盘同步）");
                }
            }
            Err(e) => log::warn!("[AI] 运行时锁不可用，分类框新布局未落盘: {e}"),
        }
    }

    // ★ 最后，对于所有 non-public 图标，调用 hide_single_desktop_icon 隐藏
    // 这确保了如果创建分类框失败，non-public 图标还在桌面上
    for icons in filtered_groups.values() {
        for (icon, is_public) in icons {
            if !is_public {
                let path_str = icon.path.to_string_lossy().to_string();
                crate::platform::windows::drag_drop::hide_single_desktop_icon(
                    &path_str,
                    &icon.display_name,
                );
            }
        }
    }

    Ok(())
}
