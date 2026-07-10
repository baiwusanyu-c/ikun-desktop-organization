//! 视图引擎

#![allow(dead_code)] // 视图引擎，部分方法在当前构建路径未使用
//!
//! 8 种视图模式的布局参数计算：超大图标、大图标、中等、小图标、列表、详细信息、平铺、内容。

use crate::config::app_config::ViewMode;

/// 视图布局参数
#[derive(Debug, Clone)]
pub struct ViewParams {
    /// 图标尺寸（像素）
    pub icon_size: i32,
    /// 单元格宽度
    pub cell_w: i32,
    /// 单元格高度
    pub cell_h: i32,
    /// 是否显示标签文字
    pub show_label: bool,
    /// 是否显示详细信息（大小/类型/日期）
    pub show_details: bool,
    /// 每行列数（0 = 自动计算）
    pub fixed_cols: Option<i32>,
}

/// 根据视图模式获取布局参数
pub fn get_view_params(mode: &ViewMode, dpi: u32) -> ViewParams {
    let scale = |px: i32| (px * dpi as i32) / 96;

    match mode {
        ViewMode::ExtraLarge => ViewParams {
            icon_size: scale(96),
            cell_w: scale(120),
            cell_h: scale(140),
            show_label: true,
            show_details: false,
            fixed_cols: None,
        },
        ViewMode::Large => ViewParams {
            icon_size: scale(64),
            cell_w: scale(96),
            cell_h: scale(120),
            show_label: true,
            show_details: false,
            fixed_cols: None,
        },
        ViewMode::Medium => ViewParams {
            icon_size: scale(48),
            cell_w: scale(80),
            cell_h: scale(100),
            show_label: true,
            show_details: false,
            fixed_cols: None,
        },
        ViewMode::Small => ViewParams {
            icon_size: scale(32),
            cell_w: scale(64),
            cell_h: scale(80),
            show_label: true,
            show_details: false,
            fixed_cols: None,
        },
        ViewMode::List => ViewParams {
            icon_size: scale(16),
            cell_w: scale(200),
            cell_h: scale(24),
            show_label: true,
            show_details: false,
            fixed_cols: Some(1),
        },
        ViewMode::Detail => ViewParams {
            icon_size: scale(16),
            cell_w: scale(600),
            cell_h: scale(28),
            show_label: true,
            show_details: true,
            fixed_cols: Some(1),
        },
        ViewMode::Tiles => ViewParams {
            icon_size: scale(48),
            cell_w: scale(240),
            cell_h: scale(64),
            show_label: true,
            show_details: true,
            fixed_cols: None,
        },
        ViewMode::Content => ViewParams {
            icon_size: scale(32),
            cell_w: scale(400),
            cell_h: scale(48),
            show_label: true,
            show_details: true,
            fixed_cols: Some(1),
        },
    }
}
