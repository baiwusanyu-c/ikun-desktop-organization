# Backend Development Guidelines

> Best practices for backend development in this project.

---

## Overview

本项目是一个 Rust 桌面应用，使用模块化架构和 trait-based 跨平台设计。
这些指南文档化了项目实际的编码规范和最佳实践。

**核心架构**：
- **Rust + egui**：纯 Rust GUI，即时模式渲染
- **Trait-based 设计**：跨平台功能通过 trait 统一接口
- **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离
- **文件持久化**：YAML/JSON 配置存储，使用 `serde` 序列化

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | 模块组织、文件布局、命名规范 | ✅ 已填写 |
| [Database Guidelines](./database-guidelines.md) | 数据持久化模式（YAML/JSON）、加载/保存策略 | ✅ 已填写 |
| [Error Handling](./error-handling.md) | 使用 anyhow::Result、优雅降级、错误日志 | ✅ 已填写 |
| [Quality Guidelines](./quality-guidelines.md) | 禁止模式、必须模式、测试要求、性能优化 | ✅ 已填写 |
| [Logging Guidelines](./logging-guidelines.md) | 日志级别、中文日志、禁止高频日志 | ✅ 已填写 |

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

---

## 开发环境配置

### RustRover 运行配置

项目已配置 4 个 RustRover 运行配置（位于 `.idea/runConfigurations/` 目录）：

| 配置名 | 用途 | 环境变量 | 快捷键 |
|--------|------|----------|--------|
| **Run** | 正常运行 | `RUST_LOG=info` | `Cmd+R` |
| **Debug** | 调试运行 | `RUST_LOG=debug` | `Cmd+D` |
| **Check** | 快速检查代码 | - | 手动启动 |
| **Test** | 运行测试 | `RUST_LOG=debug` | `Cmd+R` |

**推荐使用**：
- 日常开发：选择 **Debug** 配置，按 `Cmd+D`
- 断点调试：设置断点后，用 **Debug** 配置启动

### 自动编译工具

安装 `cargo-watch`（自动监听文件变化）：

```bash
cargo install cargo-watch
```

使用方式：
```bash
# 自动检查（推荐后台运行）
cargo watch -x check

# 自动运行（文件变化时重启）
cargo watch -x run
```

### 日志工具模块

项目提供了日志工具模块 `src/utils/logger.rs`：

```rust
// 使用自定义宏
log_info!("应用启动成功");  // 自动包含模块路径
log_debug!("变量值: {}", value);  // 自动包含文件和行号

// 性能日志
log_perf("图标分类", elapsed_ms);

// 分隔线
log_separator("开始处理桌面图标");
```

### 开发调试教程

详细教程位于 `todo/` 目录：

| 文档 | 说明 |
|------|------|
| [todo/快速入门.md](../../todo/快速入门.md) | 5 分钟上手指南 |
| [todo/开发调试指南.md](../../todo/开发调试指南.md) | 完整开发流程详解 |
| [todo/日志使用示例.md](../../todo/日志使用示例.md) | 日志实战教程 |

---

## Quick Reference

### 关键命令

```bash
# 开发
cargo run              # 开发模式运行
cargo check            # 快速编译检查
cargo watch -x check   # 自动检查（推荐）

# 调试
RUST_LOG=debug cargo run    # 显示调试日志
RUST_LOG=warn cargo run     # 只显示警告和错误

# 质量检查
cargo fmt              # 格式化代码
cargo clippy           # 代码检查
cargo test             # 运行测试
```

### 核心依赖

| Crate | 用途 |
|-------|------|
| `egui + eframe` | GUI 框架 |
| `anyhow` | 错误处理 |
| `log + env_logger` | 日志系统 |
| `serde + serde_json/yaml` | 数据序列化 |
| `dirs` | 跨平台配置路径 |

### 开发工具

| 工具 | 用途 | 安装命令 |
|------|------|----------|
| `cargo-watch` | 自动编译 | `cargo install cargo-watch` |
| `cargo-clippy` | 代码检查 | 已内置 |
| `cargo fmt` | 代码格式化 | 已内置 |

### RustRover 配置

| 配置 | 用途 | 环境变量 |
|------|------|----------|
| Debug | 调试运行 | `RUST_LOG=debug` |
| Run | 正常运行 | `RUST_LOG=info` |
| Check | 快速检查 | - |

### 架构层次

```
UI 层 (ui/)           → egui 组件，用户交互
  ↓
业务层 (classification/, config/) → 业务逻辑，数据处理
  ↓
平台层 (desktop/)     → 平台 API，系统集成
```

---

## Key Principles

1. **安全优先**：避免 `unsafe`，不使用 `panic/unwrap/expect`
2. **优雅降级**：关键功能失败时使用默认值，不阻塞应用
3. **单向依赖**：UI → 业务 → 平台，禁止反向依赖
4. **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离
5. **文档完整**：所有公开 API 必须有文档注释
6. **中文友好**：日志消息使用中文

---

## How These Guidelines Were Created

基于以下内容填写：

1. **可行性分析文档**：技术方案和架构设计
2. **现有代码分析**：[src/](src/) 目录下的实际代码模式
3. **Rust 最佳实践**：社区通用规范

---

**Language**: 本文档使用中文编写（项目用户为中文用户）。