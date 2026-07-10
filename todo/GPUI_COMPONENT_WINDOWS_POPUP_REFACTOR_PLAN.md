# gpui-component Windows 弹窗重构技术调研、可行性分析与实施计划

> 目标：只针对 Windows 平台，使用 `gpui-component` 重构当前项目中的两个弹窗/窗口：
>
> 1. 控制面板弹窗：`Desktop Organizer 控制面板`
> 2. 创建分类框弹窗：`创建分类框`
>
> 硬性约束：重构后功能、状态流转、触发入口、窗口位置、关闭/取消/确认行为、校验规则、托盘/信号文件联动等交互行为必须与现有版本保持一致。
>
> 本文档仅制定可行计划，不直接实施代码重构。

---

## 1. 调研结论摘要

### 1.1 推荐方案

推荐采用 **Windows-only 的 GPUI 主 UI 壳迁移方案**：

- 在 Windows 构建中引入 `gpui = 0.2.2` 与 `gpui-component = 0.5.1`。
- 将当前 `eframe/egui` 驱动的控制面板 root viewport 与创建分类框 viewport 迁移为 GPUI 原生窗口。
- 使用 `gpui-component` 的 `Root`、`Button`、`Input`、布局/样式扩展等组件重建两处 UI。
- 继续复用现有 Windows Shell、桌面覆盖层、分类框网格窗口、托盘、注册表、信号文件、Win32 窗口辅助函数等业务能力。
- 将 UI 操作提炼成与 UI 框架无关的 command/state 层，降低 GPUI 迁移风险。

### 1.2 不推荐方案

不推荐在同一进程中长期并行运行 `eframe/winit` 与 GPUI 两套 GUI runtime：

- 当前控制面板就是 `eframe::run_native` 的 root window；只替换“弹窗”而保留主 event loop，无法干净替换控制面板。
- 两套 GUI runtime 都倾向拥有自己的事件循环和窗口调度机制，在 Windows 上并存会引入窗口生命周期、线程、焦点、DPI、消息循环和 repaint 唤醒问题。
- 即使短期可跑，也会导致维护成本和不确定性显著上升。

### 1.3 重要边界

`gpui-component::Dialog` 是 GPUI 窗口内部 overlay 弹层，不是独立 OS 窗口。当前“创建分类框”是一个独立 OS viewport/window，具有独立标题栏、Win32 定位、X 关闭、非 resizable、always-on-top 等语义。为了保持交互一致，推荐 **不要直接用 `gpui-component::Dialog` 替换创建分类框**，而应使用 `gpui-component` 的输入框、按钮和布局组件构建一个独立 GPUI `Window`。

如果未来产品明确接受“创建分类框”改成控制面板内部 overlay，那么可以再考虑 `Dialog`，但这不满足本次“交互行为保持一致”的要求。

---

## 2. 当前代码与行为盘点

### 2.1 当前 GUI 技术栈

当前 `Cargo.toml`：

```toml
egui = "0.27"
eframe = { version = "0.27", default-features = false, features = ["glow"] }
epaint = "0.27"

[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
    ...
] }
```

当前两个目标窗口均由 `eframe/egui` 渲染：

- 控制面板：`eframe::run_native` root viewport。
- 创建分类框：`egui_ctx.show_viewport_immediate(...)` 创建的 secondary viewport。

关键文件：

- `D:\github-project\ikun_organizer\src\app.rs`
- `D:\github-project\ikun_organizer\src\main.rs`
- `D:\github-project\ikun_organizer\src\platform\windows\control_panel_window.rs`
- `D:\github-project\ikun_organizer\src\platform\windows\mod.rs`
- `D:\github-project\ikun_organizer\Cargo.toml`

### 2.2 控制面板当前行为

常量：

```rust
pub const CONTROL_PANEL_TITLE: &str = "Desktop Organizer 控制面板";
```

当前 `main.rs` 使用 `eframe::NativeOptions` 创建 root viewport：

- 标题：`Desktop Organizer 控制面板`
- 初始尺寸：`320 x 380`
- 初始位置：`[-32000, -32000]`
- decorations：true
- always_on_top：true
- resizable：true
- visible：true
- taskbar：false

实际生命周期：

1. 程序启动时创建 root window，但停放到屏幕外。
2. `MyApp::hide_control_panel()`：
   - `panel_visible = false`
   - viewport 发送 `OuterPosition([-32000, -32000])`
   - 调用 Win32 helper `hide_control_panel_window(CONTROL_PANEL_TITLE)`
3. `MyApp::open_control_panel()`：
   - `panel_visible = true`
   - 设置 inner size 为 `320 x 380`
   - Visible true
   - Focus
   - 调用 Win32 helper `show_control_panel_window(CONTROL_PANEL_TITLE)`
4. 用户点窗口 X 时：
   - 如果不是 `quit_requested`，拦截 close request
   - 发送 `CancelClose`
   - 改为隐藏窗口，而不是退出应用

控制面板 UI 与按钮行为：

- 标题：`📝 Desktop Organizer`（现有源码里可能因终端编码显示为乱码，但运行时语义应保持）
- 按钮：
  - `🔄 一键分类` -> `perform_auto_classify()`
  - `🗑 清除 & 恢复` -> `clear_and_restore()`
  - `💾 保存配置` -> `save_layout_and_config()`
  - `➕ 创建分类框` -> `open_create_box_dialog()`
- 状态消息：绿色显示 `ctx.state.status_message`
- 原生图标隐藏提示：黄色显示
- 当前格子数：`get_grid_count()`
- 底部说明文本保持一致

