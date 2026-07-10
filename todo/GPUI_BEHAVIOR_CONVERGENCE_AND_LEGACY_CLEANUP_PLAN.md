# GPUI 行为收敛与 legacy eframe / dead code 整理实施计划

> 日期：2026-07-10  
> 任务来源：`.trellis/tasks/07-10-gpui-component-windows-popups-refactor/prd.md`  
> 目标：在当前 GPUI Windows 默认入口已经可编译、`cargo test` 已通过的基础上，继续收敛 GPUI 控制面板与创建分类框窗口行为，并系统整理 legacy eframe 路径与默认构建下的 dead code / warnings，为后续 `cargo clippy -- -D warnings` 达标创造条件。

---

## 1. 当前状态判断

### 1.1 已完成/基本完成

当前代码已经具备以下基础：

- Windows 默认入口已切到 GPUI：`src/main.rs` 默认调用 `gpui_app::windows_main::run_gpui_app()`。
- Windows target 已接入：
  - `gpui = "0.2.2"`
  - `gpui-component = "0.5.1"`
- 已新增 GPUI 模块：
  - `src/gpui_app/mod.rs`
  - `src/gpui_app/app_runtime.rs`
  - `src/gpui_app/control_panel.rs`
  - `src/gpui_app/create_box_dialog.rs`
  - `src/gpui_app/windows_main.rs`
- GPUI runtime 已能：
  - 初始化 `gpui_component::init(cx)`。
  - 创建控制面板窗口。
  - 轮询托盘动作、信号文件、右键菜单 pending create box。
  - 打开创建分类框独立 GPUI window。
- 业务逻辑基本迁移到 `AppRuntime`：
  - 一键分类。
  - 清除 & 恢复。
  - 保存配置。
  - 创建空分类框。
  - 恢复布局。
  - 退出清理。
- 自动验证当前结果：
  - `cargo fmt --check` 通过。
  - `cargo check` 通过，但有大量 warning。
  - `cargo test` 通过：136 passed。
  - `cargo clippy -- -D warnings` 未通过，主要被默认 GPUI 路径下未使用的 legacy eframe 与平台辅助代码阻断。

### 1.2 仍需重点收敛的问题

从 PRD 与当前代码看，剩余工作应分成两条线并行推进：

1. **GPUI 行为收敛**：确保用户可见行为与旧 eframe/egui 版本一致。
2. **legacy eframe / dead code 整理**：减少默认 GPUI 构建下的未使用代码，避免把旧 UI 路径和新 GPUI 路径长期混在同一个默认编译单元里。

这两条线相互影响：只有 GPUI 行为足够稳定，才能放心进一步隔离或删除 legacy eframe 路径；否则应保留明确的 `eframe-ui` 回退 feature。

---

## 2. 总体实施原则

### 2.1 不做的事

- 不恢复已删除的 `todo/IMPLEMENTATION_PLAN.md`。
- 不在默认 Windows 运行路径中重新引入 eframe event loop。
- 不用 `gpui-component::Dialog` 替代“创建分类框”主实现，因为它不是独立 OS window。
- 不为了快速消 warning 大面积加 `#[allow(dead_code)]` 掩盖真实问题；只对明确的 FFI / 回调 / 测试辅助保留 allow。
- 不一次性删除 legacy eframe 代码，除非 GPUI 行为通过手动 QA 且回滚需求已确认不再需要。

### 2.2 应遵循的设计原则

- Windows 默认路径：GPUI 是唯一 GUI runtime。
- legacy eframe 路径：只能在显式 feature 下编译。
- 业务逻辑：保留在 `AppRuntime` 或更通用 command/state 层，避免 GPUI 与 eframe 双份实现。
- 平台层：继续只提供 Win32 helper，不依赖 GPUI view 或 UI 层。
- 错误处理：窗口创建、配置保存、图标恢复等失败必须记录中文日志，避免 `expect/unwrap` 处理可恢复错误。
- 验证顺序：每个小阶段至少跑 `cargo fmt --check`、`cargo check`、相关测试；行为改动后补手动 QA 项。

---

## 3. 实施路线总览

建议按以下 6 个阶段推进：

