//! 分类框 Tauri 命令。
//!
//! 命令直接操作原生 Win32 分类框与统一配置，不维护第二套内存模型。

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};
use uuid::Uuid;

use crate::config::{AppConfig, BoxLayoutData};
use crate::tauri_commands::platform_runtime::{SharedTauriRuntime, TauriRuntime};
use crate::tauri_commands::window_cmd::{show_managed_window, ManagedWindow};

/// 分类框名称校验结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub message: String,
}

/// 提供给前端的分类框摘要。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoxData {
    pub id: String,
    pub name: String,
    pub locked: bool,
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub item_count: usize,
}

impl From<&BoxLayoutData> for BoxData {
    fn from(box_data: &BoxLayoutData) -> Self {
        Self {
            id: box_data.id.to_string(),
            name: box_data.name.clone(),
            locked: box_data.locked,
            position: box_data.position,
            size: box_data.size,
            item_count: box_data.icon_names.len(),
        }
    }
}

/// 编辑窗口打开时发送的数据。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EditBoxPayload {
    pub config_id: String,
    pub current_name: String,
}

static PENDING_EDIT_BOX: Mutex<Option<EditBoxPayload>> = Mutex::new(None);

fn lock_runtime(
    runtime: &SharedTauriRuntime,
) -> Result<std::sync::MutexGuard<'_, TauriRuntime>, String> {
    runtime
        .lock()
        .map_err(|error| format!("运行时状态不可用: {error}"))
}

fn validate_name(config: &AppConfig, name: &str, current_id: Option<Uuid>) -> ValidationResult {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return ValidationResult {
            valid: false,
            message: "分类框名称不能为空".to_string(),
        };
    }
    if trimmed.chars().count() > 16 {
        return ValidationResult {
            valid: false,
            message: "分类框名称不能超过 16 个字符".to_string(),
        };
    }
    if config
        .layout
        .boxes
        .iter()
        .any(|item| Some(item.id) != current_id && item.name == trimmed)
    {
        return ValidationResult {
            valid: false,
            message: "已存在同名分类框".to_string(),
        };
    }
    ValidationResult {
        valid: true,
        message: String::new(),
    }
}

/// 创建可供原生分类框复用的编辑窗口回调。
pub fn edit_box_callback(app: tauri::AppHandle) -> Box<dyn Fn(Uuid, &str) + Send + Sync + 'static> {
    Box::new(move |config_id, current_name| {
        let payload = EditBoxPayload {
            config_id: config_id.to_string(),
            current_name: current_name.to_string(),
        };
        if let Ok(mut pending) = PENDING_EDIT_BOX.lock() {
            *pending = Some(payload.clone());
        }

        if let Err(error) = show_managed_window(&app, ManagedWindow::EditBox) {
            log::warn!("显示编辑分类框窗口失败: {error}");
            return;
        }
        let Some(window) = app.get_webview_window(ManagedWindow::EditBox.label()) else {
            log::warn!("编辑分类框窗口不存在，无法打开");
            return;
        };
        if let Err(error) = window.emit("edit-box-opened", payload) {
            log::warn!("发送编辑分类框数据失败: {error}");
        }
    })
}

/// 获取最近一次编辑窗口数据，供前端处理首次监听竞态。
#[tauri::command]
pub fn get_pending_edit_box() -> Option<EditBoxPayload> {
    PENDING_EDIT_BOX
        .lock()
        .ok()
        .and_then(|pending| pending.clone())
}

/// 校验分类框名称。
#[tauri::command]
pub fn validate_box_name(
    runtime: State<'_, SharedTauriRuntime>,
    name: String,
) -> Result<ValidationResult, String> {
    let runtime = lock_runtime(runtime.inner())?;
    Ok(validate_name(&runtime.ctx.config, &name, None))
}