Windows helper `control_panel_window.rs` 当前职责：

- `hide_control_panel_window(title)`：
  - `FindWindowW` 按标题查找窗口
  - 添加 `WS_EX_TOOLWINDOW`
  - 移除 `WS_EX_APPWINDOW`
  - `SetWindowPos(-32000, -32000, ...)` 停放到屏幕外
- `show_control_panel_window(title)`：
  - 配置为 tool window
  - 根据鼠标所在 monitor 的 work area 居中
  - `ShowWindow(SW_SHOW)`
  - `SetForegroundWindow`
- `centered_window_position()` 已有单元测试

### 2.3 创建分类框当前行为

常量：

```rust
const BOX_NAME_MAX_LEN: usize = 16;
```

状态结构：

- `DialogState`
  - `name: String`
  - `open_state: DialogOpenState`
  - `confirmed: bool`
  - `cancelled: bool`
  - `target_pos: Option<(i32, i32)>`
- `DialogOpenState`
  - `Closed`
  - `Open`
  - `Closing`

触发入口：

- 控制面板按钮 `➕ 创建分类框`
- 系统托盘 action `SystemTrayAction::CreateBox`
- 信号文件 `ikun_create_box.signal`
- 分类框右键菜单 `take_pending_create_box()`

打开行为 `open_create_box_dialog()`：

1. 调用 `get_cursor_monitor_work_area_phys()` 获取鼠标所在显示器工作区，使用物理像素虚拟桌面坐标。
2. 估算窗口尺寸：
   - 宽度：`360`
   - 高度：`200`（包含标题栏的物理像素估算）
3. 目标位置：
   - `x = work_x + (work_w - 360) / 2`
   - `y = work_y + (work_h - 200) / 3`
4. 重置状态：
   - 清空 name
   - `open_state = Open`
   - `confirmed = false`
   - `cancelled = false`
   - `target_pos = Some((x, y))`

渲染行为 `render_create_box_dialog()`：

- 使用 `egui_ctx.show_viewport_immediate(...)`
- viewport id：`create_box_dialog`
- title：`创建分类框`
- inner size：`360 x 180`
- decorations：true
- resizable：false
- always_on_top：true
- target_pos 存在时调用 `reposition_window_by_title("创建分类框", tx, ty)`

UI 行为：

- 标题：`创建分类框`
- label：`名称：`
- 单行输入框：宽 `220`，hint `必填，最多16字符`
- 名称校验：
  - trim 后为空：无效，灰色提示 `名称不能为空`
  - 字符数 > 16：无效，红色提示 `⚠ 最多 16 字符（当前 N）`
  - 有效：绿色提示 `✓ N/16`
- `确定` 按钮仅在有效时可点击
- `取消` 按钮关闭
- Enter：输入框失焦且 Enter -> 确认
- ESC：取消
- OS 标题栏 X：取消
- 关闭后进入 `Closing`，下一帧变为 `Closed`，不再渲染 viewport

确认后行为：

1. 主 update 读取 `state.confirmed`。
2. 对 name 执行 `trim`。
3. 校验非空且字符数 `<= 16`。
4. 调用 `create_empty_box(name)`。

`create_empty_box(name)` 当前核心行为：

- `ensure_desktop_overlay_running()`
- 使用 config 默认宽高
- 按屏幕中心 + existing offset 计算新 box 位置
- 调用 `create_grid_window(params)`
- 成功后收集布局、合并 overlay config、保存配置
- 设置状态消息：`已创建分类框「...」`

当前已有相关测试：

- `signal_file_path`
- `is_box_name_valid`
- `DialogState` 默认状态
- `open_create_box_dialog()` 重置字段
- 状态机 `Closed -> Open`、`Open -> Closing`
- `target_pos` 非空
- 名称最大长度和中文字符计数

---

## 3. gpui-component 与 GPUI 技术调研

### 3.1 版本与来源

通过 `cargo info` 调研到当前可用版本：

- `gpui-component = 0.5.1`
  - crates.io: `https://crates.io/crates/gpui-component/0.5.1`
  - docs.rs: `https://docs.rs/gpui-component`
  - repository: `https://github.com/longbridge/gpui-component`
  - homepage: `https://longbridge.github.io/gpui-component`
- `gpui = 0.2.2`
  - crates.io: `https://crates.io/crates/gpui/0.2.2`
  - docs.rs: `https://docs.rs/gpui/0.2.2`
  - repository: `https://github.com/zed-industries/zed`
  - homepage: `https://gpui.rs`

本地 registry 源码路径：

- `C:\Users\chenzhihui\.cargo\registry\src\...\gpui-component-0.5.1`
- `C:\Users\chenzhihui\.cargo\registry\src\...\gpui-0.2.2`

`gpui-component` features：

- `decimal`
- `inspector`
- `tree-sitter-languages`
- `webview`

本次弹窗重构不需要 `webview`、`tree-sitter-languages` 或 `inspector`。

### 3.2 GPUI 应用模型

README 基础模式：

```rust
use gpui::*;
use gpui_component::{button::*, *};

pub struct HelloWorld;

impl Render for HelloWorld {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .size_full()
            .items_center()
            .justify_center()
            .child("Hello, World!")
            .child(
                Button::new("ok")
                    .primary()
                    .label("Let's Go!")
                    .on_click(|_, _, _| println!("Clicked!")),
            )
    }
}

fn main() {
    let app = Application::new();

    app.run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| HelloWorld);
                cx.new(|cx| Root::new(view, window, cx))
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
```

关键点：

