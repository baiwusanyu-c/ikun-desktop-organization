# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

**桌面图标智能分类管理工具** - Desktop Organizer

一个基于 Rust + egui 构建的跨平台 GUI 应用，用于智能分类和管理桌面图标。

**核心特性**：
- 智能分类桌面图标（游戏、开发工具、办公、媒体等）
- 可视化分类框（拖拽、调整尺寸、样式自定义）
- 一键智能分类
- 开机自启动
- 布局记忆持久化

**平台支持**：
- ✅ **Windows 10/11**：主要支持平台（使用 Shell API）
- ⚠️ **macOS**：实验性支持（使用 cocoa/objc，返回模拟数据用于 MVP）

---

## 开发和调试

### 开发环境配置

项目已完整配置开发调试环境，包括：

1. **RustRover 运行配置**（`.idea/runConfigurations/` 目录）
   - **Run**：正常运行（`RUST_LOG=info`）
   - **Debug**：调试运行（`RUST_LOG=debug`）← 推荐日常使用
   - **Check**：快速检查代码
   - **Test**：运行测试

2. **自动编译工具**
   - 安装：`cargo install cargo-watch`
   - 使用：`cargo watch -x check`（后台自动检查）

3. **日志系统**
   - 已集成 `log` + `env_logger`
   - 工具模块：`src/utils/logger.rs`
   - 自定义宏：`log_info!`, `log_debug!`, `log_warn!`, `log_error!`

### 三种调试方式

#### 方式 A：日志调试（推荐日常使用）

```rust
// 在代码中添加日志
use log::{debug, info, warn, error};

debug!("开始处理: {:?}", data);
info!("处理完成，数量: {}", count);
warn!("配置缺失，使用默认值");
error!("处理失败: {}", e);
```

运行方式：
- **RustRover**：选择 **Debug** 配置 → `Cmd+D`
- **终端**：`RUST_LOG=debug cargo run`

#### 方式 B：断点调试（复杂问题）

步骤：
1. 在 RustRover 点击代码行号设置断点 🔴
2. 选择 **Debug** 配置
3. 按 `Cmd+D` 启动调试
4. 应用窗口打开，在 UI 中操作触发断点
5. 查看变量、调用栈
6. 按 `F8` 单步执行，`F9` 继续

**注意**：断点触发时应用窗口会"卡住"，这是正常的（调试器暂停了进程）。

#### 方式 C：自动编译（后台运行）

```bash
# 终端运行，监听文件变化
cargo watch -x check

# 或在 RustRover 启动 Watch Check 配置
```

### 开发教程文档

详细教程位于 `todo/` 目录：

| 文档 | 说明 |
|------|------|
| [todo/快速入门.md](todo/快速入门.md) | 5 分钟上手指南 |
| [todo/开发调试指南.md](todo/开发调试指南.md) | 完整开发流程详解 |
| [todo/日志使用示例.md](todo/日志使用示例.md) | 日志实战教程 |
| [todo/README.md](todo/README.md) | 文档索引 |

### 快速参考

**RustRover 操作**：

| 操作 | 配置 | 快捷键 |
|------|------|--------|
| 调试运行 | Debug | `Cmd+D` |
| 正常运行 | Run | `Cmd+R` |
| 运行测试 | Test | `Cmd+R` |

**终端命令**：

```bash
# 开发调试
RUST_LOG=debug cargo run      # 显示详细日志
cargo watch -x check          # 自动检查（推荐）

# 代码质量
cargo fmt                     # 格式化
cargo clippy                  # 检查
cargo test                    # 测试
```

---

## 构建和运行命令

```bash
# 开发模式构建和运行
cargo run

# 发布模式构建（优化编译）
cargo run --release

# 检查代码（快速编译检查）
cargo check

# 自动检查（推荐后台运行）
cargo watch -x check

# 运行测试
cargo test

# 运行特定测试
cargo test <test_name>

# 格式化代码
cargo fmt

# 代码检查（linter）
cargo clippy

# 日志控制
RUST_LOG=debug cargo run    # 显示调试日志（推荐调试时使用）
RUST_LOG=info cargo run     # 显示普通信息
RUST_LOG=warn cargo run     # 只显示警告和错误
```