| 阶段 | 主题 | 目标 |
|---|---|---|
| A | GPUI 行为差距审计 | 建立当前 GPUI vs 旧 eframe 行为差距表 |
| B | 控制面板行为收敛 | X 隐藏、托盘打开、taskbar 隐藏、居中置前、状态刷新一致 |
| C | 创建分类框行为收敛 | 四入口、重复打开、输入焦点、Enter/ESC/X/取消/确认、定位与保存一致 |
| D | Win32 window helper 泛化 | 将 control-panel 专用 helper 泛化为 tool/floating window helper |
| E | legacy eframe feature-gate 整理 | 默认 GPUI 构建不编译旧 eframe App 代码，减少 dead code |
| F | warning / clippy 分层清理 | 在不破坏架构前提下逐步靠近 `clippy -- -D warnings` |

---

## 4. 阶段 A：GPUI 行为差距审计

### 4.1 目标

先把当前 GPUI 实际行为与 PRD 中旧行为要求逐项对齐，避免后续靠猜测修改。

### 4.2 需要检查的文件

- `src/main.rs`
- `src/gpui_app/windows_main.rs`
- `src/gpui_app/app_runtime.rs`
- `src/gpui_app/control_panel.rs`
- `src/gpui_app/create_box_dialog.rs`
- `src/platform/windows/control_panel_window.rs`
- `src/platform/windows/grid_window.rs` 中 `reposition_window_by_title` / `get_cursor_monitor_work_area_phys`
- `src/platform/windows/sys_tray.rs`
- `src/platform/windows/shell_menu.rs`

### 4.3 输出物

新增或更新一份 QA 表，例如：

- `todo/GPUI_WINDOWS_MANUAL_QA_CHECKLIST.md`

建议表格字段：

| 功能 | 旧行为 | 当前 GPUI 预期 | 自动测试 | 手动测试结果 | 备注 |
|---|---|---|---|---|---|
| 启动控制面板 | 初始停放屏幕外，不显示任务栏 | 同旧行为 | 否 | 待测 | 需观察任务栏 |
| 托盘打开控制面板 | 鼠标所在屏居中、聚焦 | 同旧行为 | 否 | 待测 | 多屏必测 |
| 控制面板 X | 隐藏，不退出 | 同旧行为 | 可拆纯逻辑测试 | 待测 | GPUI close hook |
| 创建框重复打开 | 清空输入、置前、重新定位，不多开 | 同旧行为 | 部分可测 | 待测 | 当前实现为关闭旧窗后新建 |
| ESC | 取消，不创建 | 同旧行为 | 需手动或 GPUI test | 待测 | 需焦点命中 |

### 4.4 验收标准

- 每个 PRD acceptance criterion 都有对应检查项。
- 明确哪些可自动测试，哪些只能手动 QA。
- 不在审计阶段修改大量实现，只做必要的小修或记录。

---

## 5. 阶段 B：GPUI 控制面板行为收敛

### 5.1 目标

控制面板必须保持旧版本行为：

- 标题：`Desktop Organizer 控制面板`。
- 尺寸：`320 x 380`。
- 按钮顺序一致：
  1. `🔄 一键分类`
  2. `🗑 清除 & 恢复`
  3. `💾 保存配置`
  4. `➕ 创建分类框`
- 托盘打开时：鼠标所在显示器居中、显示、聚焦。
- X 按钮：隐藏，不退出。
- 不显示任务栏。
- 状态消息、图标隐藏提示、分类框数量、底部说明语义一致。

### 5.2 当前风险点

1. **启动隐藏时机不够稳**  
   当前 `windows_main.rs` 创建窗口后延迟 1 秒调用 `hide_control_panel_window`。这可能导致启动时短暂闪现。

2. **GPUI window titlebar/title 与 Win32 `FindWindowW` 查找稳定性**  
   Win32 helper 依赖标题查找窗口，必须确保 GPUI 标题创建后稳定可查。

3. **taskbar 隐藏依赖 Win32 style 修正**  
   GPUI 本身不一定等价于 eframe `.with_taskbar(false)`，必须继续依赖 `WS_EX_TOOLWINDOW` / 移除 `WS_EX_APPWINDOW`。

4. **状态刷新频率**  
   当前轮询 50ms，功能上简单但可能过于频繁；旧 egui 使用 `request_repaint_after(300ms)`。

### 5.3 建议实现步骤

#### B1. 控制面板初始隐藏改为“创建后立即 Win32 配置 + 停放”

