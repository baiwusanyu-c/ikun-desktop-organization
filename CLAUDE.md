# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

**桌面图标智能分类管理工具** — Desktop Organizer

基于 Rust 的跨平台桌面图标智能分类和管理工具。Windows 默认控制面板/弹窗入口已迁移到 **GPUI + gpui-component**，legacy `eframe/egui` 控制面板仅作为显式 `eframe-ui` feature 回退路径保留；分类框与桌面覆盖窗口继续使用平台原生窗口 + Direct2D/DirectWrite 渲染。

**核心特性**：智能分类桌面图标、可视化分类框（拖拽/缩放/锁定/样式自定义）、一键 AI 分类、布局记忆持久化、统一 JSON 配置导入导出。

**平台支持**：
- ✅ **Windows 10/11**：主要平台（Shell COM API + Direct2D/DirectWrite 渲染）
- 🔧 **macOS**：开发中（桌面覆盖窗口 + 分类框渲染 + 图标提取已实现，拖拽/菜单待完善）

---

## 构建和运行

```bash
cargo run                        # 开发模式运行（保留控制台窗口）
RUST_LOG=debug cargo run         # 带调试日志运行（推荐）
cargo build --release            # 发布构建（无控制台窗口，windows_subsystem="windows"）
cargo check                      # 快速编译检查
cargo fmt                        # 格式化
cargo clippy                     # Lint 检查
cargo test                       # 运行测试
cargo watch -x check             # 自动检查（需 cargo install cargo-watch）
```

**日志级别**：通过 `RUST_LOG` 环境变量控制（`trace` / `debug` / `info` / `warn` / `error`）

**控制台窗口**：所有构建均通过 `#![windows_subsystem = "windows"]` 禁用控制台，日志写入 `~/.ikun_organizer_log/` 目录（按日期和序号分文件）

---

## 技术栈

| Crate | 版本 | 用途 |
|-------|------|------|
| gpui / gpui-component | 0.2.2 / 0.5.1 | Windows 默认控制面板与独立创建分类框窗口 |
| egui / eframe / epaint | 0.27 | legacy 控制面板回退路径（`eframe-ui` feature） |
| windows | 0.58 | Windows API（Shell COM、Direct2D、DirectWrite、GDI、窗口管理） |
| cocoa / objc | 0.25 | macOS API（实验性） |
| serde / serde_json | 1.0 | 统一 JSON 配置序列化 |
| serde_yaml | 0.9 | YAML 分类规则（兼容旧格式） |
| anyhow | 1.0 | 错误处理 |
| log / env_logger | 0.4 / 0.11 | 日志系统 |
| uuid | 1.0 | 分类框 / 图标唯一 ID |
| chrono | 0.4 | 时间处理 |
| dirs | 5.0 | 跨平台配置目录 |
| notify | 6.0 | 文件系统监听（TODO） |

---

## 架构概述（v2.0）

### 核心原则

- **平台抽象**：`platform/` 通过 trait 统一接口，Windows/macOS 各自实现
- **业务分离**：`box_core/` 分类框业务逻辑与平台渲染完全解耦
- **上下文注入**：所有模块通过 `AppContext` 共享状态，替代全局变量
- **插件化**：`plugin/` 功能模块作为插件注册，灵活扩展
- **统一配置**：单一 `~/.ikun_organizer.json` 管理所有设置，支持导入/导出
- **单文件 ≤ 1500 行**：渲染引擎等复杂模块可适当放宽（如 d2d_render.rs ~1400 行），但应控制在合理范围内

### 项目目录结构