**调试推荐**：
- RustRover：选择 **Debug** 配置 → 按 `Cmd+D`
- 终端：`RUST_LOG=debug cargo run`

---

## 技术栈

### 核心框架

| Crate | 版本 | 用途 |
|-------|------|------|
| **egui** | 0.27 | 即时模式 GUI 框架 |
| **eframe** | 0.27 | egui 应用框架（包含 winit） |
| **epaint** | 0.27 | egui 渲染后端 |

**为什么选择 egui**：
- 纯 Rust 生态，无需 Web 前端技术栈
- 即时模式 GUI，适合动态 UI 和拖拽交互
- 原生渲染性能，GPU 加速
- 轻量级，编译产物体积小
- 完美支持透明窗口和自定义窗口形状

### 通用库

| Crate | 版本 | 用途 |
|-------|------|------|
| **anyhow** | 1.0 | 错误处理 |
| **log** | 0.4 | 日志 facade |
| **env_logger** | 0.11 | 日志实现 |
| **serde** | 1.0 | 数据序列化 |
| **serde_json** | 1.0 | JSON 序列化（布局、设置） |
| **serde_yaml** | 0.9 | YAML 序列化（分类数据库） |
| **dirs** | 5.0 | 跨平台配置目录 |
| **chrono** | 0.4 | 时间处理 |
| **uuid** | 1.0 | UUID 生成 |
| **notify** | 6.0 | 文件系统监听（桌面变化） |

### 平台 API

**Windows** (windows-rs 0.58):
```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Win32_UI_Shell",              # Shell API（桌面图标）
    "Win32_System_Com",            # COM 组件
    "Win32_UI_WindowsAndMessaging", # 窗口管理
    "Win32_Storage_FileSystem",    # 文件系统
    "Win32_Graphics_Gdi",          # GDI（多显示器）
]}
```

**macOS** (cocoa/objc 0.25):
```toml
[target.'cfg(target_os = "macos")'.dependencies]
cocoa = "0.25"
objc = "0.2"
objc-foundation = "0.1"
```

---

## 架构概述（v2.0 — 重构设计）

### 核心架构原则

- **平台抽象**：通过 trait 统一接口，Windows/macOS 各自实现
- **业务分离**：分类框业务逻辑（排序/视图/分组）与平台渲染完全解耦
- **上下文注入**：所有模块通过 `AppContext` 共享状态，替代全局变量
- **插件化**：功能模块作为插件注册，灵活扩展
- **统一配置**：单一 JSON 文件管理所有配置，支持导入/导出
- **单文件 ≤ 1000 行**：严格控制代码文件规模

### 项目目录结构

