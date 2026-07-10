# PRD: GPUI Windows 平台代码改进

## Goal

在不涉及业务功能变更的前提下，对 Windows 平台代码进行三项架构与质量改进：
1. 将 legacy eframe/egui UI 代码 feature-gate 到 `eframe-ui` feature 后
2. 清理默认 GPUI 构建下的 dead code / warnings（当前 184 个）
3. 泛化 Win32 window helper（从控制面板专用改为通用浮动窗口 helper）

## Constraints

- **不影响业务功能**：所有改动仅为架构重构和代码清理
- **只影响 Windows 平台代码**：不涉及跨平台逻辑
- **保持编译通过**：`cargo check`、`cargo test`、`cargo fmt --check` 必须通过
- **保留回退能力**：legacy eframe 代码通过 `eframe-ui` feature 仍可编译

## Scope

### In Scope
1. `src/main.rs` — 条件编译 gate legacy app/ui 模块
2. `src/app.rs` + `src/ui/` — feature-gate 到 `eframe-ui`
3. `Cargo.toml` — egui/eframe/epaint 改为 optional
4. `src/gpui_app/` — 清理 GPUI 模块自身 warning
5. `src/platform/windows/` — Win32 helper 泛化 + dead code 清理
6. `src/platform/windows/ffi_types.rs` — snake_case allow 属性
7. `src/platform/windows/control_panel_window.rs` — 泛化为通用 floating window helper
8. `src/platform/windows/ole_dnd.rs` — 清理 OLE DnD dead code（如不再需要）

### Out of Scope
- 业务功能代码（分类、配置保存、托盘逻辑等）
- GPUI 行为收敛（控制面板/创建分类框行为差异，需要手动 QA）
- 非 Windows 平台代码
- macOS 平台适配

## Expected Outcome

### 自动验证
- `cargo fmt --check` 通过
- `cargo check` 通过，warning 数从 184 显著下降
- `cargo test` 通过（当前 136 passed）
- `cargo check --features eframe-ui` 通过（legacy 路径仍可编译）

### 架构改进
- 默认 Windows 构建不再编译 legacy eframe 代码
- Win32 helper 从控制面板专用泛化为通用浮动窗口 helper
- FFI 类型保留 `#[allow(non_snake_case)]` 注释
- OLE DnD dead code 按保留/清理决策处理

## Reference Documents
- `todo/GPUI_BEHAVIOR_CONVERGENCE_AND_LEGACY_CLEANUP_PLAN.md`
- `todo/GPUI_COMPONENT_WINDOWS_POPUP_REFACTOR_PLAN.md`