```
src/
├── main.rs                    # 入口：初始化日志、DPI/OLE、分发 GPUI 或 legacy eframe
├── app.rs                     # legacy eframe::App 控制面板（仅 eframe-ui 回退路径）
├── context.rs                 # AppContext 核心上下文对象
│
├── gpui_app/                  # Windows 默认 GPUI 控制面板与独立弹窗
│   ├── mod.rs                 #   模块导出
│   ├── app_runtime.rs         #   UI 框架无关运行时/命令层（AppContext + 平台服务）
│   ├── control_panel.rs       #   GPUI 控制面板视图
│   ├── create_box_dialog.rs   #   GPUI 独立“创建分类框”窗口
│   └── windows_main.rs        #   GPUI Application 事件循环与轮询调度
│
├── platform/                  # 平台抽象层
│   ├── mod.rs                 #   DesktopManagerTrait + re-exports
│   ├── types.rs               #   跨平台类型：DesktopIcon, IconInfo, ScreenInfo
│   ├── windows/
│   │   ├── mod.rs             #   Windows 平台入口
│   │   ├── ffi_types.rs       #   Win32 FFI 类型/常量
│   │   ├── ffi_imports.rs     #   Win32 extern "system" 声明
│   │   ├── desktop_manager.rs #   桌面图标枚举（文件系统 + 虚拟 Shell 对象 CLSID）
│   │   ├── desktop_overlay/   #   桌面覆盖窗口（渲染未管理图标 + Grid 吸附布局 + 桌面接管生命周期 + 文件监听刷新）
│   │   │   ├── overlay_mouse_hook.rs # 全局鼠标钩子（WH_MOUSE_LL + 独立线程消息泵，拦截覆盖区域右键）
│   │   ├── alignment_guide/   #   对齐参考线（对齐检测 + 动态对比色 + overlay 渲染）
│   │   ├── d2d_render.rs      #   D2D/DirectWrite 渲染引擎
│   │   ├── grid_window.rs     #   窗口创建/销毁/布局收集/Z-order 管理
│   │   ├── wnd_proc.rs        #   窗口消息处理 wnd_proc
│   │   ├── zorder.rs          #   Z-order 层级管理（分类框低于其他应用窗口 + 坐标查找）
│   │   ├── menu_window.rs     #   自定义分层窗口深色菜单（D2D/DirectWrite 渲染）
│   │   ├── shell_menu.rs      #   Shell 右键菜单 + 桌面背景注册表菜单（覆盖区域含 Ikun 动作，分类框空白处不含）
│   │   ├── app_icon.rs        #   嵌入 logo.ico 资源加载（托盘/菜单图标）
│   │   ├── icon_extract.rs    #   图标提取（SHDefExtractIconW + .lnk 解析目标图标）
│   │   ├── icon_hider.rs      #   图标隐藏/恢复（SysListView32）+ managed_icons 仓库（Vault）+ 删除分类框后还原非管理图标
│   │   ├── rename_editor.rs   #   内联重命名编辑器（独立线程 EDIT 控件 + 异步消息通信 + 扩展名保留）
│   │   ├── grid_layout.rs     #   布局计算/按钮矩形/命中测试/滚动重排
│   │   ├── monitor_utils.rs   #   显示器查询/窗口定位工具
│   │   ├── drag_image.rs      #   拖拽残影覆盖窗口（D2D 渲染跟随光标的半透明图标）
│   │   ├── ole_dnd.rs         #   OLE 拖拽源（IDropSource + IDataObject 实现）
│   │   ├── drop_target.rs     #   分类框拖放目标（IDropTarget 实现）
│   │   ├── drag_handler.rs    #   拖拽协调器（拖拽生命周期：阈值检测/位置更新/释放分发）
│   │   ├── drag_drop.rs       #   拖拽放置操作（同框/跨框/桌面/OLE drop）
│   │   ├── drag_visual.rs     #   拖拽可视化反馈（插入分隔线/替换高亮 + 纯逻辑函数）
│   │   ├── icon_manager.rs    #   图标管理器（SysListView32 跨进程操作 + 图像列表提取）
│   │   ├── control_panel_window.rs # 控制面板 Win32 显示/隐藏辅助（托盘优先生命周期）
│   │   ├── sys_tray.rs        #   系统托盘入口（隐藏消息窗口 + 固定菜单动作队列）
│   │   └── global_hotkey.rs   #   全局热键（RegisterHotKey + 消息窗口 + 快捷键解析）
│   └── macos/
│       ├── mod.rs             #   macOS 平台入口 + stub 函数
│       ├── desktop_manager.rs #   桌面图标枚举（NSWorkspace + FileManager）
│       ├── ffi_imports.rs     #   macOS FFI 声明（CoreGraphics 等）
│       ├── icon_extract.rs    #   图标提取（NSWorkspace iconForFile）
│       └── desktop_overlay/   #   桌面覆盖窗口
│           ├── mod.rs             #   模块导出
│           ├── overlay_window.rs  #   NSWindow 创建/销毁 + 鼠标穿透
│           ├── overlay_render.rs  #   渲染编排（分类框 + 图标）
│           ├── box_render.rs      #   分类框渲染（背景/标题/图标网格/标签）
│           └── icon_layout.rs     #   图标网格布局计算
│
├── box_core/                  # 分类框核心业务逻辑（平台无关）
│   ├── mod.rs                 #   模块导出
│   ├── box_model.rs           #   ClassificationBox 数据模型 + BoxItem
│   ├── box_manager.rs         #   分类框管理器（CRUD + 持久化）
│   ├── sort_engine.rs         #   排序引擎：8 种维度 × 升降序
│   ├── view_engine.rs         #   视图引擎：8 种视图模式布局参数
│   ├── group_engine.rs        #   分组引擎：按名称/类型/时间等
│   ├── layout_calc.rs         #   网格布局计算 + 命中测试
│   └── interaction.rs         #   交互状态：选中/hover/锁定/拖拽
│
├── classification/            # 分类系统
│   ├── mod.rs                 #   AppClassifier（from_config 从配置创建）
│   ├── database.rs            #   AppCategoryDB（从配置 CategoryRule 构建）
│   ├── matcher.rs             #   AppMatcher 通配符匹配算法
│   └── ai_classifier.rs       #   AI 分类器（TODO：LLM API 调用）
│
├── config/                    # 统一配置系统
│   ├── mod.rs                 #   模块导出
│   └── app_config.rs          #   统一 JSON 配置结构（AppConfig）+ 单元测试
│
├── plugin/                    # 插件系统
│   ├── mod.rs                 #   Plugin trait + PluginRegistry
│   └── builtin/
│       ├── mod.rs             #   register_builtin_plugins()
│       ├── classification_plugin.rs  #   分类功能插件
│       └── style_plugin.rs    #   样式设置插件
│
├── ui/                        # legacy eframe UI 层（仅 eframe-ui 回退路径）
│   ├── mod.rs                 #   模块导出
│   ├── control_panel.rs       #   控制面板主界面
│   ├── settings_panel.rs      #   设置面板（样式/AI/热键/调试）
│   └── tray_menu.rs           #   托盘菜单逻辑
│
├── utils/                     # 工具层
│   ├── mod.rs                 #   模块导出
│   ├── logger.rs              #   日志系统（滚动文件输出 + 自定义宏 + 发行版日志）
│   ├── platform.rs            #   跨平台工具（开机自启、配置目录）
│   ├── dpi.rs                 #   DPI 缩放工具
│   └── string_utils.rs        #   UTF-16 字符串转换
│
└── ipc/                       # 进程间通信（TODO 占位）
    └── mod.rs                 #   热键唤醒、单实例检测
```