- 必须调用 `gpui_component::init(cx)`。
- 窗口内容推荐包在 `gpui_component::Root` 中，以获得组件库上下文能力。
- GPUI 是 retained/entity 风格 UI，不是 egui immediate-mode；需要将当前“每帧 render + 共享 Mutex 状态”重构为 entity/update/subscription 模型。

### 3.3 GPUI 窗口能力

`gpui::WindowOptions` 关键字段：

```rust
pub struct WindowOptions {
    pub window_bounds: Option<WindowBounds>,
    pub titlebar: Option<TitlebarOptions>,
    pub focus: bool,
    pub show: bool,
    pub kind: WindowKind,
    pub is_movable: bool,
    pub is_resizable: bool,
    pub is_minimizable: bool,
    pub display_id: Option<DisplayId>,
    pub window_background: WindowBackgroundAppearance,
    pub app_id: Option<String>,
    pub window_min_size: Option<Size<Pixels>>,
    pub window_decorations: Option<WindowDecorations>,
    pub tabbing_identifier: Option<String>,
}
```

`WindowBounds`：

```rust
pub enum WindowBounds {
    Windowed(Bounds<Pixels>),
    Maximized(Bounds<Pixels>),
    Fullscreen(Bounds<Pixels>),
}
```

`WindowKind`：

```rust
pub enum WindowKind {
    Normal,
    PopUp,
    Floating,
}
```

可用窗口方法/能力：

- `window.remove_window()`：关闭窗口
- `window.activate_window()`：激活/聚焦窗口
- `window.resize(size)`：调整尺寸
- `window.bounds()`：获取窗口 bounds
- `window.set_window_title(title)`：设置标题
- `window.on_window_should_close(...)`：拦截关闭请求，返回 false 可阻止关闭
- `window.refresh()`：刷新

### 3.4 gpui-component Dialog 能力及限制

`gpui_component::dialog::Dialog` 支持：

- `Dialog::new(window, cx)`
- `title(...)`
- `confirm()` / `alert()`
- `on_ok(...) -> bool`
- `on_cancel(...) -> bool`
- `on_close(...)`
- `close_button(bool)`
- `margin_top(...)`
- `width(...)`
- `overlay(bool)`
- `overlay_closable(bool)`
- `keyboard(bool)`

`WindowExt` 支持：

```rust
fn open_dialog<F>(&mut self, cx: &mut App, build: F)
where
    F: Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static;

fn close_dialog(&mut self, cx: &mut App);
fn close_all_dialogs(&mut self, cx: &mut App);
```

限制：

- 该 Dialog 是 GPUI root window 内的 overlay，不是独立 OS window。
- 当前创建分类框具有独立标题栏和 OS 窗口级别的定位/关闭语义，因此若直接替换为 overlay，会改变用户可见交互。

结论：

- 控制面板：用 `gpui-component` Button/布局重建一个 GPUI window。
- 创建分类框：用 `gpui-component` Input/Button/布局重建另一个 GPUI window。
- 暂不使用 `gpui_component::Dialog` 作为主实现；可作为未来产品形态变更时的候选。

### 3.5 gpui-component Input 能力

`InputState` 与 `Input` 可满足当前创建分类框输入需求：

- `InputState::new(window, cx)`
- `.placeholder("必填，最多16字符")`
- `.default_value(...)`
- `.set_value(...)`
- `.value() -> SharedString`
- 事件：
  - `InputEvent::Change`
  - `InputEvent::PressEnter { secondary }`
  - `InputEvent::Focus`
  - `InputEvent::Blur`

可行模式：

```rust
use gpui::*;
use gpui_component::{
    button::*,
    input::{Input, InputEvent, InputState},
    Root,
    StyledExt,
};

struct CreateBoxView {
    input: Entity<InputState>,
    _sub: Subscription,
}

impl CreateBoxView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("必填，最多16字符")
        });
        let _sub = cx.subscribe(&input, |_, input, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                let value = input.read(cx).value();
                // 更新校验状态
            }
        });
        Self { input, _sub }
    }
}
```

注意：

- `v_flex`、`gap_2`、`w_64` 等样式扩展需要 `use gpui_component::StyledExt;`。
- Enter 行为可通过 `InputEvent::PressEnter` 实现。
- ESC 行为需要在窗口或视图层注册 key binding/action，或通过 GPUI 的 input/action 机制处理；需要在实现阶段做最小原型验证。

### 3.6 Windows 编译可行性

已做过最小探针编译：

- 临时 probe：`D:\github-project\ikun_organizer\target\gpui_component_probe`
- 使用 `gpui-component::Root`、`Button`、`Input`、`InputState`、`StyledExt` 的最小窗口可编译。

依赖注意事项：

- `gpui = 0.2.2` 默认 features 包含 `font-kit`、`wayland`、`x11`、`windows-manifest`。
- `gpui-component 0.5.1` 对 `gpui` 的依赖未显式关闭 default features，因此即使本项目直接依赖 `gpui` 并设置 `default-features = false`，也可能因 feature unification 被 `gpui-component` 重新启用默认 features。
- 已知最小 probe 在 Windows 环境可编译，因此这不是阻断项，但正式接入时需要用 `cargo tree -e features` 复核依赖体积和 Windows 构建表现。
- 若后续构建体积/时间不可接受，可评估 fork/patch `gpui-component` 的 `Cargo.toml`，但这会增加维护成本，不作为第一阶段目标。

---

## 4. 方案对比与可行性分析

### 4.1 方案 A：Windows-only 全 UI 壳迁移到 GPUI（推荐）

做法：

