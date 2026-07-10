//! 排序引擎

#![allow(dead_code)] // 排序引擎，部分方法在当前构建路径未使用
//!
//! 支持 8 种排序维度 × 升降序排列。

use super::box_model::BoxItem;
use crate::config::app_config::SortMode;

/// 对文件项列表排序
pub fn sort_items(items: &mut [BoxItem], mode: &SortMode, ascending: bool) {
    match mode {
        SortMode::Name => items.sort_by(|a, b| {
            let ord = a
                .display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase());
            if ascending {
                ord
            } else {
                ord.reverse()
            }
        }),
        SortMode::Type => items.sort_by(|a, b| {
            let ord = a.file_type.cmp(&b.file_type);
            if ascending {
                ord
            } else {
                ord.reverse()
            }
        }),
        SortMode::Size => items.sort_by(|a, b| {
            let ord = a.file_size.unwrap_or(0).cmp(&b.file_size.unwrap_or(0));
            if ascending {
                ord
            } else {
                ord.reverse()
            }
        }),
        SortMode::ModifiedTime => items.sort_by(|a, b| {
            let ord = a.modified_time.cmp(&b.modified_time);
            if ascending {
                ord
            } else {
                ord.reverse()
            }
        }),
        SortMode::CreatedTime => items.sort_by(|a, b| {
            let ord = a.created_time.cmp(&b.created_time);
            if ascending {
                ord
            } else {
                ord.reverse()
            }
        }),
        // TODO: 以下排序维度待完善
        SortMode::Author | SortMode::Tag | SortMode::Title => {
            // 暂时按名称排序
            items.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        }
    }
}