**非代码目录**：
```
build/
├── build.rs                   # 嵌入 manifest 到 Windows 可执行文件
└── app.manifest               # Per-Monitor DPI V2 感知清单
```

### 核心设计

#### 1. AppContext — 中心上下文

```rust
// src/context.rs（实际代码）
pub struct AppContext {
    pub config: AppConfig,           // 统一配置（JSON，可导入/导出）
    pub desktop_manager: DesktopManager, // 平台桌面管理
    pub classifier: AppClassifier,   // 分类引擎
    pub state: AppState,             // 运行时状态
    // TODO: pub box_manager: BoxManager
    // TODO: pub plugins: PluginRegistry
}
```

所有 UI 层和插件通过 `AppContext` 访问配置、平台服务和分类引擎。`app.rs` 已接入。

#### 2. 平台抽象

```rust
// src/platform/mod.rs（实际代码）
pub trait DesktopManagerTrait {
    fn new() -> anyhow::Result<Self> where Self: Sized;
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;
    fn watch_desktop(&self) -> anyhow::Result<()>;
}
```

当前通过 `DesktopManagerTrait` 抽象桌面图标枚举。渲染/窗口/图标提取通过 `platform::windows::*` 的公共函数直接调用（Windows 专用），后续计划引入 `PlatformService` / `RenderBackend` / `WindowBackend` trait 进一步抽象。