- Windows 入口从 `eframe::run_native` 切换到 `gpui::Application::new().run(...)`。
- 控制面板与创建分类框均由 GPUI window 承载。
- 业务模块继续复用现有代码。
- 只针对 Windows 实施；非 Windows 路径不纳入本次计划。

优点：

- 一个进程只保留一个 GUI runtime，避免事件循环冲突。
- 可以完整使用 `gpui-component`，符合用户诉求。
- 控制面板和创建分类框窗口可以统一管理生命周期。
- 长期架构更清晰。

缺点/风险：

- 需要重构主 app loop、状态更新和 repaint/timer 机制。
- 当前 `eframe::App::update` 中的轮询逻辑需要迁移为 GPUI 的 timer/task/waker 模式。
- GPUI Windows 窗口级能力与 eframe viewport 命令不完全一一对应，仍需保留部分 Win32 helper。

可行性结论：

- 技术上可行，且是满足“两个弹窗都用 gpui-component 重构 + 行为一致”的最稳妥方案。
- 建议采用 feature/模块隔离与分阶段验收，避免一次性大爆炸改造。

### 4.2 方案 B：eframe 主循环 + GPUI 子窗口混合运行（不推荐）

做法：

- 保留 `eframe` 主 app。
- 尝试为两个弹窗单独启动 GPUI window/runtime。

问题：

- 控制面板本身是当前 eframe root window，无法只作为“子窗口”替换。
- GPUI 与 eframe/winit 都需要处理 Windows 消息循环、DPI、窗口生命周期和 repaint。
- 同进程双 runtime 易出现焦点、退出、线程和消息泵问题。

可行性结论：

- 不建议作为正式实现。
- 只可作为极短期实验，不应进入主分支。

### 4.3 方案 C：独立 GPUI helper 进程承载弹窗（备选，不推荐优先）

做法：

- 主程序保持 eframe。
- 另起 helper binary，用 GPUI 渲染控制面板和创建分类框。
- 主进程与 helper 通过 IPC/信号文件通信。

优点：

- 隔离两个 GUI runtime。
- 主程序改动较少。

缺点：

- 控制面板按钮需要触发主进程业务命令，必须设计 IPC。
- 创建分类框确认后要调用 `create_empty_box`，涉及 AppContext、布局、配置、overlay 运行状态，IPC 同步复杂。
- 窗口焦点、退出、崩溃恢复、单实例管理都会变复杂。

可行性结论：

- 技术可行但不经济。
- 除非 GPUI 全迁移遇到阻断，否则不建议。

---

## 5. 推荐架构设计

### 5.1 目标模块边界

建议新增/调整模块：

```text
src/
  main.rs
  app.rs                         # 逐步瘦身；保留/抽出业务逻辑
  ui/
    mod.rs
    commands.rs                  # UI 无关命令入口与状态转换
    windows_gpui/
      mod.rs
      runtime.rs                 # GPUI Application 启动与主调度
      control_panel.rs           # GPUI 控制面板 window/view
      create_box_dialog.rs       # GPUI 创建分类框 window/view
      state.rs                   # GPUI UI state/entity bridge
  platform/
    windows/
      control_panel_window.rs    # 泛化为 tool window/定位 helper
      ...                        # 现有 shell/grid/tray 模块继续复用
```

说明：

- 文件名可在实现时根据现有风格微调。
- 如果希望最小侵入，也可以先放在 `src/gpui_windows/`，待稳定后再整理到 `src/ui/windows_gpui/`。

### 5.2 状态与命令层

当前 `MyApp` 同时承担：

- eframe app lifecycle
- 控制面板 UI render
- 创建分类框 UI render
- 业务命令
- 信号文件轮询
- 托盘 action 消费
- 关闭/退出控制

建议先抽出 UI 无关 command：

```rust
pub enum UiCommand {
    OpenControlPanel,
    HideControlPanel,
    OpenCreateBoxDialog,
    ConfirmCreateBox { name: String },
    CancelCreateBoxDialog,
    AutoClassify,
    ClearAndRestore,
    SaveLayoutAndConfig,
    Quit,
}
```

并抽出状态快照：

```rust
pub struct ControlPanelSnapshot {
    pub panel_visible: bool,
    pub status_message: String,
    pub icons_hidden: bool,
    pub grid_count: usize,
}

pub struct CreateBoxDialogState {
    pub name: String,
    pub open_state: DialogOpenState,
    pub target_pos: Option<(i32, i32)>,
}
```

推荐策略：

- 业务 command 继续操作现有 `AppContext`。
- UI 层只读 snapshot 并发送 command。
- `confirm_create_box(name)` 内部统一执行 trim 与 `BOX_NAME_MAX_LEN` 校验，并调用现有创建逻辑。
- `is_box_name_valid()` 保留为纯函数，供 GPUI 和测试共用。

### 5.3 GPUI runtime 设计

Windows `main.rs` 建议结构：

```rust
#[cfg(target_os = "windows")]
fn main() -> anyhow::Result<()> {
    init_logging();
    platform::windows::init_dpi_awareness();
    platform::windows::init_ole();

    ui::windows_gpui::runtime::run_windows_gpui_app()
}

#[cfg(not(target_os = "windows"))]
fn main() -> anyhow::Result<()> {
    // 本次不要求非 Windows；可保留旧 eframe 或直接 compile_error，按项目策略定。
}
```

GPUI 启动：

```rust
let app = gpui::Application::new();
app.run(move |cx| {
    gpui_component::init(cx);
    // 初始化业务 AppContext
    // 创建控制面板 window（初始隐藏或 offscreen）
    // 注册托盘 action waker / signal polling / timer
});
```

注意事项：

