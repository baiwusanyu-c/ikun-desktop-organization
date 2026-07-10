//! GPUI 应用模块（Windows 平台）
//!
//! 使用 GPUI + gpui-component 替代 eframe/egui，
//! 实现控制面板和创建分类框对话框的完整 UI。
//!
//! ## 架构
//! - app_runtime — UI 框架无关的业务逻辑（完整移植自 app.rs）
//! - control_panel — 控制面板 GPUI 视图
//! - create_box_dialog — 创建分类框对话框 GPUI 视图
//! - windows_main — Windows GPUI 应用入口

pub mod app_runtime;
pub mod assets;
pub mod control_panel;
pub mod create_box_dialog;
pub mod windows_main;