#### 3. 插件系统

```rust
// src/plugin/mod.rs
pub trait Plugin: Send {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn init(&mut self, ctx: &mut AppContext) -> anyhow::Result<()>;
    fn menu_items(&self) -> Vec<PluginMenuItem>;
    fn handle_command(&mut self, cmd: &str, ctx: &mut AppContext) -> anyhow::Result<()>;
    fn cleanup(&mut self, ctx: &mut AppContext);
}
```

已定义 `PluginRegistry`（注册/分发/清理），内置 `ClassificationPlugin` + `StylePlugin`。TODO：接入 `AppContext`。

#### 4. 统一 JSON 配置

`~/.ikun_organizer.json` 管理所有设置，支持导入/导出：

| 配置段 | 内容 |
|--------|------|
| `general` | 开机自启、语言、单实例、启动恢复布局、启动自动分类 |
| `hotkeys` | toggle_panel、auto_classify |
| `box_style` | 背景色/透明度、圆角、字号、图标大小、标题栏、锁定/关闭按钮 |
| `box_defaults` | 新建分类框默认视图/排序/分组/尺寸 |
| `ai` | enabled、base_url、api_key、model、temperature、timeout、retries |
| `classification` | 分类规则列表（AI 失败时的兜底规则） |
| `layout` | 分类框列表（位置/尺寸/锁定/排序/视图/分组/文件列表/样式覆盖） |
| `overlay` | 桌面覆盖窗口设置（未管理图标逻辑坐标、图标尺寸、排列方向） |
| `debug` | log_level、log_to_file、show_fps、render_wireframe |

**存储位置**：`~/.ikun_organizer.json`（用户主目录）

**布局持久化流程**：
```
保存: collect_grid_layout() → config.layout.boxes → merge_latest_overlay_config() → save() → ~/.ikun_organizer.json
恢复: load() → restore_layout() → hide_native_icons() → create_desktop_overlay() → create_grid_window()
删除: 菜单 CMD_DELETE → destroy_grid_window(hwnd) → AppConfig.retain(id != config_id) → save()
```

**桌面覆盖窗口生命周期（阶段 6/7）**：当 `general.restore_layout_on_start = true` 时，应用启动即接管桌面图标生命周期：先初始化图标仓库并隐藏原生 `SysListView32`，再创建 `DesktopOrgOverlay` 渲染未进入分类框的图标，最后恢复分类框窗口。即使没有保存的分类框，也会创建覆盖窗口以保持桌面接管一致性。覆盖窗口创建后会启动桌面文件监听，监听用户桌面和公共桌面目录；新增、删除、重命名或修改桌面文件时经过约 500ms debounce 后重新枚举并刷新未管理图标。退出或“清除 & 恢复”时按顺序保存布局、销毁分类框、停止桌面文件监听、销毁覆盖窗口、恢复原生图标、将仓库中的图标还原到桌面并清理桌面右键注册表。运行中如果 Explorer 重启导致原生桌面列表句柄失效或重新可见，`native_icons_hidden()` 会触发重新隐藏并保持覆盖窗口运行。