```
src/
├── main.rs                          # 入口：初始化日志、加载配置、启动应用
├── app.rs                           # eframe::App 控制面板 UI
├── context.rs                       # AppContext 核心上下文对象
│
├── platform/                        # 平台抽象层
│   ├── mod.rs                       #   trait 定义：PlatformService, RenderBackend, WindowBackend
│   ├── types.rs                     #   跨平台公共类型：RawIconHandle, WindowHandle, DesktopIcon
│   ├── windows/
│   │   ├── mod.rs                   #   Windows 平台入口，组装各服务
│   │   ├── ffi_types.rs             #   Win32 FFI 类型定义
│   │   ├── ffi_imports.rs           #   Win32 extern "system" 声明
│   │   ├── desktop_manager.rs       #   桌面图标枚举
│   │   ├── d2d_render.rs            #   D2D/DirectWrite 渲染引擎
│   │   ├── grid_window.rs           #   窗口创建/销毁/生命周期
│   │   ├── wnd_proc.rs              #   窗口过程：消息分发
│   │   ├── shell_menu.rs            #   Shell 右键菜单（文件+桌面背景）
│   │   ├── icon_extract.rs          #   图标提取
│   │   ├── icon_hider.rs            #   图标隐藏/恢复
│   │   └── sys_tray.rs              #   系统托盘图标
│   └── macos/
│       ├── mod.rs                   #   macOS 平台入口
│       ├── desktop_manager.rs       #   桌面图标枚举
│       ├── render.rs                #   macOS 渲染后端
│       ├── grid_window.rs           #   macOS 窗口管理
│       └── icon_extract.rs          #   macOS 图标提取
│
├── box_core/                        # 分类框核心业务逻辑（平台无关）
│   ├── mod.rs                       #   模块导出
│   ├── box_model.rs                 #   ClassificationBox 数据模型
│   ├── box_manager.rs               #   分类框管理器
│   ├── sort_engine.rs               #   排序引擎：8种排序 × 升降序
│   ├── view_engine.rs               #   视图引擎：8种视图模式布局
│   ├── group_engine.rs              #   分组引擎
│   ├── layout_calc.rs               #   网格布局计算
│   └── interaction.rs               #   交互状态：选中/hover/锁定/拖拽
│
├── classification/                  # 分类系统
│   ├── mod.rs                       #   AppClassifier
│   ├── database.rs                  #   AppCategoryDB
│   ├── matcher.rs                   #   AppMatcher 匹配算法
│   └── ai_classifier.rs             #   AI 分类插件（调用 LLM API）
│
├── config/                          # 统一配置系统
│   ├── mod.rs                       #   模块导出 + 加载/保存
│   ├── app_config.rs                #   统一 JSON 配置结构
│   ├── box_style.rs                 #   分类框样式配置
│   ├── hotkey.rs                    #   热键配置
│   └── ai_config.rs                 #   AI 相关配置
│
├── plugin/                          # 插件系统
│   ├── mod.rs                       #   Plugin trait + PluginRegistry
│   └── builtin/
│       ├── mod.rs                   #   内置插件注册
│       ├── classification_plugin.rs #   分类功能插件
│       └── style_plugin.rs          #   样式设置插件
│
├── ui/                              # UI 层
│   ├── mod.rs                       #   模块导出
│   ├── control_panel.rs             #   控制面板主界面
│   ├── settings_panel.rs            #   设置面板（样式/AI/热键）
│   └── tray_menu.rs                 #   托盘菜单逻辑
│
├── utils/                           # 工具层
│   ├── mod.rs                       #   模块导出
│   ├── logger.rs                    #   日志系统（结构化、文件输出）
│   ├── platform.rs                  #   跨平台工具函数
│   ├── dpi.rs                       #   DPI 缩放工具
│   └── string_utils.rs              #   字符串工具（UTF-16 转换等）
│
└── ipc/                             # 进程间通信（可选，后续）
    └── mod.rs                       #   热键唤醒、单实例检测
```

**非代码目录**：
```
build/
├── build.rs                         # 嵌入 manifest 到 Windows 可执行文件
└── app.manifest                     # Windows DPI 感知清单
```

### 核心架构设计

**1. AppContext — 中心上下文对象**

所有模块通过 `AppContext` 共享状态和配置，替代散落的全局变量：

```rust
// src/context.rs
pub struct AppContext {
    pub config: AppConfig,              // 统一配置（JSON）
    pub platform: Box<dyn PlatformService>,  // 平台服务
    pub box_manager: BoxManager,         // 分类框管理器
    pub classifier: AppClassifier,       // 分类器
    pub plugins: PluginRegistry,         // 插件注册表
    pub state: AppState,                 // 运行时状态
}
```

**2. 平台抽象 Trait**

```rust
// src/platform/mod.rs
pub trait PlatformService: Send {
    fn desktop_manager(&self) -> &dyn DesktopManagerTrait;
    fn render_backend(&self) -> &dyn RenderBackend;
    fn window_backend(&self) -> &dyn WindowBackend;
    fn icon_service(&self) -> &dyn IconService;
    fn icon_hider(&self) -> &dyn IconHider;
    fn tray_service(&self) -> &dyn TrayService;
    fn screen_info(&self) -> ScreenInfo;
}

pub trait RenderBackend: Send {
    fn render_box(&self, ctx: &RenderContext) -> anyhow::Result<()>;
    fn init(&mut self) -> anyhow::Result<()>;
    fn cleanup(&mut self);
}

pub trait WindowBackend: Send {
    fn create_box_window(&self, params: &BoxWindowParams) -> anyhow::Result<WindowHandle>;
    fn destroy_window(&self, handle: WindowHandle) -> anyhow::Result<()>;
    fn destroy_all(&self);
    fn window_count(&self) -> usize;
}
```