- 保留创建 GPUI root window。
- 在 `ControlPanelView::open_window` 成功后立即调用：
  - 配置 tool window。
  - 停放到 `(-32000, -32000)`。
- 减少或取消 1 秒延迟隐藏。
- 如果 GPUI 窗口刚创建时还未被 `FindWindowW` 找到，可改成短延迟重试，但重试上限明确，例如 10 次 × 50ms。

#### B2. 泛化 Win32 helper 前先补最小函数

在 `platform/windows/control_panel_window.rs` 或新文件中补通用语义函数：

- `configure_tool_window_by_title(title: &str) -> bool`
- `park_window_offscreen_by_title(title: &str) -> bool`
- `show_tool_window_centered_by_title(title: &str) -> bool`
- `activate_window_by_title(title: &str) -> bool`

短期可以让旧函数包装新函数：

```rust
pub fn hide_control_panel_window(title: &str) -> bool {
    configure_tool_window_by_title(title) && park_window_offscreen_by_title(title)
}
```

#### B3. 控制面板 close hook 保持只隐藏

当前 `window.on_window_should_close` 已经做非 quit 隐藏并返回 `false`，需继续验证：

- 返回 `false` 是否真的阻止 GPUI window 被移除。
- 隐藏后是否可以通过托盘再次显示。
- 点击 X 后 AppRuntime 中 `panel_visible == false`。

如果发现 GPUI close hook 返回 `false` 后仍有异常，则改为：

- close hook 内调用 `hide_control_panel_window`。
- 不 remove window。
- 保留 handle 常驻。

#### B4. 状态刷新策略调整

当前 50ms 轮询建议分层：

- 托盘/信号响应：100ms 或 150ms 足够。
- native icon retry：300ms 或 500ms 足够。
- UI refresh：仅在 `tick()` 返回 changed 时刷新，否则低频 refresh。

为了降低风险，第一版可保留 50ms，等行为稳定后再调整到 100-250ms。

### 5.4 建议测试

自动测试：

- `ControlPanelSnapshot` 或等价纯函数：status、icons_hidden、grid_count 可读。
- close decision 纯函数：`quit=false -> hide`，`quit=true -> allow close`。

手动 QA：

- 启动后任务栏无控制面板入口。
- 启动无明显闪窗。
- 托盘 Open：当前鼠标所在屏居中。
- X：窗口消失但进程/托盘仍在。
- 再次托盘 Open：窗口恢复并聚焦。

---

## 6. 阶段 C：GPUI 创建分类框行为收敛

### 6.1 目标

创建分类框窗口必须保持旧行为：

- 独立 OS window。
- 标题：`创建分类框`。
- inner size 近似 `360 x 180`。
- 位置算法：`get_cursor_monitor_work_area_phys()` + 360x200 估算，水平居中、垂直三分之一。
- 打开时清空输入。
- 四入口均可打开：
  - 控制面板按钮。
  - 系统托盘 CreateBox。
  - 信号文件 `ikun_create_box.signal`。
  - 分类框右键菜单 `take_pending_create_box()`。
- 校验：trim 非空且 `chars().count() <= 16`。
- 文案：
  - 空：`名称不能为空`
  - 超长：`⚠ 最多 16 字符（当前 N）`
  - 有效：`✓ N/16`
- `确定` 仅有效时可点。
- Enter 与确定一致。
- ESC / X / 取消均取消，不创建。
- 确认后创建分类框、保存配置、状态消息一致。
- 重复打开：清空输入、置前/重新定位，不产生多个窗口。

### 6.2 当前风险点

1. **重复打开策略**  
   当前实现趋向“关闭旧窗口后新建窗口”。这可满足清空输入与重新定位，但可能比旧行为多一次窗口销毁/创建闪动。若用户可见闪烁，再改为复用窗口并 reset input。

2. **输入焦点**  
   当前 `InputState` 创建后未明确 `focus()`。打开窗口后用户可能需要手动点输入框。

3. **ESC 事件焦点**  
   `div().on_key_down` 只有在该节点/窗口获得键盘事件时才可靠。若焦点在 `Input` 内部，ESC 是否冒泡需实测。

4. **X 关闭后的 dialog handle 状态**  
   需要确保 `dialog_handle` 在下一次打开时能发现旧 handle 失效并重新创建。

