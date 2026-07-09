# Backend Development Guidelines

> 代码规范：指导 AI 如何编写符合项目标准的代码（v2.0 架构）

---

## Overview

本项目是一个 Rust 桌面应用，使用模块化架构和 trait-based 跨平台设计。

**核心架构原则**：
- **Rust + egui**：纯 Rust GUI，即时模式渲染
- **AppContext 上下文注入**：所有模块通过中心上下文共享状态和配置
- **Trait-based 设计**：跨平台功能通过 trait 统一接口
- **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离
- **插件化**：功能模块作为 Plugin 注册，灵活扩展
- **统一配置**：单一 JSON 文件管理所有设置
- **单向依赖**：UI → AppContext → 业务/平台，禁止反向依赖

**架构层次**：
```
上下文层 (context.rs)           → AppContext 中心依赖注入对象
  ├── 平台层 (platform/)        → 平台抽象 trait + Windows/macOS 实现
  ├── 业务层 (box_core/)        → 分类框业务逻辑（平台无关）
  ├── 分类层 (classification/)  → 分类规则 + AI 分类
  ├── 配置层 (config/)          → 统一 JSON 配置（AppConfig）
  └── 插件层 (plugin/)          → Plugin trait + Registry
UI 层 (app.rs, ui/)             → egui 控制面板，通过 AppContext 交互
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

### Windows Shell / COM API
- [ ] 阅读 [Windows Shell API Guide](../guides/windows-shell-api-guide.md) 了解正确的 Shell API 调用模式
- [ ] 确认 PIDL 类型（绝对 vs 相对）、菜单层级（文件夹 vs 背景）、verb 偏移

---

## Core Dependencies

| Crate | 用途 |
|-------|------|
| `egui + eframe` | GUI 框架（控制面板窗口） |
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
3. **上下文注入**：通过 AppContext 传递依赖，不散落全局变量
4. **单向依赖**：UI → AppContext → 业务/平台，禁止反向依赖
5. **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离
6. **文档完整**：所有公开 API 必须有文档注释
7. **中文日志**：日志消息使用中文
8. **单文件 ≤ 1000 行**：严格控制代码规模

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
