# Backend Development Guidelines

> 代码规范：指导 AI 如何编写符合项目标准的代码

---

## Overview

本项目是一个 Rust 桌面应用，使用模块化架构和 trait-based 跨平台设计。

**核心架构原则**：
- **Rust + egui**：纯 Rust GUI，即时模式渲染
- **Trait-based 设计**：跨平台功能通过 trait 统一接口
- **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离
- **文件持久化**：YAML/JSON 配置存储，使用 `serde` 序列化
- **单向依赖**：UI → 业务 → 平台，禁止反向依赖

**架构层次**：
```
UI 层 (ui/)           → egui 组件，用户交互
  ↓
业务层 (classification/, config/) → 业务逻辑，数据处理
  ↓
平台层 (desktop/)     → 平台 API，系统集成
```

---

## Guidelines Index

| Guide | Description |
|-------|-------------|
| [Directory Structure](./directory-structure.md) | 模块组织、文件布局、命名规范 |
| [Database Guidelines](./database-guidelines.md) | 数据持久化模式（YAML/JSON）、加载/保存策略 |
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
- [ ] 阅读 [Database Guidelines](./database-guidelines.md) 了解数据结构设计
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
| `egui + eframe` | GUI 框架 |
| `anyhow` | 错误处理（统一使用 `anyhow::Result`） |
| `log + env_logger` | 日志系统 |
| `serde + serde_json/yaml` | 数据序列化 |
| `dirs` | 跨平台配置路径 |
| `windows` (Windows) | Windows Shell API |
| `cocoa/objc` (macOS) | macOS API |

---

## Key Principles

1. **安全优先**：避免 `unsafe`，不使用 `panic/unwrap/expect`
2. **优雅降级**：关键功能失败时使用默认值，不阻塞应用
3. **单向依赖**：UI → 业务 → 平台，禁止反向依赖
4. **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离
5. **文档完整**：所有公开 API 必须有文档注释
6. **中文日志**：日志消息使用中文

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