5. **确认后状态一致性**  
   `confirm_create_box()` 使用 `runtime.dialog_name`，GPUI view 点击/Enter 时应先写入 trimmed name，再调用 confirm。当前做法基本正确，但应补纯测试覆盖。

### 6.3 建议实现步骤

#### C1. 抽出 dialog 操作纯状态函数

在 `app_runtime.rs` 中新增/整理纯函数，便于测试：

- `normalized_box_name(input: &str) -> String`
- `box_name_validation(input: &str) -> BoxNameValidation`
- `validation_message(input: &str) -> ValidationMessage`

示例结构：

```rust
pub struct BoxNameValidation {
    pub trimmed: String,
    pub char_count: usize,
    pub is_empty: bool,
    pub is_too_long: bool,
    pub is_valid: bool,
}
```

GPUI view 与测试均使用同一逻辑，避免 view 内重复判断。

#### C2. 打开 dialog 时确保输入框聚焦

在 `CreateBoxDialogView::new` 创建 `InputState` 后：

- 尝试调用 `input_state.update(cx, |input, cx| input.focus(window, cx));`
- 或在窗口创建后下一 tick 聚焦。

需要用最小编译验证 gpui-component API 签名。

#### C3. ESC 行为双保险

保留当前 root `on_key_down`，再补充以下任一方案：

- 对 `InputState` 的 key/action 做 ESC 订阅（若 gpui-component 支持）。
- 或在 window / view 层注册 GPUI action。

验收前必须手动确认：输入框有焦点时按 ESC 也取消。

#### C4. 重复打开策略二选一

**推荐短期策略：关闭旧窗口后新建**

优点：
- 实现简单。
- 保证输入一定清空。
- 保证按新鼠标 monitor 重新定位。

需要补：
- 关闭旧窗口时同时 `rt.cancel_create_box_dialog()` 或至少不触发 confirm。
- 新窗口创建失败时恢复 `dialog_open=false` 或记录状态，避免 runtime 认为打开但没有窗口。

**后续优化策略：复用窗口**

若关闭重建产生闪动，再实现：

- `CreateBoxDialogView::reset(window, cx)`：清空 input、focus、activate。
- 使用 `reposition_window_by_title("创建分类框", x, y)` 重新定位。

#### C5. 确认 / 取消路径统一

在 `CreateBoxDialogView` 中提取私有方法（或闭包 helper）：

- `confirm_if_valid(runtime, input, window, cx)`
- `cancel(runtime, window, cx)`

避免 Enter、确定按钮、ESC、取消按钮、X close hook 各写一套逻辑。

### 6.4 建议测试

自动测试：

- 空字符串 / 全空格无效。
- 16 ASCII 有效，17 ASCII 无效。
- 16 中文有效，17 中文无效。
- trim 后保存：`"  测试  " -> "测试"`。
- 打开对话框会重置：`dialog_name`、`dialog_target_pos`、`dialog_needs_open`。
- cancel 不会调用 create。

手动 QA：

- 控制面板按钮打开。
- 托盘打开。
- 信号文件打开。
- 分类框右键菜单打开。
- 打开后光标直接在输入框中。
- 中文输入法输入正常。
- Enter 创建。
- ESC 取消。
- X 取消。
- 取消按钮取消。
- 重复打开不会出现多个“创建分类框”窗口。
- 多屏/负坐标/不同 DPI 下位置合理。

---

## 7. 阶段 D：Win32 window helper 泛化

### 7.1 目标

当前 `control_panel_window.rs` 名称和职责仍偏 eframe 控制面板，但 GPUI 迁移后它实际上承担通用 Win32 window 语义补齐：

- tool window 样式。
- taskbar 隐藏。
- 停放屏幕外。
- 鼠标所在显示器居中。
- 显示/聚焦。
- 可选 topmost/floating。

应将其泛化，减少后续 “control panel helper 被 create dialog 复用” 的语义混乱。

### 7.2 建议模块结构

方案一：保守改名（低风险）

- 保留文件：`src/platform/windows/control_panel_window.rs`
- 新增通用函数。
- 旧函数继续存在并包装通用函数。

方案二：正式拆分（中风险）

- 新增文件：`src/platform/windows/window_helper.rs`
- `control_panel_window.rs` 只保留兼容包装或删除。
- `platform/windows/mod.rs` re-export 通用 helper。