**覆盖窗口位置持久化**：覆盖窗口图标位置保存在 `config.overlay.icon_positions`，键为文件路径，值为 96 DPI 基准逻辑坐标。覆盖窗口拖动图标后仅保存用户拖拽设定的位置（不保存自动布局位置），避免刷新时图标重叠；应用级保存布局前必须先合并磁盘上的最新 `overlay` 配置，避免用旧的内存配置覆盖最新位置。
**覆盖窗口图标布局**：覆盖窗口使用 Grid 网格布局（overlay_grid.rs），图标拖拽释放时自动吸附到最近的网格格子。若目标格子已有图标则交换位置，新图标加入时自动找第一个空闲格子。所有网格计算通过 overlay_layout_params() 统一参数化，兼容多屏幕不同 DPI/分辨率。初始布局使用纵向优先（列优先），先填满一列再换下一列。收集未管理图标时按 display_name 去重（同一图标可能在用户桌面和公共桌面各有一个 .lnk）。is_managed_icon_path 使用精确文件名匹配（非 ends_with），避免“企业微信”子串误匹配“微信”。分类框删除后通过 restore_unmanaged_vault_icons() 将仓库中不再被管理的图标还原到桌面，再通过 request_desktop_overlay_refresh() 刷新覆盖窗口。

**UUID 持久化标识**：每个窗口创建时分配 `config_id: uuid::Uuid`（`GridCreateParams.config_id` → `GridWindowData.config_id`），`collect_grid_layout()` 使用该固定 ID（而非每次 `Uuid::new_v4()`），确保删除操作可按 UUID 精确匹配配置条目。

**DPI 处理**：配置存储逻辑像素（DPI 96 基准），创建/恢复时 `create_grid_window` 按实际 DPI 缩放，`collect_grid_layout` 反向转换回逻辑像素。

**固定行为（代码硬编码，不可配置）**：
- 始终显示系统托盘，任务栏不显示
- 始终记忆窗口位置和尺寸
- 托盘双击 = 呼出控制面板
- 托盘右键 = 固定菜单（打开控制面板 / 创建分类框 / 一键AI分类 / 退出）

### 依赖关系

```
main.rs
  ├── context.rs (AppContext)
  │     ├── config/app_config.rs (AppConfig 统一配置)
  │     ├── platform/ (DesktopManagerTrait + 平台实现)
  │     │     ├── windows/ (D2D渲染 + Win32窗口 + Shell/注册表菜单 + GPUI 窗口辅助)
  │     │     └── macos/ (桌面覆盖窗口 + 分类框渲染 + stub/实验性功能)
  │     ├── box_core/ (分类框业务逻辑 — 平台无关)
  │     ├── classification/ (分类规则 + AI分类)
  │     └── plugin/ (Plugin trait + Registry)
  ├── gpui_app/ (Windows 默认 UI shell)
  │     ├── app_runtime.rs (UI 框架无关命令/运行时层，持有 AppContext)
  │     ├── control_panel.rs (GPUI 控制面板视图)
  │     ├── create_box_dialog.rs (独立 GPUI 创建分类框窗口)
  │     ├── edit_box_dialog.rs (独立 GPUI 编辑分类框窗口)
  │     └── windows_main.rs (GPUI 事件循环)
  ├── app.rs + ui/ (legacy eframe 回退，仅 eframe-ui feature)
  └── utils/ (日志、DPI、字符串)
```

**约束**：
- `box_core/` 不直接调用 `platform/` — 通过 trait 间接调用
- `classification/` 不依赖 `platform/` — 纯业务逻辑
- `ui/` 只通过 `AppContext` 或 `gpui_app::AppRuntime` 与后端交互
- `gpui_app/` 属于 Windows UI shell：View 层只渲染和派发命令，业务/平台调用集中在 `AppRuntime`
- `platform/` 各平台实现通过 `#[cfg]` 隔离