- GPUI `Application::run` 拥有主事件循环；不要再调用 `eframe::run_native`。
- 托盘 action waker 当前调用 `egui_ctx.request_repaint()`，需要改成调用 GPUI 的 weak app/entity update 或 channel 唤醒。
- 当前 `eframe::App::update` 中每帧轮询的逻辑需要迁移为定时任务，例如每 100-250ms tick：
  - consume tray action
  - check signal files
  - check native icon hide retry
  - sync UI snapshot

### 5.4 控制面板 GPUI window 设计

窗口要求：

- 标题：`Desktop Organizer 控制面板`
- 内容尺寸：`320 x 380`
- decorations：true
- resizable：true
- always-on-top：等价或尽量等价
- 不显示任务栏：保留 Win32 `WS_EX_TOOLWINDOW` 方案
- 启动时隐藏/停放屏幕外
- 打开时居中到鼠标所在显示器工作区并聚焦
- 点 X 不退出，改为隐藏

GPUI 实现要点：

1. `WindowOptions`：
   - `window_bounds = Some(WindowBounds::Windowed(...))`
   - 初始 bounds 可设为 `(-32000, -32000, 320, 380)`
   - `show = true` 或先 `show = false` 后通过 helper 显示，需实现阶段验证哪种更稳定
   - `kind = WindowKind::Floating` 或 `WindowKind::PopUp`，需验证在 Windows 上与 taskbar/always-on-top 的实际效果
   - `is_resizable = true`
2. window title 设置为 `CONTROL_PANEL_TITLE`。
3. `on_window_should_close`：
   - 如果不是 quit，调用 hide command 并返回 `false`。
   - 如果 quit，允许关闭。
4. Win32 helper：
   - 继续通过标题查找 hwnd，配置 `WS_EX_TOOLWINDOW`，移除 `WS_EX_APPWINDOW`。
   - 打开时 `center_on_cursor_monitor` + `ShowWindow` + `SetForegroundWindow`。
   - 隐藏时 `park_offscreen`，必要时 `ShowWindow(SW_HIDE)` 也可评估，但要保证下次 show/focus 稳定。
5. UI 内容：
   - 用 `gpui-component::button::Button` 重建四个按钮。
   - 用 layout `div().v_flex().gap_...` 重建现有布局。
   - 颜色状态：绿色状态消息、黄色原生图标隐藏提示。
   - 保持文案和按钮顺序一致。

### 5.5 创建分类框 GPUI window 设计

窗口要求：

- 独立 OS window，而不是 overlay。
- 标题：`创建分类框`
- 内容尺寸：`360 x 180`
- 位置估算使用当前算法：以 `360 x 200` 作为包含标题栏的物理像素估算。
- decorations：true
- resizable：false
- always-on-top：等价或尽量等价
- 打开时清空输入并聚焦
- 支持 Enter 确认、ESC 取消、X 取消、取消按钮、确定按钮
- 确定按钮仅在有效时 enabled
- 校验文案与颜色保持一致
- 确认后调用现有 `create_empty_box(name)` 等价逻辑

GPUI 实现要点：

1. `open_create_box_dialog()` 仍先计算 target position：

```rust
let (work_x, work_y, work_w, work_h) = get_cursor_monitor_work_area_phys();
let dlg_w = 360i32;
let dlg_h = 200i32;
let x = work_x + (work_w - dlg_w) / 2;
let y = work_y + (work_h - dlg_h) / 3;
```

2. 若 dialog window 不存在：
   - `cx.open_window(...)` 创建新 GPUI window。
   - bounds 设置为 `(x, y, 360, 180)` 或创建后调用 Win32 `reposition_window_by_title("创建分类框", x, y)` 二次校正。
3. 若 dialog window 已存在：
   - 清空 input。
   - 激活/置前。
   - 重新定位到当前鼠标所在显示器的目标位置。
4. `CreateBoxDialogView` 持有：
   - `input: Entity<InputState>`
   - 当前字符数/有效性可从 input value 即时计算，或维护在 view state 中。
   - `_sub: Subscription` 订阅 `InputEvent::Change` 和 `InputEvent::PressEnter`。
5. Enter 行为：
   - 在 `InputEvent::PressEnter` 中，如果当前 name valid，则 dispatch `ConfirmCreateBox { name }` 并关闭 window。
   - 现有 egui 代码是“TextEdit lost focus && Enter”，最终用户感知是按 Enter 可确认；GPUI 实现以输入框 PressEnter 确认为准。
6. ESC 行为：
   - 通过 GPUI key/action 处理；若遇到 API 限制，可在第一阶段优先实现 X/取消按钮，再专门补 ESC。
   - 验收前必须保证 ESC 取消。
7. X 关闭：
   - `on_window_should_close` 中设置 cancel/close state，允许关闭或手动 `remove_window`。
   - 语义必须是取消，不触发创建。
8. 关闭后状态：
   - GPUI 可不再需要 `Closing` 中间态，但为了测试和兼容 command 层，可保留 `Open/Closing/Closed` 纯状态。
   - GPUI window 真正 remove 后将 state 置为 `Closed`。

---

## 6. 详细实施阶段计划

### 阶段 0：建立迁移保护线

目标：保证迁移前后行为可对照。

任务：

1. 新建开发分支，例如：`codex/gpui-component-windows-popups`。
2. 运行并记录当前测试基线：
   - `cargo test`
   - 如已有 Windows 手动测试脚本，一并记录。
3. 建立手动行为核对表：
   - 启动后控制面板不在任务栏显示。
   - 托盘打开控制面板。
   - 控制面板 X 按钮隐藏而非退出。
   - 创建分类框从控制面板/托盘/信号文件/右键入口均可打开。
   - 输入校验、Enter、ESC、X、取消、确认行为一致。
   - 多显示器与 DPI 缩放下窗口位置合理。