推荐先采用方案一，行为稳定后再做方案二。

### 7.3 建议 API

```rust
pub fn configure_tool_window_by_title(title: &str) -> bool;
pub fn park_window_offscreen_by_title(title: &str) -> bool;
pub fn show_tool_window_centered_by_title(title: &str) -> bool;
pub fn activate_window_by_title(title: &str) -> bool;
pub fn set_window_topmost_by_title(title: &str, topmost: bool) -> bool;
```

如果后续能从 GPUI 获取 HWND，再新增 hwnd 版本，减少标题查找脆弱性：

```rust
pub unsafe fn configure_tool_window(hwnd: HWND) -> bool;
pub unsafe fn park_window_offscreen(hwnd: HWND) -> bool;
```

### 7.4 注意事项

- 日志继续中文。
- `FindWindowW` 失败应 debug，不应 error。
- `SetWindowLongPtrW` / `SetWindowPos` 修改 style 后必须带 `SWP_FRAMECHANGED`。
- `ShowWindow` + `SetForegroundWindow` 顺序保持。
- 多显示器坐标必须使用物理像素虚拟桌面坐标。

---

## 8. 阶段 E：legacy eframe feature-gate 整理

### 8.1 目标

默认 Windows GPUI 构建不应编译旧 eframe App 代码。当前 `src/main.rs` 虽然只在 `feature = "eframe-ui"` 时运行 eframe，但 `mod app; mod ui;` 仍在默认构建中编译，导致大量 dead code warning。

### 8.2 当前问题

默认构建中：

- `src/app.rs` 的 `MyApp`、`DialogState`、旧 `is_box_name_valid` 等不再被运行路径使用。
- `src/ui/control_panel.rs`、`src/ui/settings_panel.rs`、`src/ui/tray_menu.rs` 仍编译但未使用。
- `egui` / `eframe` / `epaint` 仍是全局依赖，而不是 legacy feature 专属依赖。

这些会直接导致：

- `cargo check` warning 很多。
- `cargo clippy -- -D warnings` 无法通过。
- 新 GPUI 代码质量问题容易被旧 warning 淹没。

### 8.3 推荐实施策略

#### E1. 第一阶段：模块级 feature-gate

在 `src/main.rs` 调整模块声明：

```rust
#[cfg(feature = "eframe-ui")]
mod app;

#[cfg(feature = "eframe-ui")]
mod ui;
```

注意：
- 如果 `plugin` 或其他模块依赖 `ui`，需先搜索确认。
- 当前默认 GPUI 路径若不使用 `ui/`，可直接 gate。

#### E2. 第二阶段：测试迁移或 gate

`app.rs` 中仍有名称校验、对话框状态等测试价值，但这些逻辑已迁移到 `gpui_app/app_runtime.rs`。

处理方式：

- 将仍有价值的测试迁移到 `gpui_app/app_runtime.rs`。
- `app.rs` 测试只在 `eframe-ui` feature 下编译。
- 避免默认 test 继续编译 legacy eframe App。

建议：

```rust
#[cfg(all(test, feature = "eframe-ui"))]
mod tests { ... }
```

#### E3. 第三阶段：依赖 feature 化

将全局依赖：

```toml
egui = "0.27"
eframe = { version = "0.27", default-features = false, features = ["glow"] }
epaint = "0.27"
```

改成 optional：

```toml
egui = { version = "0.27", optional = true }
eframe = { version = "0.27", default-features = false, features = ["glow"], optional = true }
epaint = { version = "0.27", optional = true }

[features]
default = []
eframe-ui = ["dep:egui", "dep:eframe", "dep:epaint"]
```

风险：
- 非 Windows macOS 现有路径是否仍依赖 eframe？需要先确认。
- 如果非 Windows 仍要保留 eframe UI，则 `cfg` 要按平台与 feature 组合设计。

#### E4. 第四阶段：决定保留还是删除 legacy eframe

在 GPUI 行为 QA 全部通过前，建议保留 `eframe-ui` feature。

删除条件：

- GPUI 控制面板 QA 通过。
- GPUI 创建分类框 QA 通过。
- 托盘/信号文件/右键菜单入口通过。
- 退出清理无回归。
- 用户确认不需要 legacy rollback。

