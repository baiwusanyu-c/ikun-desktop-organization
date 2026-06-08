# Trellis Spec 索引

> 项目规范和指南文档索引

---

## 目录结构

```
.trellis/spec/
├── backend/          # Rust 后端开发指南
│   ├── index.md                  # Backend 指南索引
│   ├── directory-structure.md    # 模块组织规则
│   ├── database-guidelines.md    # 数据持久化模式
│   ├── error-handling.md         # 错误处理规范
│   ├── quality-guidelines.md     # 代码质量要求
│   └── logging-guidelines.md     # 日志系统规范
│
└── guides/           # 思维指南
    ├── index.md                  # 思维指南索引
    ├── code-reuse-thinking-guide.md    # 代码复用思维
    └── cross-layer-thinking-guide.md   # 跨层思维
```

---

## 快速导航

### Backend 开发指南

**必读文档**（开始开发前）：

1. [Backend Index](backend/index.md) - 总览和快速参考
2. [Directory Structure](backend/directory-structure.md) - 模块组织规则
3. [Error Handling](backend/error-handling.md) - 错误处理规范
4. [Quality Guidelines](backend/quality-guidelines.md) - 禁止和必须模式
5. [Logging Guidelines](backend/logging-guidelines.md) - 日志规范

**数据持久化相关**：
- [Database Guidelines](backend/database-guidelines.md) - YAML/JSON 配置管理

### 思维指南

**编码前思维扩展**：

- [Guides Index](guides/index.md) - 思维指南总览
- [Code Reuse Thinking](guides/code-reuse-thinking-guide.md) - 识别重复模式
- [Cross-Layer Thinking](guides/cross-layer-thinking-guide.md) - 跨层数据流思考

---

## 开发环境配置

### RustRover 运行配置

项目已配置 4 个 RustRover 运行配置：

| 配置名 | 用途 | 环境变量 |
|--------|------|----------|
| **Run** | 正常运行 | `RUST_LOG=info` |
| **Debug** | 调试运行 | `RUST_LOG=debug` |
| **Check** | 快速检查 | - |
| **Test** | 运行测试 | `RUST_LOG=debug` |

配置文件位置：`.idea/runConfigurations/*.xml`

### 自动编译工具

安装 `cargo-watch`：

```bash
cargo install cargo-watch
```

使用方式：
```bash
# 自动检查
cargo watch -x check

# 自动运行
cargo watch -x run
```

### 开发调试教程

详细教程文档位置：`todo/`

| 文档 | 说明 |
|------|------|
| [todo/快速入门.md](../todo/快速入门.md) | 5 分钟上手指南 |
| [todo/开发调试指南.md](../todo/开发调试指南.md) | 完整开发流程 |
| [todo/日志使用示例.md](../todo/日志使用示例.md) | 日志实战教程 |

---

## 核心架构

### 技术栈

- **Rust + egui**：纯 Rust GUI，即时模式渲染
- **Trait-based 设计**：跨平台功能通过 trait 统一接口
- **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离

### 模块层次

```
UI 层 (src/ui/)
  ↓
业务层 (src/classification/, src/config/)
  ↓
平台层 (src/desktop/)
  ↓
工具层 (src/utils/)
```

---

## 开发流程

### 日常开发

```
1. 阅读 backend/index.md 和相关指南
2. 在 RustRover 选择 Debug 配置
3. Cmd+D 启动调试
4. 查看日志/断点调试
5. 修改代码，重新运行
```

### 新功能开发

```
1. 阅读 Directory Structure 了解模块位置
2. 阅读 Quality Guidelines 了解禁止模式
3. 阅读 Error Handling 了解错误处理
4. 实现功能
5. 添加日志（遵循 Logging Guidelines）
6. 测试验证
```

---

## 快速参考

### 常用命令

```bash
# 开发
cargo run              # 运行
cargo check            # 快速检查
cargo watch -x check   # 自动检查

# 调试
RUST_LOG=debug cargo run    # 详细日志

# 质量
cargo fmt               # 格式化
cargo clippy            # 代码检查
cargo test              # 测试
```

### RustRover 操作

| 操作 | 配置 | 快捷键 |
|------|------|--------|
| 运行 | Run | `Cmd+R` |
| 调试 | Debug | `Cmd+D` |
| 测试 | Test | `Cmd+R` |

---

**更新时间**：2026-06-08