平台实现：
- **Windows**：Direct2D + DirectWrite 渲染，WS_POPUP 分层窗口，Shell COM API
- **macOS**：CoreGraphics/Metal 渲染，NSWindow，NSWorkspace（实验性）

**3. 插件系统**

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

**4. 统一 JSON 配置**

单一 `config.json` 管理所有设置，支持导入/导出：

```json
{
  "version": "2.0",
  "general": { "auto_start": false, "language": "zh-CN" },
  "box_style": {
    "background_color": [1, 1, 1], "background_alpha": 38,
    "border_radius": 16.0, "icon_size": 48,
    "title_font_size": 15.0, "label_font_size": 13.0
  },
  "ai": { "enabled": false, "base_url": "", "api_key": "", "model": "gpt-4o-mini" },
  "hotkeys": { "toggle_panel": "Ctrl+Alt+D" },
  "tray": { "enabled": true },
  "layout": { "boxes": [] },
  "classification": { "source": "embedded", "categories": [] },
  "debug": { "log_level": "info", "log_to_file": true }
}
```

**存储位置**：
- Windows: `%APPDATA%/ikun-desktop-organization/config.json`
- macOS/Linux: `~/.config/ikun-desktop-organization/config.json`

**5. 智能分类系统**

三级混合方案：

```
第一优先级：本地规则（自动分类）
  - config.json → classification.categories（统一配置）
  - 代码 Default 实现（兜底）

第二优先级：用户自定义规则（手动定制）
  - 支持手动创建分类框
  - 支持拖拽调整分类

第三优先级：AI 分类（LLM API）
  - 通过 ai_classifier.rs 调用外部 AI 服务
```

**匹配算法**：

```rust
// 简单通配符匹配
exe_name.matches_pattern("idea*.exe")  // 匹配 IntelliJ IDEA
path.contains("\\Games\\")              // 路径包含关键词
```

### 依赖关系图

```
main.rs
  ├── context.rs (AppContext)
  │     ├── config/ (AppConfig 统一配置)
  │     ├── platform/ (PlatformService trait + 平台实现)
  │     │     ├── windows/ (D2D渲染 + Win32窗口 + Shell菜单)
  │     │     └── macos/   (CoreGraphics + NSWindow)
  │     ├── box_core/ (分类框业务逻辑 - 平台无关)
  │     ├── classification/ (分类规则 + AI分类)
  │     └── plugin/ (插件注册表)
  ├── ui/ (egui 控制面板) → 通过 AppContext 与后端交互
  └── utils/ (日志、DPI、字符串)
```

**核心原则**：
- `box_core/` 不直接调用 `platform/` — 通过 trait 对象间接调用
- `classification/` 不依赖 `platform/` — 纯业务逻辑
- `ui/` 只通过 `AppContext` 与后端交互
- `platform/` 各平台实现互不依赖 — 通过 `#[cfg]` 隔离

---

## 平台差异

### Windows

**技术路径**：Windows Shell API (COM) + Direct2D/DirectWrite 渲染

**关键 API**：
- `SHGetDesktopFolder()` - 获取桌面文件夹
- `IShellFolder::EnumObjects()` - 枚举项目
- `IShellView::GetItemPosition()` - 获取位置
- `UpdateLayeredWindow()` - 分层窗口渲染
- `D2D1CreateFactory()` - Direct2D 渲染引擎

**窗口层级控制**：
```
[最顶层]
  └─ 顶层窗口 (Topmost)
  └─ 普通应用窗口
  └─ **我们的分类框窗口** ← 目标层级
  └─ 桌面图标 (Desktop Icons)
  └─ 桌面壁纸窗口 (Progman/WorkerW)
[最底层]
```

**实现方案**：使用 `SetWindowPos` 设置正确的 Z-Order

### macOS

**技术路径**：cocoa + objc（实验性）

**主要挑战**：
- 沙盒限制（App Store 应用无法访问桌面）
- Finder 集成（桌面由 Finder 完全控制）
- 权限要求（需要 Accessibility 权限）

**当前状态**：
- 返回模拟数据用于 MVP 测试
- 后续版本将完善真实桌面图标读取

---

## 重构执行计划