### 8.4 验收标准

- 默认 `cargo check` 不再因为 `app.rs` / `ui/` 产生 dead code warning。
- `cargo check --features eframe-ui` 仍可编译旧 UI 回退路径。
- `cargo test` 默认路径仍覆盖 GPUI command/state 逻辑。
- `cargo test --features eframe-ui` 可作为回退路径验证（如保留）。

---

## 9. 阶段 F：warning / clippy 分层清理

### 9.1 目标

逐步让默认 GPUI 构建接近：

```powershell
cargo clippy -- -D warnings
```

### 9.2 当前 clippy 阻断分类

从当前输出看，大致分为：

1. **legacy eframe dead code**  
   应通过阶段 E 的 feature-gate 解决。

2. **平台模块 re-export 未使用**  
   例如 `platform/windows/mod.rs` 中导出的拖拽/OLE/drop target 类型。

3. **FFI / COM / Win32 回调常量未使用**  
   有些是未来功能/测试/回调 ABI 需要保留，有些可删除。

4. **变量未使用**  
   例如 `_dpi`、`_f_release`、`_icon_handle` 等，应按语义改名或移除。

5. **业务模块暂未接入**  
   `box_core` / plugin 等结构可能是架构预留，不宜盲删。

### 9.3 清理顺序

#### F1. 先清默认路径明显无用项

- 未使用 import。
- 未使用局部变量。
- 已被迁移且默认路径不需要的旧 UI 模块。

#### F2. 再处理 re-export

对于 `platform/windows/mod.rs`：

- 搜索每个 re-export 是否被 crate 外或其他模块使用。
- 内部只用模块路径时，不需要 re-export。
- 如果是 API 预留，但默认路径未使用，考虑：
  - 移除 re-export。
  - 或加精确 `#[allow(unused_imports)]` 并注明原因。

#### F3. FFI / 回调代码谨慎处理

对 Win32/COM ABI 相关：

- 回调签名中的未使用参数改为 `_param`。
- ABI struct 字段命名如果必须保持 Win32 风格，可加模块级或结构体级 allow，并注明是 FFI 映射。
- 不因为 warning 删除可能由 Windows 回调间接使用的 static vtable / extern 函数，除非确认该 COM 路径已不再使用。

#### F4. 架构预留模块分流

对于 `box_core` / plugin 等暂未接入但属于架构规划的模块：

- 不建议删除。
- 可通过实际接入使用、测试使用，或模块级 allow 处理。
- allow 必须有注释：说明该模块为后续分类框业务抽象预留。

### 9.4 每步验证

每个子批次后运行：

```powershell
cargo fmt --check
cargo check
cargo test
```

当 warning 数明显下降后再尝试：

```powershell
cargo clippy -- -D warnings
```

### 9.5 验收标准

短期：

- 默认 `cargo check` warning 数明显下降。
- 新 GPUI 相关模块无 `expect/unwrap`、无未使用 import/变量。

中期：

- legacy eframe warning 不再出现在默认构建。
- `cargo clippy -- -D warnings` 剩余问题集中在 Win32 FFI / 架构预留模块。

最终：

- 默认 GPUI 构建 `cargo clippy -- -D warnings` 通过。
- 如保留 legacy：`cargo clippy --features eframe-ui -- -D warnings` 可单独列为后续目标，不阻塞默认路径。

---

## 10. 推荐具体执行顺序

### Step 1：补 QA 清单

新增：

- `todo/GPUI_WINDOWS_MANUAL_QA_CHECKLIST.md`

内容覆盖控制面板、创建分类框、托盘、信号文件、右键菜单、多屏/DPI、中文 IME、退出清理。

### Step 2：先收敛创建分类框

优先级最高，因为它是多入口、多状态、多输入交互的回归高风险点。

改动建议：

1. 抽 `BoxNameValidation` 纯逻辑。
2. GPUI view 复用验证逻辑。
3. 打开后 input focus。
4. ESC 输入框焦点下可取消。
5. 重复打开策略稳定化。
6. 补单元测试。

### Step 3：收敛控制面板窗口生命周期

改动建议：

1. 创建后立即 toolwindow + offscreen。
2. 移除或缩短 1 秒延迟隐藏。
3. 验证 X hide 不 close。
4. 托盘打开后居中置前。

