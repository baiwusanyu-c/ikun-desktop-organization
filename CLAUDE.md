# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

**桌面图标智能分类管理工具** — Desktop Organizer

基于 Rust + egui 的跨平台 GUI 应用，智能分类和管理桌面图标。

**核心特性**：智能分类桌面图标、可视化分类框（拖拽/缩放/锁定/样式自定义）、一键 AI 分类、布局记忆持久化、统一 JSON 配置导入导出。

**平台支持**：
- ✅ **Windows 10/11**：主要平台（Shell COM API + Direct2D/DirectWrite 渲染）
- ⚠️ **macOS**：实验性（返回模拟数据，待完善）

---

## 构建和运行

```bash
cargo run                        # 开发模式运行
RUST_LOG=debug cargo run         # 带调试日志运行（推荐）
cargo check                      # 快速编译检查
cargo fmt                        # 格式化
cargo clippy                     # Lint 检查
cargo test                       # 运行测试
cargo watch -x check             # 自动检查（需 cargo install cargo-watch）
```

**日志级别**：通过 `RUST_LOG` 环境变量控制（`trace` / `debug` / `info` / `warn` / `error`）

---

## 技术栈

| Crate | 版本 | 用途 |
|-------|------|------|
| egui / eframe / epaint | 0.27 | GUI 框架（控制面板窗口） |
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
- **单文件 ≤ 1000 行**：严格控制代码规模

### 项目目录结构

```
src/
├── main.rs                    # 入口：初始化日志、DPI 感知、启动 egui
├── app.rs                     # eframe::App 控制面板（已接入 AppContext）
├── context.rs                 # AppContext 核心上下文对象
│
├── platform/                  # 平台抽象层
│   ├── mod.rs                 #   DesktopManagerTrait + re-exports
│   ├── types.rs               #   跨平台类型：DesktopIcon, IconInfo, ScreenInfo
│   ├── windows/
│   │   ├── mod.rs             #   Windows 平台入口
│   │   ├── ffi_types.rs       #   Win32 FFI 类型/常量（251 行）
│   │   ├── ffi_imports.rs     #   Win32 extern "system" 声明（374 行）
│   │   ├── desktop_manager.rs #   桌面图标枚举（文件系统读取）
│   │   ├── d2d_render.rs      #   D2D/DirectWrite 渲染引擎（625 行）
│   │   ├── grid_window.rs     #   窗口创建/销毁 + wnd_proc 消息处理（701 行）
│   │   ├── shell_menu.rs      #   Shell 右键菜单：文件 + 桌面背景（349 行）
│   │   ├── icon_extract.rs    #   图标提取（SHGetFileInfoW + .lnk 解析）
│   │   ├── icon_hider.rs      #   图标隐藏/恢复（SysListView32）
│   │   └── sys_tray.rs        #   系统托盘（TODO 占位）
│   └── macos/
│       ├── mod.rs             #   macOS 平台入口
│       └── desktop_manager.rs #   桌面图标枚举（返回模拟数据）
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
├── ui/                        # UI 层（egui 组件）
│   ├── mod.rs                 #   模块导出
│   ├── control_panel.rs       #   控制面板主界面
│   ├── settings_panel.rs      #   设置面板（样式/AI/热键/调试）
│   └── tray_menu.rs           #   托盘菜单逻辑
│
├── utils/                     # 工具层
│   ├── mod.rs                 #   模块导出
│   ├── logger.rs              #   日志初始化 + 自定义宏
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
| `debug` | log_level、log_to_file、show_fps、render_wireframe |

**存储位置**：`~/.ikun_organizer.json`（用户主目录）

**布局持久化流程**：
```
保存: collect_grid_layout() → config.layout.boxes → save() → ~/.ikun_organizer.json
恢复: load() → config.layout.boxes → restore_layout() → create_grid_window()
```

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
  │     │     ├── windows/ (D2D渲染 + Win32窗口 + Shell菜单)
  │     │     └── macos/ (模拟数据)
  │     ├── box_core/ (分类框业务逻辑 — 平台无关)
  │     ├── classification/ (分类规则 + AI分类)
  │     └── plugin/ (Plugin trait + Registry)
  ├── app.rs (eframe::App) → 通过 AppContext 交互
  ├── ui/ (控制面板/设置面板/托盘菜单)
  └── utils/ (日志、DPI、字符串)
```

**约束**：
- `box_core/` 不直接调用 `platform/` — 通过 trait 间接调用
- `classification/` 不依赖 `platform/` — 纯业务逻辑
- `ui/` 只通过 `AppContext` 与后端交互
- `platform/` 各平台实现通过 `#[cfg]` 隔离

---

## Windows 平台技术要点

**渲染管线**：Direct2D + DirectWrite → DIB Section → `UpdateLayeredWindow()`（per-pixel alpha 分层窗口）

**窗口类型**：`WS_POPUP | WS_EX_LAYERED | WS_EX_TOOLWINDOW`，标题栏拖拽通过 `WM_NCHITTEST → HTCAPTION`，边缘缩放通过 `HTLEFT/HTRIGHT/...`

**窗口层级**：
```
顶层窗口 > 普通应用 > **分类框窗口** > 桌面图标 (SysListView32) > 壁纸 (Progman/WorkerW)
```

**Shell 右键菜单**：`IShellFolder` + `IContextMenu` + `IShellView::GetItemObject(SVGIO_BACKGROUND)` 获取真实桌面菜单，回退方案用 `ShellExecuteW`

**图标提取**：`SHDefExtractIconW` 提取 128×128 高清图标，`.lnk` 先通过 `IShellLink` + `IPersistFile` 解析目标

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
- 单文件不超过 1000 行

### 日志
- 使用 `log::{debug, info, warn, error}`
- 自定义宏：`log_debug!`、`log_info!`、`log_warn!`、`log_error!`（自动附加文件/模块信息）

---

## 功能需求（待实现）

### 分类框功能
- [x] 拖拽移动 + 位置记忆（通过 collect_grid_layout + restore_layout）
- [x] 边缘拖拽修改尺寸 + 尺寸记忆（同上）
- [ ] 锁定按钮 + 锁定记忆（锁定后不可拖拽/缩放）
- [ ] 分类框名称修改
- [x] 排序方式：名称、类型、修改时间、作者、创建时间、大小、标记、标题 × 递增/递减（引擎已实现）
- [x] 查看方式：超大、大、中、小、列表、详细信息、平铺、内容（引擎已实现）
- [x] 分组展示：按名称、类型、修改时间、作者、创建时间、大小、标记、标题（引擎已实现）
- [ ] 拖拽快捷方式到外部桌面 / 外部桌面快捷方式拖入分类框
- [x] 右键功能与原生快捷方式一致（Shell 上下文菜单已实现）
- [ ] 分类框删除

### 桌面右键菜单
- [ ] 创建桌面分类框
- [ ] 一键 AI 桌面分类
- [ ] 退出桌面分类

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