---

## Windows 平台技术要点

**渲染管线**：Direct2D + DirectWrite → DIB Section → `UpdateLayeredWindow()`（per-pixel alpha 分层窗口）

**窗口类型**：`WS_POPUP | WS_EX_LAYERED | WS_EX_TOOLWINDOW`，标题栏拖拽通过 `WM_NCHITTEST → HTCAPTION`，边缘缩放通过 `HTLEFT/HTRIGHT/...`

**标题栏布局**：左侧纯文本名称（无图标、无数量），右侧 🔒/🔓 锁定按钮 + ⋮ "更多"按钮（`Segoe UI Emoji` / `Segoe UI Symbol` 字体）。锁定时整个窗口返回 `HTCLIENT`（禁止拖拽和缩放），未锁定时正常边缘缩放（`hover_edge` 驱动蓝色指示条反馈）。按钮 hover 时显示手型光标 + 半透明背景高亮。

**分类框名称编辑**：双击标题文字打开 GPUI 编辑对话框（`edit_box_dialog.rs`，样式与"创建分类框"对话框对齐）。通过 `edit_box_callback` 回调传递 `config_id` 和当前名称，支持中文 IME 输入（利用 GPUI 内置的 `Input` 组件）。输入限制 16 字符。Enter 或点击"确定"保存（调用 `AppRuntime::rename_box()` 更新配置和窗口标题），Escape 或点击"取消"关闭。保存时通过 `update_grid_window_title()` 实时更新格子窗口标题。

**更多下拉菜单**：点击 ⋮ 按钮弹出自定义分层窗口菜单（`WS_POPUP | WS_EX_LAYERED` + D2D/DirectWrite 渲染），支持 per-pixel alpha 半透明深色背景、圆角、DirectWrite 清晰文字（任意 DPI 不模糊）、无系统阴影。包含删除、排序（8维度+递增/递减）、分组（8维度）、查看（8模式，预留回调接口暂未实现）四个功能组，子菜单自动定位（屏幕边界翻转）。多分类框菜单互斥（全局 `OPEN_MENU_HWND` + `PostMessage` 延迟切换）。菜单打开期间分类框自动锁定。通过 `SetForegroundWindow` + `WM_ACTIVATE(WA_INACTIVE)` 实现点击外部关闭，子菜单使用 `SW_SHOWNOACTIVATE` + `MA_NOACTIVATE` 避免抢焦点。

**锁定状态持久化**：`GridWindowData.locked` ↔ `BoxLayoutData.locked` ↔ `~/.ikun_organizer.json`，保存时 `collect_grid_layout()` 读取实际状态，恢复时 `restore_layout()` 传入 `locked` 参数。

**窗口层级**：
```
顶层窗口 > 普通应用 > **分类框窗口** > **桌面覆盖窗口 DesktopOrgOverlay** > 桌面图标 (SysListView32，隐藏) > 壁纸 (Progman/WorkerW)
```

**Shell 右键菜单**：文件菜单使用 `SHParseDisplayName → SHBindToParent → IShellFolder::GetUIObjectOf(IContextMenu)`，桌面背景菜单使用 `IShellFolder → IShellView::GetItemObject(SVGIO_BACKGROUND)` 获取真实 Shell 菜单；`InvokeCommand` 使用 `cmd - FCIDM_SHVIEWFIRST` 作为 verb 偏移，避免绝对/相对 PIDL 混用。COM 菜单链优先使用 `windows` crate 类型，失败时回退到自定义 `ShellExecuteW` 菜单。`ShellMenuResult` 区分 `Cancelled`（菜单未能显示，触发回退）和 `Dismissed`（菜单已显示但用户关闭，不触发回退），避免关闭菜单后误弹回退菜单。

