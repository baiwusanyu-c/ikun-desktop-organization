# GPUI Component Windows Popups Refactor

## Goal
按迁移计划分阶段将 Windows 平台的两个现有 egui/eframe 窗口迁移到 GPUI + gpui-component：

1. 控制面板窗口：`Desktop Organizer 控制面板`
2. 创建分类框窗口：`创建分类框`

迁移后必须保持现有功能、状态流转、触发入口、窗口位置、关闭/取消/确认行为、校验规则、托盘/信号文件联动等交互语义一致。采用 Windows-only 的 GPUI 主 UI 壳迁移方案，不在同一运行路径长期并行运行 eframe/winit 与 GPUI 两套 GUI runtime。

## Requirements
- 分阶段实施并逐阶段验证，优先建立行为基线与测试保护线。
- Windows 构建引入 `gpui = "0.2.2"` 与 `gpui-component = "0.5.1"`，先完成最小可编译 GPUI 原型。
- 抽离 UI 无关的 command/state 层，让旧 egui 路径与后续 GPUI 路径复用同一业务逻辑。
- 控制面板 GPUI window 保持旧窗口标题、尺寸、按钮顺序、状态显示、托盘打开、X 隐藏而非退出、不显示任务栏、鼠标所在显示器居中等行为。
- 创建分类框 GPUI window 必须是独立 OS window，不使用 gpui-component Dialog overlay 作为主实现；保持名称校验、Enter 确认、ESC/X/取消取消、重复打开清空与置前、确认后创建并保存配置等行为。
- 保留现有 Win32 helper 或泛化其职责，用于 taskbar/tool window、置顶、物理像素定位、多显示器/DPI 校正。
- Windows 默认入口最终切到 GPUI runtime；非 Windows 路径不作为本次重点，但不得无意破坏现有条件编译。
- 不恢复或覆盖已有被删除的 `todo/IMPLEMENTATION_PLAN.md`，避免破坏现有未提交变更。

## Acceptance Criteria
- [ ] 阶段 0：记录当前测试基线和手动行为核对清单。
- [ ] 阶段 1：项目内可编译最小 GPUI/gpui-component Windows 原型，`Button` 与 `Input` 可用。
- [ ] 阶段 2：名称校验、创建分类框位置计算、控制面板快照、业务 command 从 egui 渲染层中抽离并有测试覆盖。
- [ ] 阶段 3：GPUI 控制面板外观/文案/按钮顺序与旧版本一致，托盘打开、X 隐藏、不显示任务栏、居中置前正常。
- [ ] 阶段 4：GPUI 创建分类框四个入口均可打开，校验和确认/取消行为一致，确认后创建分类框并保存配置。
- [ ] 阶段 5：Windows 默认启动 GPUI 版本，旧 egui 控制面板和 create viewport 不再参与运行。
- [ ] 阶段 6：`cargo fmt --check`、`cargo check`、`cargo test` 通过；如项目现状允许，`cargo clippy -- -D warnings` 通过。
- [ ] 退出时系统托盘、注册表、overlay/grid 生命周期清理不回归。
- [ ] 多 DPI、多显示器、中文输入、托盘/信号文件/桌面右键入口经过手动 QA。

## Technical Notes
- 本任务跨 UI、AppContext、platform/windows、config、托盘/IPC 等多个层，必须遵守单向依赖：UI → AppContext/command → 业务/平台；platform 不依赖 UI。
- command/state 层应成为校验和业务调用的单一入口，避免 egui 与 GPUI 重复实现名称校验、trim、创建分类框保存等逻辑。
- 公开 API 和新模块需要文档注释；错误处理使用 `anyhow::Result` 和中文 `.context()`；不使用 `unwrap/expect` 处理可恢复错误。
- GPUI runtime 需要调用 `gpui_component::init(cx)`，窗口内容使用 `Root::new(view, window, cx)` 包裹。
- 创建分类框窗口定位继续以 `get_cursor_monitor_work_area_phys()` + `reposition_window_by_title("创建分类框", x, y)` 作为物理像素校正手段。
- 第一阶段以可编译和行为保护为优先，不急于删除 egui/eframe 依赖；最终阶段再评估 legacy feature 或依赖收敛。

## Source Plan
- `todo/GPUI_COMPONENT_WINDOWS_POPUP_REFACTOR_PLAN.md`
