//! 分组引擎

#![allow(dead_code)] // 分组引擎，部分类型在当前构建路径未使用
//!
//! 按名称/类型/修改时间/创建时间/大小等维度对文件项进行分组。

use super::box_model::BoxItem;
use crate::config::app_config::GroupBy;

/// 分组结果
#[derive(Debug, Clone)]
pub struct Group {
    /// 分组标题
    pub title: String,
    /// 该组内的文件项索引
    pub item_indices: Vec<usize>,
}

/// 对文件项进行分组
pub fn group_items(items: &[BoxItem], group_by: &GroupBy) -> Vec<Group> {
    match group_by {
        GroupBy::None => {
            // 不分组，所有项归为一组
            vec![Group {
                title: String::new(),
                item_indices: (0..items.len()).collect(),
            }]
        }
        GroupBy::Type => {
            // TODO: 按文件类型分组
            group_by_key(items, |item| item.file_type.clone())
        }
        GroupBy::Name => {
            // TODO: 按名称首字母分组
            group_by_key(items, |item| {
                item.display_name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_else(|| "#".to_string())
            })
        }
        _ => {
            // TODO: 其他分组维度待实现
            vec![Group {
                title: String::new(),
                item_indices: (0..items.len()).collect(),
            }]
        }
    }
}

/// 通用按 key 分组
fn group_by_key<F>(items: &[BoxItem], key_fn: F) -> Vec<Group>
where
    F: Fn(&BoxItem) -> String,
{
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, item) in items.iter().enumerate() {
        let key = key_fn(item);
        map.entry(key).or_default().push(i);
    }
    map.into_iter()
        .map(|(title, item_indices)| Group {
            title,
            item_indices,
        })
        .collect()
}
