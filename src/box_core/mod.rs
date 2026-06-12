//! 分类框核心业务逻辑（平台无关）
//!
//! 本模块包含分类框的数据模型、排序/视图/分组引擎、布局计算和交互状态管理。
//! 所有逻辑与平台渲染解耦，通过 trait 接口与 `platform/` 层交互。

pub mod box_manager;
pub mod box_model;
pub mod group_engine;
pub mod interaction;
pub mod layout_calc;
pub mod sort_engine;
pub mod view_engine;