| Phase | 目标 | 风险 |
|-------|------|------|
| **Phase 0** | 项目结构整理（build.rs 移动 + data/ 清理）| 零风险 |
| **Phase 1** | 拆分 grid_window.rs（2560行 → 8个文件）| 最安全 |
| **Phase 2** | 建立平台抽象层（platform/ 目录 + trait）| 中等 |
| **Phase 3** | AppContext + 统一 JSON 配置 | 中等 |
| **Phase 4** | box_core 业务层（排序/视图/分组引擎）| 渐进式 |
| **Phase 5** | 插件系统（Plugin trait + Registry）| 渐进式 |
| **Phase 6** | UI 重构 + 系统托盘 + 日志增强 | 渐进式 |

**核心约束**：全程保持 `cargo build` 通过，每个 Phase 完成后手动测试验证

### P0 - 核心功能（必须）

- ✅ egui + winit 窗口系统
  - 透明窗口
  - 无边框窗口
  - 基础窗口层级控制
- ✅ Windows 桌面图标读取
  - 使用 windows-rs 调用 Shell API
  - 获取图标位置、名称、目标路径
- ✅ 基础分类功能
  - 本地 YAML 数据库
  - 简单匹配算法
- ✅ 分类框 UI（egui 实现）
  - 方框容器
  - Grid 布局
  - 基础拖拽（整体移动）
- ✅ 布局保存和恢复
  - JSON 存储
  - 启动时自动加载

### P1 - 重要功能（尽量）

- ⚠️ 分类框尺寸调整（边缘拖拽）
- ⚠️ 基础样式（圆角、背景色）
- ⚠️ 分类框命名
- ⚠️ 一键智能分类按钮

### P2 - 次要功能（后续）

- ⏸️ 用户自定义规则界面
- ⏸️ 锁定功能
- ⏸️ 高级样式（毛玻璃）
- ⏸️ 开机自启
- ⏸️ 多显示器支持

### 不包含（后续版本）

- ❌ MCP + Agent 方案（低优先级）
- ❌ macOS 完整支持
- ❌ 完善的设置界面
- ❌ 安装包和更新机制

---

## 开发规范

### 必读指南

**开发前必须阅读**（[.trellis/spec/backend/index.md](.trellis/spec/backend/index.md)）：

1. [Directory Structure](.trellis/spec/backend/directory-structure.md) - 模块组织规则
2. [Error Handling](.trellis/spec/backend/error-handling.md) - 使用 anyhow::Result
3. [Quality Guidelines](.trellis/spec/backend/quality-guidelines.md) - 禁止和必须模式
4. [Logging Guidelines](.trellis/spec/backend/logging-guidelines.md) - 日志规范

**开发环境配置**：
- [开发调试教程](todo/README.md) - 开发调试完整指南
- [RustRover 配置](.run/) - 4 个运行配置（Run/Debug/Watch Check/Test）

### 核心原则

**错误处理**：
- 使用 `anyhow::Result`
- 失败时使用默认值（不阻塞应用）
- 所有错误必须记录日志

**平台特定代码**：
- 使用 `#[cfg(windows)]` / `#[cfg(target_os = "macos")]`
- 通过 Trait 统一接口

**代码质量**：
- 提交前运行 `cargo fmt` 和 `cargo clippy`
- 所有公开 API 必须有文档注释

---

## 技术风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| Windows 更新导致 API 变化 | 高 | 使用稳定 API，监控更新 |
| 杀毒软件误报 | 中 | 申请代码签名，加入白名单 |
| macOS 权限被拒 | 高 | 简化功能，提供详细说明 |
| 桌面图标不同步 | 高 | 实时监听 + 定时刷新 |
| 用户卸载后图标丢失 | 中 | 提供还原功能，备份原始布局 |

---

## 参考资源

### Rust 和 egui 学习
- [egui 官方文档](https://docs.rs/egui/)
- [egui 示例代码](https://github.com/emilk/egui/tree/master/examples)
- [Rust Windows API 绑定](https://github.com/microsoft/windows-rs)

### Windows Shell API 文档
- [Windows Shell 编程指南](https://learn.microsoft.com/en-us/windows/win32/shell/)
- [IShellFolder 接口](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ishellfolder)

---

**生成时间**：2026-06-08
**版本**：v1.0（基于可行性分析文档和现有代码）