### Step 4：泛化 Win32 helper

改动建议：

1. 在现有 `control_panel_window.rs` 内先加通用函数。
2. 旧函数改为 wrapper。
3. GPUI 代码逐步调用通用函数。
4. 测试 `wide_null`、位置计算继续保留。

### Step 5：默认构建 gate 掉 legacy eframe App

改动建议：

1. `#[cfg(feature = "eframe-ui")] mod app;`
2. `#[cfg(feature = "eframe-ui")] mod ui;`
3. 迁移有价值测试到 `gpui_app/app_runtime.rs`。
4. `egui/eframe/epaint` optional 化。
5. 验证：
   - `cargo check`
   - `cargo check --features eframe-ui`
   - `cargo test`
   - `cargo test --features eframe-ui`

### Step 6：分批清 warning

优先清本次 GPUI 改造引入/暴露的 warning，再清平台层历史 warning。

---

## 11. 风险与回滚方案

### 11.1 GPUI 窗口无法稳定 taskbar 隐藏

回滚/缓解：

- 保留 GPUI window，但强制通过 Win32 helper 修改 style。
- 如果标题查找不稳定，再研究 GPUI `Window` 的 raw HWND 获取能力。

### 11.2 ESC 在 Input 焦点下不触发

回滚/缓解：

- 使用 GPUI action/key binding。
- 或在 `InputState` 订阅/处理 key event。
- 最后备选：窗口级 Win32 message hook，但这应作为最后手段。

### 11.3 legacy eframe gate 后非 Windows 编译受影响

回滚/缓解：

- 将 gate 条件改为：
  - Windows 默认 GPUI。
  - 非 Windows 仍编译 legacy eframe。
- 或新增 feature：`legacy-egui-ui`，明确由用户选择。

### 11.4 clippy 清理误删平台预留代码

回滚/缓解：

- 每批改动小步提交/小步验证。
- 对 FFI/COM vtable 不做大删减。
- 对架构预留模块优先 allow + 注释，而不是删除。

---

## 12. 最终验收清单

### 自动验证

- [ ] `cargo fmt --check` 通过。
- [ ] `cargo check` 通过，默认 GPUI 路径 warning 明显减少。
- [ ] `cargo test` 通过。
- [ ] `cargo check --features eframe-ui` 通过（如保留 legacy）。
- [ ] `cargo test --features eframe-ui` 通过（如保留 legacy）。
- [ ] `cargo clippy -- -D warnings` 通过，或剩余项有明确追踪清单。

### 手动 QA

- [ ] 启动后控制面板不显示任务栏、不闪窗。
- [ ] 托盘打开控制面板，窗口在鼠标所在显示器居中并聚焦。
- [ ] 控制面板 X 隐藏而非退出。
- [ ] 控制面板四个按钮功能一致。
- [ ] 创建分类框四入口均可打开。
- [ ] 创建分类框打开后输入框聚焦。
- [ ] 空/超长/有效名称提示一致。
- [ ] 中文字符按 `chars().count()` 计数。
- [ ] Enter 创建。
- [ ] ESC / X / 取消不创建。
- [ ] 重复打开不会多开窗口，且输入清空、位置刷新。
- [ ] 创建成功后配置保存，状态消息一致。
- [ ] 多显示器、负坐标显示器、高 DPI 下位置合理。
- [ ] Quit 后托盘、注册表、overlay、grid、原生图标恢复无回归。

---

## 13. 建议下一次实际开发切入点

下一次编码建议从 **创建分类框行为收敛** 开始，原因：

1. PRD 对创建分类框的行为要求最多，风险最高。
2. 它跨 GPUI input、runtime state、Win32 定位、业务创建与配置保存。
3. 抽出验证纯逻辑后，能立刻增加测试保护。
4. 解决后再 gate legacy eframe，会更有信心。

推荐第一批代码任务：

1. 在 `app_runtime.rs` 增加 `BoxNameValidation` 与测试。
2. `CreateBoxDialogView` 改为使用该验证结构渲染提示和控制确认按钮。
3. 打开窗口后 focus input。
4. 手动验证 ESC 在输入框焦点下是否可取消；若不行，再补 action/key binding。
5. 跑：
   - `cargo fmt --check`
   - `cargo check`
   - `cargo test gpui_app`
   - `cargo test`
