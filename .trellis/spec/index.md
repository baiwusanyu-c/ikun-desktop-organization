# Trellis Spec 索引

> 项目规范和指南文档索引（v2.0 架构）

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
    ├── cross-layer-thinking-guide.md   # 跨层思维
    └── windows-shell-api-guide.md      # Windows Shell API 指南
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
- [Database Guidelines](backend/database-guidelines.md) - 统一 JSON 配置管理

### 思维指南

- [Guides Index](guides/index.md) - 思维指南总览
- [Code Reuse Thinking](guides/code-reuse-thinking-guide.md) - 识别重复模式
- [Cross-Layer Thinking](guides/cross-layer-thinking-guide.md) - 跨层数据流思考
- [Windows Shell API](guides/windows-shell-api-guide.md) - Shell/COM API 调用模式

---

## 核心架构

### 技术栈

- **Rust + egui**：纯 Rust GUI，即时模式渲染
- **Trait-based 设计**：跨平台功能通过 trait 统一接口
- **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离
- **AppContext 上下文注入**：所有模块通过中心上下文共享状态
- **插件化**：功能模块作为插件注册
- **统一 JSON 配置**：单一 config.json 管理所有设置

### 模块层次

```
上下文层 (context.rs)        → AppContext 中心依赖注入
  ├── 平台层 (platform/)     → 平台抽象 + Windows/macOS 实现
  ├── 业务层 (box_core/)     → 分类框业务逻辑（平台无关）
  ├── 分类层 (classification/) → 分类规则 + AI 分类
  ├── 配置层 (config/)       → 统一 JSON 配置
  └── 插件层 (plugin/)       → Plugin trait + Registry
UI 层 (ui/, app.rs)          → egui 控制面板，通过 AppContext 交互
工具层 (utils/)              → 日志、DPI、字符串
```

---

## 常用命令

```bash
cargo run                       # 运行
RUST_LOG=debug cargo run        # 调试日志
cargo check                     # 快速检查
cargo fmt                       # 格式化
cargo clippy                    # Lint
cargo test                      # 测试
cargo watch -x check            # 自动检查
```

---

**更新时间**：2026-06-12
