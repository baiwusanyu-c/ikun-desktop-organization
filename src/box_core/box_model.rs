//! 分类框数据模型

#![allow(dead_code)] // 分类框数据模型，部分字段/方法在当前构建路径未使用
//!
//! `ClassificationBox` 是分类框的核心数据结构，包含名称、图标列表、
//! 排序/视图/分组设置等。不包含任何渲染或窗口相关代码。

use crate::config::app_config::{GroupBy, SortMode, ViewMode};
use uuid::Uuid;

/// 分类框数据模型
#[derive(Debug, Clone)]
pub struct ClassificationBox {
    /// 唯一 ID
    pub id: Uuid,
    /// 显示名称
    pub name: String,
    /// 是否锁定（锁定后不可拖拽/缩放）
    pub locked: bool,
    /// 位置 (x, y)
    pub position: [f32; 2],
    /// 尺寸 (width, height)
    pub size: [f32; 2],
    /// 包含的文件项
    pub items: Vec<BoxItem>,
    /// 排序模式
    pub sort_mode: SortMode,
    /// 是否升序
    pub sort_ascending: bool,
    /// 视图模式
    pub view_mode: ViewMode,
    /// 分组方式
    pub group_by: GroupBy,
}

/// 分类框中的文件项
#[derive(Debug, Clone)]
pub struct BoxItem {
    /// 文件路径
    pub file_path: String,
    /// 显示名称
    pub display_name: String,
    /// 文件类型（exe/lnk/folder/other）
    pub file_type: String,
    /// 文件修改时间
    pub modified_time: Option<chrono::DateTime<chrono::Utc>>,
    /// 文件创建时间
    pub created_time: Option<chrono::DateTime<chrono::Utc>>,
    /// 文件大小
    pub file_size: Option<u64>,
}

impl Default for ClassificationBox {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "新分类框".to_string(),
            locked: false,
            position: [100.0, 100.0],
            size: [400.0, 600.0],
            items: Vec::new(),
            sort_mode: SortMode::default(),
            sort_ascending: true,
            view_mode: ViewMode::default(),
            group_by: GroupBy::default(),
        }
    }
}

impl ClassificationBox {
    /// 创建新分类框
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
}