4. 不恢复或覆盖已有被删除的 `todo/IMPLEMENTATION_PLAN.md`，避免破坏现有未提交变更。

交付物：

- 当前行为清单与测试基线。

### 阶段 1：依赖接入与最小 GPUI Windows 原型

目标：证明本项目内可编译、可启动 GPUI window，且能使用 `gpui-component`。

任务：

1. 在 `Cargo.toml` Windows target 下加入：

```toml
[target.'cfg(windows)'.dependencies]
gpui = "0.2.2"
gpui-component = "0.5.1"
```

> 可先不关闭 default features，因为 `gpui-component` 当前会启用 `gpui` 默认 features；后续再优化。

2. 新增临时/feature-gated 原型模块：
   - `src/ui/windows_gpui/runtime.rs`
   - `src/ui/windows_gpui/control_panel.rs`
3. 实现一个只含标题和按钮的 GPUI window。
4. 使用 `gpui_component::init(cx)` 与 `Root::new(view, window, cx)`。
5. 编译验证：
   - `cargo check --target x86_64-pc-windows-msvc`（如果本机默认即 Windows MSVC，可直接 `cargo check`）
   - `cargo tree -e features | findstr gpui`

验收标准：

- 项目可编译。
- GPUI window 可启动。
- `Button`、`Input` 等 gpui-component 组件可正常渲染。

### 阶段 2：抽离 UI 无关 command/state 层

目标：把当前 egui UI 与业务逻辑解耦，为 GPUI 替换做准备。

任务：

1. 从 `src/app.rs` 抽出纯函数/业务函数：
   - `is_box_name_valid(name)`
   - 创建分类框目标位置计算函数，例如 `calculate_create_box_dialog_position(work_area)`
   - 控制面板快照生成函数
2. 抽出 command dispatcher：
   - `perform_auto_classify`
   - `clear_and_restore`
   - `save_layout_and_config`
   - `create_empty_box`
   - `open_create_box_dialog` 的状态更新部分
3. 保持旧 egui UI 调用 command，确保抽离后行为不变。
4. 补充/迁移单元测试：
   - 名称校验
   - 位置计算
   - dialog open 重置状态
   - confirm trim 行为
   - cancel 不创建

验收标准：

- 旧 egui 路径仍通过测试。
- 业务逻辑不再强依赖 `egui::Context`。
- GPUI 后续只需要调用 command，不复制业务逻辑。

### 阶段 3：实现 GPUI 控制面板 window

目标：用 GPUI/gpui-component 重建控制面板 UI 并保持行为一致。

任务：

1. 新建 `ControlPanelView`：
   - 使用 `gpui_component::button::Button`
   - 使用 `StyledExt` 组织布局
   - 从 snapshot 渲染状态消息、icons_hidden、grid_count
2. 接入按钮命令：
   - `AutoClassify`
   - `ClearAndRestore`
   - `SaveLayoutAndConfig`
   - `OpenCreateBoxDialog`
3. 实现窗口打开/隐藏：
   - `open_control_panel()`：更新状态、调整 size、show、focus、调用 Win32 show helper。
   - `hide_control_panel()`：更新状态、park offscreen、调用 Win32 hide helper。
4. 实现 close request 拦截：
   - 非 quit：隐藏并返回 false。
   - quit：允许关闭。
5. 保留或泛化 `control_panel_window.rs`：
   - 函数名可从 `egui control-panel` 语义改为更通用的 `configure_tool_window_by_title`、`show_tool_window_centered`、`park_tool_window_offscreen`。

验收标准：

- 控制面板外观/文案/按钮顺序与旧版本一致。
- 托盘打开控制面板正常。
- X 按钮隐藏，不退出。
- 不显示任务栏图标。
- 打开时在鼠标所在显示器居中并置前。

### 阶段 4：实现 GPUI 创建分类框 window

目标：用 GPUI/gpui-component 重建创建分类框独立窗口，并保持行为一致。

任务：

1. 新建 `CreateBoxDialogView`：
   - `InputState` + `Input`
   - `Button::new("ok")` / `Button::new("cancel")`
   - 校验提示 label
2. 实现打开逻辑：
   - 每次打开清空输入。
   - 重新计算 target position。
   - 创建或复用窗口。
   - 调用 `reposition_window_by_title("创建分类框", x, y)` 做 Win32 物理像素校正。
3. 实现确认逻辑：
   - `确定` enabled 条件：trim 非空 && chars <= 16。
   - 点击确定：dispatch `ConfirmCreateBox { name }`，关闭窗口。
   - `InputEvent::PressEnter`：同确定。
4. 实现取消逻辑：
   - 点击取消：关闭窗口，不创建。
   - ESC：关闭窗口，不创建。
   - X：关闭窗口，不创建。
5. 实现重复打开逻辑：
   - 如果窗口已打开，再次触发时应清空输入、置前、按当前鼠标 monitor 重新定位。
   - 防止创建多个同名 dialog window。
6. 保留状态机测试：
   - GPUI 实现内部可简化，但 command 层保留 `Open/Closing/Closed` 或等价状态以便测试。

验收标准：

- 四个入口都能打开创建分类框。
- 名称校验提示颜色和文案一致。
- 16 字符上限按 `chars().count()`，中文按字符计数。
- 空白输入不能确认。
- Enter 与确定按钮行为一致。
- ESC/X/取消均不创建分类框。
- 确认后创建分类框并保存配置，状态消息一致。

