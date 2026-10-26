# Trellis Spec 索引

> 项目规范与指导文档总览（v3.0 架构：Windows-only，单一 Tauri 运行时）

---

## 目录结构

```text
.trellis/spec/
├── backend/          # Rust 后端开发指南
│   ├── index.md                  # Backend 指南入口
│   ├── directory-structure.md    # 模块组织规则
│   ├── database-guidelines.md    # 数据持久化模式（统一 JSON 配置）
│   ├── error-handling.md         # 错误处理规范
│   ├── icon-specification.md     # 图标规范
│   ├── logging-guidelines.md     # 日志系统规范
│   ├── quality-guidelines.md     # 质量红线（禁止/必须模式）
│   └── windows-platform-guidelines.md  # Windows 平台行为规范（分层窗口/Shell/D2D）
├── frontend/         # Tauri + Svelte 5 前端规范
│   ├── index.md
│   └── tauri-svelte-architecture.md  # IPC、状态和持久化契约
└── guides/           # 思维与方法指南
    ├── index.md
    ├── tauri-window-contract.md          # 共享运行时与窗口策略契约
    ├── code-reuse-thinking-guide.md
    ├── cross-layer-thinking-guide.md
    ├── windows-layered-window-guide.md
    └── windows-shell-api-guide.md
```

---

## 快速导航

### Backend 开发指南

**必读文档**（开始任务前）：

1. [Backend Index](backend/index.md) - 概览和快速参考
2. [Directory Structure](backend/directory-structure.md) - 模块组织规则
3. [Error Handling](backend/error-handling.md) - 错误处理规范
4. [Quality Guidelines](backend/quality-guidelines.md) - 禁止和必须模式
5. [Logging Guidelines](backend/logging-guidelines.md) - 日志规范
6. [Windows Platform Guidelines](backend/windows-platform-guidelines.md) - 分层窗口/Shell/渲染行为

### Frontend 开发指南

1. [Frontend Index](frontend/index.md) - 前端开发规则
2. [Tauri + Svelte Architecture](frontend/tauri-svelte-architecture.md) - IPC、响应式状态与持久化

### 数据持久化设计

- [Database Guidelines](backend/database-guidelines.md) - 统一 JSON 配置管理

### 思维指南

- [Guides Index](guides/index.md)
- [Tauri Window Contract](guides/tauri-window-contract.md) - 共享运行时、命令与窗口生命周期
- [Code Reuse Thinking](guides/code-reuse-thinking-guide.md)
- [Cross-Layer Thinking](guides/cross-layer-thinking-guide.md)
- [Windows Shell API](guides/windows-shell-api-guide.md)
- [Windows Layered Window Guide](guides/windows-layered-window-guide.md)

---

## 核心架构（v3.0）

### 技术栈

- **Windows 10/11 专属产品**：无跨平台承诺，禁止 macOS/Linux 预留代码与"回退路径"残留。
- **Tauri 2 + Svelte 5**：控制面板、创建/编辑弹窗、AI 分类弹窗（WebView2）。
- **Win32 + Direct2D/DirectWrite**：分类框与桌面覆盖层原生窗口（不进 WebView）。
- **liquid-verre-rs（crates.io 依赖库）**：液态玻璃 / 透明玻璃底板（WGC/DDA 采集 + D3D11 模糊金字塔 + DirectComposition 合成 + Snell 折射，着色器由 D3DCompile 运行时编译）；分类框内容层仅绘制前景（D2D/DirectWrite）。
- **单一运行时注入**：Tauri Commands 与平台轮询线程共享唯一 `SharedTauriRuntime`（内含 `AppContext`）。
- **无插件体系**：自研 PluginRegistry 已移除；Tauri 官方插件（opener/dialog）正常使用。
- **无 trait 平台抽象**：平台能力直接调用 `platform/windows` 具体实现。
- **统一 JSON 配置**：单一 config.json 管理所有设置。

### 模块依赖

```text
Svelte(frontend) → tauri_commands(IPC) → TauriRuntime/AppContext
  ├── box_core/          分类框业务逻辑 + 拖拽/多选/几何纯逻辑（平台无关）
  ├── classification/    规则匹配 + AI 分类
  ├── config/            统一 JSON 配置
  ├── platform/windows/  Win32/COM/D2D 全部原生实现
  └── utils/             日志、DPI、自启注册表、分配探针
```

方向约束：只能自上而下依赖；platform 内不得引用 tauri_commands；box_core 不得引用 windows。

---

## 常用命令

```bash
cargo run                       # 运行（唯一 Tauri 路径）
RUST_LOG=debug cargo run        # 调试日志运行
cargo check --all-targets       # 快速检查
cargo test                      # 单元测试（门禁）
cargo fmt                       # 格式化
cargo clippy                    # Lint
pnpm run pack:win               # 发布打包（NSIS/MSI）→ target/dist/
pnpm -C frontend run check      # 前端类型检查
```

---

**更新时间**：2026-10-23
