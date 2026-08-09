# Backend Development Guidelines

> 代码规范：指导 AI 如何编写符合项目标准的代码（v2.0 架构）

---

## Overview

本项目是一个 Rust 桌面应用，使用模块化架构和 trait-based 跨平台设计。

**核心架构原则**：
- **Tauri 2 + Svelte 5 默认 UI**：三个 WebView 只负责 UI；原生分类框保持 Win32 + Direct2D
- **共享运行时注入**：Tauri Commands、轮询线程和插件共享 `SharedTauriRuntime` 中唯一的 `AppContext`
- **Trait-based 设计**：跨平台功能通过 trait 统一接口
- **条件编译**：平台特定代码和 legacy UI 使用 `#[cfg(...)]` 隔离
- **插件扩展边界**：`PluginRegistry` 已接入 Tauri 生命周期，支持进程内 trait 插件热安装/卸载；平台渲染和 Shell 细节仍留在 `platform/`
- **统一配置**：单一 JSON 文件管理所有设置
- **单向依赖**：Svelte → IPC adapter → SharedTauriRuntime/AppContext → 业务/平台，禁止反向依赖

**架构层次**：
```
UI 层
  ├── frontend/ (默认) → Svelte 页面与组件
  ├── tauri_commands/ → IPC adapter、持久窗口策略和原生服务编排
  └── gpui_app/ + app.rs + ui/ → legacy 回退路径
运行时层 (platform_runtime.rs)  → SharedTauriRuntime(AppContext + PluginRegistry)
上下文层 (context.rs)           → 配置、分类器和平台 trait 对象
  ├── 平台层 (platform/)        → 平台抽象 trait + Windows/macOS 实现
  ├── 业务层 (box_core/)        → 分类框业务逻辑（平台无关）
  ├── 分类层 (classification/)  → 分类规则 + AI 分类
  ├── 配置层 (config/)          → 统一 JSON 配置（AppConfig）
  └── 插件层 (plugin/)          → Plugin trait + Registry
工具层 (utils/)                 → 日志、DPI、字符串工具
```

---

## Guidelines Index

| Guide | Description |
|-------|-------------|
| [Directory Structure](./directory-structure.md) | 模块组织、文件布局、命名规范 |
| [Database Guidelines](./database-guidelines.md) | 统一 JSON 配置管理、加载/保存策略 |
| [Error Handling](./error-handling.md) | 使用 anyhow::Result、优雅降级、错误日志 |
| [Quality Guidelines](./quality-guidelines.md) | 禁止模式、必须模式、测试要求 |
| [Logging Guidelines](./logging-guidelines.md) | 日志级别、中文日志、禁止高频日志 |
| [Windows Platform Guidelines](./windows-platform-guidelines.md) | WS_EX_LAYERED alpha 行为、坐标系统、D2D 渲染 |
| [Plugin Architecture](./plugin-architecture.md) | 进程内插件安装、卸载、分发、错误与测试契约 |

---

## Pre-Development Checklist

**开始开发前必须阅读**：

### 新模块开发
- [ ] 阅读 [Directory Structure](./directory-structure.md) 了解模块组织规则
- [ ] 阅读 [Quality Guidelines](./quality-guidelines.md) 了解禁止和必须模式
- [ ] 阅读 [Error Handling](./error-handling.md) 了解错误处理规范

### 数据持久化
- [ ] 阅读 [Database Guidelines](./database-guidelines.md) 了解统一配置设计
- [ ] 阅读 [Error Handling](./error-handling.md) 了解加载失败降级策略

### 跨平台功能
- [ ] 阅读 [Directory Structure](./directory-structure.md) 了解平台特定代码规则
- [ ] 阅读 [Quality Guidelines](./quality-guidelines.md) 了解条件编译模式

### 插件或 Tauri 运行时
- [ ] 阅读 [Plugin Architecture](./plugin-architecture.md) 了解生命周期和扩展边界
- [ ] 阅读 [Tauri Window Contract](../guides/tauri-window-contract.md) 了解共享状态、IPC 与窗口策略

### Windows Shell / COM API
- [ ] 阅读 [Windows Shell API Guide](../guides/windows-shell-api-guide.md) 了解正确的 Shell API 调用模式
- [ ] 确认 PIDL 类型（绝对 vs 相对）、菜单层级（文件夹 vs 背景）、verb 偏移

---

## Core Dependencies

| Crate | 用途 |
|-------|------|
| `tauri + tauri-plugin-opener` | 默认 WebView shell、IPC、持久窗口和安全外链 |
| `gpui + gpui-component` (Windows) | legacy Windows UI 回退路径 |
| `egui + eframe` | legacy 控制面板回退路径（`eframe-ui` feature） |
| `anyhow` | 错误处理（统一使用 `anyhow::Result`） |
| `log + env_logger` | 日志系统 |
| `serde + serde_json/yaml` | 数据序列化 |
| `dirs` | 跨平台配置路径 |
| `uuid` | 分类框 / 图标唯一 ID |
| `chrono` | 时间处理 |
| `windows` (Windows) | Windows Shell API、Direct2D/DirectWrite、GDI |
| `cocoa/objc` (macOS) | macOS API（NSWindow/NSWorkspace/CGContext 渲染） |

---

## Key Principles

1. **安全优先**：避免 `unsafe`，不使用 `panic/unwrap/expect`
2. **优雅降级**：关键功能失败时使用默认值，不阻塞应用
3. **上下文注入**：默认路径通过 `SharedTauriRuntime` 传递唯一 `AppContext`；命令不得另建配置内存模型
4. **单向依赖**：Svelte → Tauri Commands → AppContext → 业务/平台，禁止反向依赖
5. **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离
6. **文档完整**：所有公开 API 必须有文档注释
7. **中文日志**：日志消息使用中文
8. **单文件 ≤ 1500 行**：渲染引擎等复杂模块可适当放宽；可独立测试的纯逻辑应拆到小模块

---

## Code Quality Requirements

**提交前必须通过**：
- `cargo fmt` 格式化通过
- `cargo clippy` 无严重警告
- `cargo test` 测试通过

**禁止模式**（见 [Quality Guidelines](./quality-guidelines.md)）：
- ❌ `unwrap()` / `expect()` 处理可恢复错误
- ❌ 硬编码配置路径
- ❌ 跨层级反向依赖
- ❌ 不必要的克隆
- ❌ 高频路径日志
- ❌ 手动定义 COM vtable（使用 `windows` crate 安全方法）
- ❌ 混合手动 FFI 和 `windows` crate 类型

**必须模式**：
- ✅ 所有公开 API 有文档注释
- ✅ 所有模块有模块文档
- ✅ 所有数据结构实现 `Default`
- ✅ 错误处理使用 `.context()` 添加上下文
- ✅ 平台特定代码使用条件编译

---

**Language**: 本文档使用中文编写（项目用户为中文用户）。