### 阶段 5：替换 Windows 主入口并移除 egui 弹窗路径

目标：Windows 构建实际使用 GPUI UI 壳。

任务：

1. 修改 `src/main.rs`：
   - Windows 调用 `run_windows_gpui_app()`。
   - 非 Windows 路径按项目策略保留旧代码或暂不支持。
2. 将原 `eframe::App` 路径 feature-gate 或删除：
   - 若项目只支持 Windows，可直接逐步删除 eframe UI。
   - 若希望保留回滚能力，可加 feature `legacy-egui-ui`。
3. 检查 `Cargo.toml`：
   - 若 Windows 已完全迁移且非 Windows 不需要 eframe，可移除全局 egui/eframe/epaint 依赖或放到 legacy feature。
   - 第一版可暂保留，降低迁移风险。
4. 将原 `egui_ctx.request_repaint()` 相关 waker 改为 GPUI 更新机制。
5. 确保退出流程：
   - 托盘 Quit
   - 信号文件 quit
   - 清理系统托盘
   - 清理注册表/overlay 相关资源

验收标准：

- Windows 默认启动 GPUI 版本。
- 旧 egui 控制面板和 egui create viewport 不再参与运行。
- 程序退出清理行为与旧版本一致。

### 阶段 6：测试、QA 与稳定性修复

自动测试：

- `cargo test`
- `cargo check`
- `cargo clippy -- -D warnings`（如项目当前能通过）
- `cargo fmt --check`

建议新增/保留测试：

1. 名称校验：
   - 空字符串
   - 全空格
   - 16 ASCII 字符
   - 17 ASCII 字符
   - 16 中文字符
   - 中英混合
2. 位置计算：
   - 主屏
   - 副屏负坐标
   - work area 小于默认窗口尺寸时的防御策略
3. command 行为：
   - confirm valid -> 创建命令被调用
   - confirm invalid -> 不创建
   - cancel -> 不创建
4. 控制面板 close 行为：
   - quit=false -> hide
   - quit=true -> close
5. signal/tray action 消费：
   - open panel signal
   - create box signal
   - quit signal

手动 QA 清单：

- 单显示器 100% DPI。
- 单显示器 150%/200% DPI。
- 双显示器，副屏在左侧导致虚拟桌面负坐标。
- 鼠标位于不同显示器时打开控制面板/创建框。
- Explorer 重启后 native icons hide retry 行为。
- 托盘菜单 Open/Create/Quit。
- 桌面右键菜单触发 create/open/quit。
- 创建分类框后布局保存与下次启动恢复。
- 不以管理员运行时拖拽/OLE 行为正常；以管理员运行时仍有现有 warning。

---

## 7. Windows 专项兼容性风险与缓解

### 7.1 任务栏隐藏/tool window

风险：

- GPUI `WindowOptions` 没有直接等价于 egui `.with_taskbar(false)` 的明确 API。
- `WindowKind::PopUp/Floating` 在 Windows 上是否完全避免任务栏图标，需要实测。

缓解：

- 继续使用现有 Win32 helper：添加 `WS_EX_TOOLWINDOW`、移除 `WS_EX_APPWINDOW`。
- GPUI window 标题必须稳定，以便 `FindWindowW` 查找。
- 必要时新增 hwnd 获取方式，减少标题查找脆弱性。

### 7.2 always-on-top 等价性

风险：

- GPUI API 中 `WindowKind::Floating/PopUp` 与 egui `with_always_on_top()` 并非严格同义。

缓解：

- 优先实测 `WindowKind::Floating` 与 `PopUp`。
- 若不能满足，新增 Win32 `SetWindowPos(HWND_TOPMOST, ...)` helper，并与现有 tool window helper 合并。

### 7.3 DPI 与多显示器定位

风险：

- 当前创建分类框位置用 Win32 物理像素校正，而 GPUI bounds 可能使用 logical/pixels 抽象。

缓解：

- 保留 `get_cursor_monitor_work_area_phys()` 与 `reposition_window_by_title(...)`。
- 创建 GPUI window 后用 Win32 二次定位，保证与旧行为一致。
- 重点测试多屏、负坐标、不同 DPI。

### 7.4 事件循环与托盘唤醒

风险：

- 当前 `register_tray_action_waker` 直接绑定 egui repaint。
- GPUI 需要用自己的 app/entity update 唤醒。

缓解：

- 将 tray action 放入线程安全队列，GPUI 定时 tick 或 waker 消费。
- 第一版可用 100-250ms timer 轮询，后续再优化为事件驱动。

### 7.5 输入法与中文输入

风险：

- 创建分类框名称主要是中文场景，需要验证 GPUI Input 在 Windows 中文 IME 下的表现。

缓解：

- 将中文输入作为手动 QA 必测项。
- 若发现 composition 问题，优先查 gpui/gpui-component issue 或降级为原生 Win32 edit 控件方案；但这会偏离 gpui-component，应作为最后手段。

### 7.6 依赖体积与构建稳定性

风险：

- `gpui-component` 启用 `gpui` 默认 features，可能拉入非 Windows 相关依赖 feature。

缓解：

- 接入后运行 `cargo tree -e features`。
- 若 build 时间/体积不可接受，再评估 patch/fork；第一版不建议一开始 fork。

---

## 8. 验收标准

### 8.1 功能验收

- 控制面板：
  - 托盘/信号文件可打开。
  - 初始不扰民，不显示任务栏入口。
  - X 按钮隐藏而非退出。
  - 四个按钮功能与旧版本一致。
  - 状态消息、图标隐藏提示、格子数显示一致。