**桌面右键动作**：通过 Windows 注册表 (`HKCU\...\Directory\Background\shell\IkunOpenPanel` / `IkunCreateBox` / `IkunAiClassify` / `IkunQuit`) 注入菜单项，Shell 自动合并到 `QueryContextMenu` 结果（覆盖初始菜单和“显示更多选项”legacy 菜单）。程序启动时先清理旧残留再注册，退出时清理，确保不运行时无残留。用户点击时通过命令行参数（如 `--create-box`）写入临时信号文件；运行实例由 `AppRuntime::tick()` 轮询消费后打开 GPUI 控制面板、独立创建分类框窗口、执行 AI 分类或退出。多屏定位通过 Win32 `FindWindowW` + `SetWindowPos` 物理像素坐标实现。

**虚拟 Shell 对象（CLSID）**：回收站、此电脑、网络、控制面板等虚拟对象通过 `::{CLSID}` 路径标识。`desktop_manager.rs` 维护 `VIRTUAL_SHELL_OBJECTS` 静态列表，`clsid_display_name()` 函数提供大小写不敏感的 CLSID → 中文名称映射。`BoxLayoutData.display_names` 持久化显示名称，恢复布局时 CLSID 路径始终通过 `clsid_display_name()` 解析（不依赖 `display_names` 数组），普通文件路径优先使用 `display_names`。

**图标提取与项目图标**：`SHDefExtractIconW` 优先提取 128×128 高清图标，`.lnk` 先通过 `IShellLink` + `IPersistFile` 解析目标后提取目标图标，避免快捷方式箭头覆层污染分类框图标。应用项目图标从 `logo.ico` 经 `build.rs` 嵌入可执行文件，供系统托盘和菜单项图标使用。

**DPI 适配**：manifest + 运行时 `SetProcessDpiAwarenessContext(PER_MONITOR_V2)`，`GetDpiForWindow` + `WM_DPICHANGED` 动态响应

---

## 开发规范

### 错误处理
- 统一使用 `anyhow::Result`
- 失败时 fallback 到默认值（不阻塞应用）
- 所有错误必须记录日志

### 平台代码
- 使用 `#[cfg(windows)]` / `#[cfg(target_os = "macos")]` 隔离
- 通过 trait 统一接口

### 代码质量
- 提交前 `cargo fmt` + `cargo clippy`
- 所有公开 API 必须有文档注释
- 单文件不超过 1500 行（渲染引擎等复杂模块可适当放宽）

### 日志
- 使用 `log::{debug, info, warn, error}`
- 日志写入 `~/.ikun_organizer_log/` 目录，文件命名：`.ikun_organizer_log_YYYYMMDD_N`（如 `.ikun_organizer_log_20260722_1`）
- 单文件超过 2MB 自动切换到下一个序号文件
- 启动时自动清理超过 7 天的日志文件
- 日志级别通过 `RUST_LOG` 环境变量控制
- 开发日志宏：`log_debug!`、`log_info!`、`log_warn!`、`log_error!`（自动附加文件/模块信息）
- 发行版日志宏：`release_info!`、`release_warn!`、`release_error!`（带 `[RELEASE]` 前缀，用于生产环境问题排查，不应作为调试代码删除）
---

## 功能需求（待实现）

