# Trellis Spec 索引

> 项目规范和指南文档索引（v2.0 架构）

---

## 目录结构

```
.trellis/spec/
├── backend/          # Rust 后端开发指南
│   ├── index.md                  # Backend 指南索引
│   ├── directory-structure.md    # 模块组织规则
│   ├── plugin-architecture.md    # 进程内插件生命周期与热安装/卸载契约
│   ├── database-guidelines.md    # 数据持久化模式
│   ├── error-handling.md         # 错误处理规范
│   ├── quality-guidelines.md     # 代码质量要求
│   └── logging-guidelines.md     # 日志系统规范
├── frontend/         # Tauri + Svelte 5 前端规范
│   ├── index.md                  # Frontend 指南索引
│   └── tauri-svelte-architecture.md # IPC、状态和持久窗口契约
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
6. [Plugin Architecture](backend/plugin-architecture.md) - 插件生命周期与扩展边界

### Frontend 开发指南

1. [Frontend Index](frontend/index.md) - 前端开发入口
2. [Tauri + Svelte Architecture](frontend/tauri-svelte-architecture.md) - IPC、响应式状态和窗口生命周期

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

- **Tauri 2 + Svelte 5**：Windows 默认控制面板与创建/编辑弹窗
- **Win32 + Direct2D/DirectWrite**：原生分类框与桌面覆盖层
- **Trait-based 设计**：跨平台功能通过 trait 统一接口
- **条件编译**：平台特定代码使用 `#[cfg(...)]` 隔离
- **SharedTauriRuntime 上下文注入**：Commands、后台线程和插件共享同一个 `AppContext`
- **进程内插件化**：trait 插件支持运行时安装/卸载，不提供动态库 ABI 加载
- **统一 JSON 配置**：单一 config.json 管理所有设置

### 模块层次

```
UI adapter (frontend/ + tauri_commands/) → IPC 与原生服务编排
运行时层 (SharedTauriRuntime) → AppContext + PluginRegistry
上下文层 (context.rs)        → 配置、分类器、平台 trait 对象
  ├── 平台层 (platform/)     → 平台抽象 + Windows/macOS 实现
  ├── 业务层 (box_core/)     → 分类框业务逻辑（平台无关）
  ├── 分类层 (classification/) → 分类规则 + AI 分类
  ├── 配置层 (config/)       → 统一 JSON 配置
  └── 插件层 (plugin/)       → Plugin trait + Registry
legacy UI (gpui_app/, ui/)   → 显式回退路径
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
