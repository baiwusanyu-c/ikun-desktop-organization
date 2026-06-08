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

1. **RustRover 运行配置**（`.run/` 目录）
   - **Run**：正常运行（`RUST_LOG=info`）
   - **Debug**：调试运行（`RUST_LOG=debug`）← 推荐日常使用
   - **Watch Check**：自动检查代码
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

## 架构概述

### 核心模块结构

```
src/
├── main.rs              # egui 应用主入口，窗口配置
├── app.rs               # eframe::App 实现，应用主逻辑
│
├── desktop/             # 桌面系统集成（跨平台）
│   ├── mod.rs          # DesktopManagerTrait trait 定义
│   ├── icons.rs        # DesktopIcon 数据结构
│   ├── windows.rs      # Windows 平台实现 #[cfg(windows)]
│   └── macos.rs        # macOS 平台实现 #[cfg(target_os = "macos")]
│
├── classification/      # 智能分类系统
│   ├── mod.rs          # AppClassifier 入口（组合 database + matcher）
│   ├── database.rs     # AppCategoryDB 分类数据库（YAML 加载）
│   └── matcher.rs      # AppMatcher 匹配算法
│
├── ui/                 # egui UI 组件
│   ├── mod.rs          # UI 模块导出
│   ├── box_widget.rs   # ClassificationBoxWidget 分类框组件
│   └── icon_widget.rs  # IconWidget 图标组件
│
├── config/             # 配置管理
│   ├── mod.rs          # Config 模块导出
│   ├── layout.rs       # DesktopLayout 布局持久化（JSON）
│   └── settings.rs     # AppSettings 应用设置
│
└── utils/              # 工具函数
    ├── mod.rs          # Utils 模块导出
    ├── platform.rs     # 平台相关工具函数
    └── logger.rs       # 日志工具（自定义宏、性能日志）
```

### 关键架构设计

**1. 跨平台桌面管理**

通过 `DesktopManagerTrait` trait 定义统一接口：

```rust
// src/desktop/mod.rs
pub trait DesktopManagerTrait {
    fn new() -> anyhow::Result<Self> where Self: Sized;
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;
    fn watch_desktop(&self) -> anyhow::Result<()>;
}
```

平台特定实现：
- Windows：使用 `IShellFolder` + `IShellView` 枚举桌面项目
- macOS：使用 `NSWorkspace` + `NSFileManager`（实验性）

**2. 智能分类系统**

三级混合方案：

```
第一优先级：本地数据库（自动分类）
  - data/categories.yaml（项目默认）
  - ~/.config/.../categories.yaml（用户自定义）

第二优先级：用户自定义规则（手动定制）
  - 支持手动创建分类框
  - 支持拖拽调整分类

第三优先级：MCP + Agent（后续版本）
  - AI 智能分类（低优先级）
```

**AppCategoryDB 结构**：

```yaml
version: "1.0"
categories:
  - id: games
    name: 游戏
    icon: 🎮
    exe_patterns: ["steam.exe", "epicgameslauncher.exe"]
    path_patterns: ["\\Games\\", "\\Steam\\"]
    keywords: ["game", "play", "娱乐"]

  - id: dev
    name: 开发工具
    icon: 💻
    exe_patterns: ["code.exe", "idea*.exe"]
    path_patterns: ["\\Dev\\", "\\IDE\\"]
    keywords: ["dev", "code", "ide"]
```

**匹配算法**：

```rust
// 简单通配符匹配
exe_name.matches_pattern("idea*.exe")  // 匹配 IntelliJ IDEA
path.contains("\\Games\\")              // 路径包含关键词
```

**3. UI 层**

使用 egui 即时模式 GUI：

- `ClassificationBoxWidget`：可拖拽、可调整尺寸的分类框容器
  - 支持锁定保护
  - 支持样式自定义（圆角、透明、毛玻璃）
- `IconWidget`：图标组件
  - 显示图标名称和图片
  - 支持拖拽到分类框

**egui 窗口配置**：

```rust
// 透明无边框窗口
let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
        .with_title("Desktop Organizer")
        .with_inner_size([800.0, 600.0])
        .with_decorations(false)     // 无边框
        .with_transparent(true),     // 透明背景
    ..Default::default()
};
```

**4. 配置持久化**

**布局配置**（JSON 格式）：

```json
{
  "version": "1.0",
  "last_modified": "2026-06-08T10:00:00Z",
  "boxes": [
    {
      "id": "uuid-...",
      "name": "游戏",
      "position": [100.0, 100.0],
      "size": [200.0, 300.0],
      "locked": false,
      "icon_ids": ["uuid-...", "uuid-..."]
    }
  ]
}
```

**存储位置**：
- macOS/Linux: `~/.config/ikun-desktop-organization/layout.json`
- Windows: `%APPDATA%/ikun-desktop-organization/layout.json`

---

## 数据文件

### 分类数据库

**默认位置**：`data/categories.yaml`

**加载优先级**：
1. 用户配置：`~/.config/.../categories.yaml`
2. 项目默认：`data/categories.yaml`
3. 内置默认：代码中的 `Default` 实现

**用户可编辑**：
- YAML 格式，支持注释
- 用户可手动添加自定义分类规则

---

## 平台差异

### Windows

**技术路径**：Windows Shell API (COM)

**关键 API**：
- `SHGetDesktopFolder()` - 获取桌面文件夹
- `IShellFolder::EnumObjects()` - 枚举项目
- `IShellView::GetItemPosition()` - 获取位置

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

**技术难点**：
- 窗口层级控制（需要 Windows API）
- 桌面图标同步（监听文件系统变化）
- 多显示器支持

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

## MVP 开发路线图

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