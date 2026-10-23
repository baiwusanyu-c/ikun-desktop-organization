//! 分类框业务逻辑核心（平台无关）
//!
//! 分类框数据模型、布局/视图/排序/分组引擎、交互状态机，以及从旧 `shared/`
//! 层并入的纯逻辑：拖拽计算（drag_logic/drag_state）、几何与对齐检测
//! （geometry）、布局重排（layout）、全局多选注册表（selection）、共享
//! 几何类型（types）。本模块不允许出现 Win32/渲染依赖，由 platform/ 层
//! 负责落地渲染与窗口管理。

pub mod box_manager;
pub mod box_model;
pub mod drag_logic;
pub mod drag_state;
pub mod geometry;
pub mod group_engine;
pub mod interaction;
pub mod layout;
pub mod layout_calc;
pub mod selection;
pub mod sort_engine;
pub mod types;
pub mod view_engine;