### 分类框功能
- [x] 拖拽移动 + 位置记忆（通过 collect_grid_layout + restore_layout）
- [x] 边缘拖拽修改尺寸 + 尺寸记忆（同上）
- [x] 锁定按钮 + 锁定记忆（锁定后不可拖拽/缩放，hover 反馈，状态持久化）
- [x] "更多"下拉菜单（排序/分组/查看/删除，自定义分层窗口 + D2D/DirectWrite 渲染，删除功能已实现）
- [x] 分类框名称修改（双击标题文字打开 GPUI 编辑对话框，输入框 + 确定/取消按钮，16字符限制，Enter保存/Esc取消）
- [x] 图标内联重命名（Shell 右键菜单 F2 → 独立线程 EDIT 控件，预选文件名不含扩展名，异步 PostMessage 结果，扩展名自动保留）
- [x] 排序方式：名称、类型、修改时间、作者、创建时间、大小、标记、标题 × 递增/递减（引擎已实现，菜单可选）
- [x] 查看方式：超大、大、中、小、列表、详细信息、平铺、内容（引擎已实现，菜单可选）
- [x] 分组展示：按名称、类型、修改时间、作者、创建时间、大小、标记、标题（引擎已实现，菜单可选）
- [x] 拖拽快捷方式到外部桌面 / 外部桌面快捷方式拖入分类框（通过 DesktopOrgOverlay + Vault 无管理员双向移动）
- [x] 右键功能与原生快捷方式一致（Shell 上下文菜单已实现）
- [x] 分类框删除（更多菜单 → DestroyWindow + UUID 精确删除配置 + rerender_all 修复分层窗口）
- [x] 图标 Y 轴滚动（鼠标滚轮 + 半透明滚动条，DPI 自适应，滚动区域裁剪防止溢出标题栏）
- [x] 窗口层级管理（分类框始终低于其他应用窗口，分类框之间点击的在最前，WM_WINDOWPOSCHANGING + HWND_BOTTOM + 递归抑制）
- [x] 图标自动重排（窗口尺寸变化时按线性索引重新映射，保持视觉顺序和空位，支持拖拽后固定位置的动态重排）
- [x] 统一图标样式（覆盖区域和分类框使用相同的 hover/选中背景色和圆角，暗蓝灰色半透明 + 6px 圆角）
- [x] 拖拽可视化反馈（插入分隔线 + 替换高亮，支持同框/跨框/覆盖区域拖拽，shift-insert 替代 swap）
- [x] 分类框拖拽文件到资源管理器（OLE CF_HDROP 复制到 Explorer，图标保持原位）

### 桌面右键菜单
- [x] 打开控制面板（注册表注入 + 信号文件 IPC + GPUI 控制面板显示）
- [x] 创建桌面分类框（注册表注入 + 信号文件 IPC + 独立 GPUI 创建分类框窗口）
- [x] 一键 AI 桌面分类（注册表注入 + 信号文件 IPC + 运行时分类入口）
- [x] 退出桌面分类（注册表注入 + 信号文件 IPC + 清理退出）

### macOS 平台（开发中）
- [x] 桌面图标枚举（NSWorkspace + FileManager API）
- [x] 图标提取（NSWorkspace iconForFile + NSImage → CGImage）
- [x] 桌面覆盖窗口（NSWindow + 透明 + 鼠标穿透）
- [x] 分类框渲染（圆角背景 + 标题 + 图标网格 + Finder 风格标签）
- [x] 标签截断（第一行最大前缀 + 第二行中间截断保留扩展名）
- [ ] 分类框拖拽/缩放
- [ ] 右键菜单
- [ ] 图标拖拽到分类框
- [ ] 布局持久化

### 控制台（控制面板）
- [ ] 键盘快捷键呼出
- [x] 不在任务栏显示（WS_EX_TOOLWINDOW）
- [x] 仅系统托盘图标（右键菜单：打开面板/创建分类框/AI分类/退出）
- [x] 分类框风格设置（背景色、圆角、图标大小、文本大小）
- [x] AI BaseUrl / API Key 设置
- [x] 统一配置持久化（~/.ikun_organizer.json）

---

## 参考

- [egui 文档](https://docs.rs/egui/)
- [egui 示例](https://github.com/emilk/egui/tree/master/examples)
- [windows-rs](https://github.com/microsoft/windows-rs)
- [Windows Shell API](https://learn.microsoft.com/en-us/windows/win32/shell/)
- 架构重构方案：`todo/架构重构.md`
- 需求 PRD：`todo/desktop-prd.md`