/// 创建原生分类框并持久化布局。
#[tauri::command]
pub fn create_box(
    app: tauri::AppHandle,
    runtime: State<'_, SharedTauriRuntime>,
    name: String,
    _description: String,
) -> Result<BoxData, String> {
    let name = name.trim().to_string();
    let mut runtime = lock_runtime(runtime.inner())?;
    let mut config = runtime.ctx.config.clone();
    let validation = validate_name(&config, &name, None);
    if !validation.valid {
        return Err(validation.message);
    }

    use crate::platform::{
        collect_grid_layout, create_grid_window, get_grid_count, get_primary_monitor_dpi,
        get_screen_resolution, GridCreateParams,
    };

    let (screen_width_physical, screen_height_physical) = get_screen_resolution();
    let dpi = get_primary_monitor_dpi().max(96);
    let screen_width = screen_width_physical * 96 / dpi as i32;
    let screen_height = screen_height_physical * 96 / dpi as i32;
    let existing = get_grid_count();
    let width = 400;
    let height = 400;
    let position_x = ((screen_width - width) / 2 + (existing as i32 * 40) % 200).max(10);
    let position_y = ((screen_height - height) / 2 + (existing as i32 * 30) % 150).max(10);
    let config_id = Uuid::new_v4();

    create_grid_window(GridCreateParams {
        id: existing,
        title: name.clone(),
        x: position_x,
        y: position_y,
        width,
        height,
        items: Vec::new(),
        view_mode: config.box_defaults.view_mode.clone(),
        locked: false,
        sort_mode: config.box_defaults.sort_mode.clone(),
        sort_ascending: config.box_defaults.sort_ascending,
        group_by: config.box_defaults.group_by.clone(),
        config_id,
        edit_box_callback: Some(edit_box_callback(app)),
        // 面板新建框无历史归属：建窗收尾自动收养当前所在屏
        monitor_hint: None,
        monitor_dpi_hint: None,
    })
    .map_err(|error| format!("创建分类框失败: {error}"))?;

    config.layout.boxes = collect_grid_layout();
    config.last_modified = chrono::Utc::now();
    config
        .save()
        .map_err(|error| format!("分类框已创建，但保存布局失败: {error}"))?;
    runtime.replace_config(config.clone());

    let created = config
        .layout
        .boxes
        .iter()
        .find(|item| item.id == config_id)
        .map(BoxData::from)
        .unwrap_or(BoxData {
            id: config_id.to_string(),
            name,
            locked: false,
            position: [position_x as f32, position_y as f32],
            size: [width as f32, height as f32],
            item_count: 0,
        });
    Ok(created)
}

/// 编辑分类框名称并同步原生窗口与配置。
#[tauri::command]
pub fn edit_box(
    runtime: State<'_, SharedTauriRuntime>,
    id: String,
    name: String,
    _description: String,
) -> Result<BoxData, String> {
    let config_id = Uuid::parse_str(&id).map_err(|error| format!("分类框 ID 无效: {error}"))?;
    let name = name.trim().to_string();
    let mut runtime = lock_runtime(runtime.inner())?;
    let mut config = runtime.ctx.config.clone();
    let validation = validate_name(&config, &name, Some(config_id));
    if !validation.valid {
        return Err(validation.message);
    }
    if !config.layout.boxes.iter().any(|item| item.id == config_id) {
        return Err("未找到分类框".to_string());
    }

    #[cfg(windows)]
    {
        crate::platform::update_grid_window_title(config_id, &name);
        config.layout.boxes = crate::platform::collect_grid_layout();
    }
    if let Some(item) = config
        .layout
        .boxes
        .iter_mut()
        .find(|item| item.id == config_id)
    {
        item.name = name;
    }
    config.last_modified = chrono::Utc::now();
    config
        .save()
        .map_err(|error| format!("保存分类框名称失败: {error}"))?;
    runtime.replace_config(config.clone());

    config
        .layout
        .boxes
        .iter()
        .find(|item| item.id == config_id)
        .map(BoxData::from)
        .ok_or_else(|| "未找到分类框".to_string())
}

/// 删除分类框布局记录。
#[tauri::command]
pub fn delete_box(runtime: State<'_, SharedTauriRuntime>, id: String) -> Result<(), String> {
    let config_id = Uuid::parse_str(&id).map_err(|error| format!("分类框 ID 无效: {error}"))?;
    let mut runtime = lock_runtime(runtime.inner())?;
    let mut config = runtime.ctx.config.clone();
    let old_len = config.layout.boxes.len();
    config.layout.boxes.retain(|item| item.id != config_id);
    if old_len == config.layout.boxes.len() {
        return Err("未找到分类框".to_string());
    }
    config
        .save()
        .map_err(|error| format!("保存分类框布局失败: {error}"))?;
    runtime.replace_config(config);
    Ok(())
}

/// 列出分类框。
#[tauri::command]
pub fn list_boxes(runtime: State<'_, SharedTauriRuntime>) -> Result<Vec<BoxData>, String> {
    Ok(lock_runtime(runtime.inner())?
        .ctx
        .config
        .layout
        .boxes
        .iter()
        .map(BoxData::from)
        .collect())
}

/// 获取指定分类框。
#[tauri::command]
pub fn get_box(runtime: State<'_, SharedTauriRuntime>, id: String) -> Result<BoxData, String> {
    let config_id = Uuid::parse_str(&id).map_err(|error| format!("分类框 ID 无效: {error}"))?;
    lock_runtime(runtime.inner())?
        .ctx
        .config
        .layout
        .boxes
        .iter()
        .find(|item| item.id == config_id)
        .map(BoxData::from)
        .ok_or_else(|| "未找到分类框".to_string())
}
