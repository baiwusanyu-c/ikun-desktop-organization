//! 跨平台共享逻辑层
//!
//! 从 Windows 平台实现中提取的平台无关代码，供 macOS 重新实现时直接使用。
//!
//! # 模块结构
//! - \selection\: 全局多选注册表（跨容器选择/锚点/框选）
//! - \drag_logic\: 拖拽纯逻辑（拖拽状态计算/shift-insert）
//! - \geometry\: 几何计算（对齐检测/像素边界/等比居中）
//! - \layout\: 布局重排逻辑（列数变化时图标重映射）
//! - \drag_state\: 拖拽生命周期状态机
//! - \	ypes\: 跨平台共享类型定义

pub mod drag_logic;
pub mod drag_state;
pub mod geometry;
pub mod layout;
pub mod selection;
pub mod types;