- 创建分类框：
  - 控制面板按钮、托盘、信号文件、分类框右键入口均可打开。
  - 打开时清空输入。
  - 位置与旧算法一致。
  - 非 resizable、带标题栏、always-on-top 行为一致或通过 Win32 helper 达成。
  - 输入校验、颜色、文案一致。
  - Enter 确认，ESC/X/取消取消。
  - 确认后创建分类框、保存配置、状态消息一致。

### 8.2 技术验收

- Windows 默认路径使用 GPUI/gpui-component 渲染两个目标窗口。
- 不在同一运行路径中同时启动 eframe event loop 与 GPUI event loop。
- 业务逻辑不复制，创建/保存/恢复仍走现有 AppContext 与 platform 模块。
- 单元测试覆盖名称校验、状态机、位置计算、command 行为。
- 手动 QA 覆盖多 DPI、多显示器、中文输入、托盘/信号文件。

### 8.3 回归验收

- `cargo test` 通过。
- `cargo check` 通过。
- 无新增 panic/unwrap 热路径。
- 退出时系统托盘清理正常。
- 不破坏现有桌面 overlay/grid 分类框功能。

---

## 9. 建议实施顺序与预估复杂度

| 阶段 | 内容 | 复杂度 | 风险 | 建议 |
|---|---|---:|---:|---|
| 0 | 建立基线与手动 QA 表 | 低 | 低 | 必做 |
| 1 | 接入依赖与最小 GPUI 原型 | 中 | 中 | 必做，先验证 |
| 2 | 抽 command/state | 中 | 中 | 必做，降低后续风险 |
| 3 | GPUI 控制面板 | 中 | 中 | 先做，窗口少、逻辑直观 |
| 4 | GPUI 创建分类框 | 中高 | 中高 | 输入/ESC/定位需重点验证 |
| 5 | 替换 Windows main | 高 | 高 | 等 3/4 稳定后再切 |
| 6 | QA 与修复 | 中 | 中 | 多屏/DPI/IME 必测 |

整体评估：

- 可行性：高。
- 工程风险：中高，主要在 event loop 迁移、窗口 Win32 语义、DPI/IME。
- 推荐采用分阶段 PR/commit，避免一次性大改。

---

## 10. 第一轮实现时的最小代码骨架建议

### 10.1 `Cargo.toml`

```toml
[target.'cfg(windows)'.dependencies]
gpui = "0.2.2"
gpui-component = "0.5.1"
```

### 10.2 GPUI runtime 伪代码

```rust
pub fn run_windows_gpui_app() -> anyhow::Result<()> {
    let application = gpui::Application::new();

    application.run(move |cx| {
        gpui_component::init(cx);

        let app_state = Arc::new(Mutex::new(AppRuntime::new()));

        cx.spawn({
            let app_state = app_state.clone();
            async move |cx| {
                open_control_panel_window(app_state.clone(), cx)?;
                start_periodic_polling(app_state.clone(), cx);
                Ok::<_, anyhow::Error>(())
            }
        }).detach();
    });

    Ok(())
}
```

### 10.3 控制面板 view 伪代码

```rust
struct ControlPanelView {
    app: SharedAppRuntime,
}

impl Render for ControlPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.app.lock().unwrap().control_panel_snapshot();

        div()
            .v_flex()
            .gap_2()
            .child("📝 Desktop Organizer")
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .child(Button::new("auto").label("🔄 一键分类").on_click(...))
                    .child(Button::new("clear").label("🗑 清除 & 恢复").on_click(...))
                    .child(Button::new("save").label("💾 保存配置").on_click(...))
                    .child(Button::new("create").label("➕ 创建分类框").on_click(...))
            )
            // status / icons / count / footer
    }
}
```

### 10.4 创建分类框 view 伪代码

```rust
struct CreateBoxDialogView {
    app: SharedAppRuntime,
    input: Entity<InputState>,
    _sub: Subscription,
}

impl CreateBoxDialogView {
    fn confirm_if_valid(&self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).value().trim().to_string();
        if is_box_name_valid(&name) {
            self.app.lock().unwrap().confirm_create_box(name);
            window.remove_window();
        }
    }
}

impl Render for CreateBoxDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let value = self.input.read(cx).value();
        let cc = value.chars().count();
        let valid = is_box_name_valid(value.as_ref());

        div()
            .v_flex()
            .gap_2()
            .child("创建分类框")
            .child(
                div()
                    .h_flex()
                    .child("名称：")
                    .child(Input::new(&self.input).w_64())
            )
            .child(validation_label(cc, valid, value.trim().is_empty()))
            .child(
                div()
                    .h_flex()
                    .child(Button::new("ok").label("✓ 确定").disabled(!valid).on_click(...))
                    .child(Button::new("cancel").label("✗ 取消").on_click(...))
            )
    }
}
```

---

## 11. 最终建议

本次需求的关键不是简单替换控件，而是把当前 `eframe/egui` root window 与 secondary viewport 的生命周期完整迁移到 GPUI。为了同时满足“使用 gpui-component”和“行为保持一致”，建议：

1. 不走 eframe + GPUI 混合 runtime。
2. 不用 `gpui-component::Dialog` 承载创建分类框的主实现。
3. 采用 Windows-only GPUI 主 UI 壳迁移。
4. 保留 Win32 helper 来补齐 GPUI 在 taskbar/tool window/定位/置顶方面的窗口语义差异。
5. 先抽 command/state，再换 UI，最后切 main 入口。

这样能最大程度降低回归风险，并保持现有 Windows 桌面整理器的所有入口和交互语义。
