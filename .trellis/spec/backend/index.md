# Backend Development Guidelines

> 代码规范：指导 AI 如何编写符合项目标准的代码（v3.0 架构：Windows-only）

---

## Overview

本项目是一个 Rust Windows 桌面应用，模块化架构，**单一平台、单一 UI 运行时**。

**核心架构原则**：
- **Tauri 2 + Svelte 5 唯一 UI**：四个 WebView 窗口只负责面板/弹窗；原生分类框保持 Win32 + Direct2D
- **共享运行时注入**：Tauri Commands、轮询线程共享 `SharedTauriRuntime` 中唯一的 `AppContext`
- **平台层直接实现**：所有平台代码集中在 `platform/windows/`，调用方使用具体函数/结构体；
  禁止为"未来平台"引入 trait 抽象或 `Box<dyn ...>` 预留（历史 DesktopManagerTrait/PlatformService 已移除）
- **无插件体系**：不存在自研插件注册表；扩展功能直接实现为模块（Tauri 官方插件除外）
- **统一配置**：单一 JSON 文件管理所有设置
- **单向依赖**：Svelte → IPC adapter → SharedTauriRuntime/AppContext → 业务/平台，禁止反向依赖

**架构层次**：
```text
UI 层
  ├── frontend/            Svelte 页面与组件（控制面板/弹窗）
  └── tauri_commands/      IPC adapter、持久窗口策略和原生服务编排
运行时层 (tauri_commands/platform_runtime.rs)  → SharedTauriRuntime(AppContext)
上下文层 (context.rs)      → AppConfig + config_loaded（无 trait 对象）
  ├── 业务层 (box_core/)        → 分类框逻辑 + 拖拽/多选/几何纯逻辑（平台无关）
  ├── 分类层 (classification/)  → 规则匹配 + AI 分类
  ├── 配置层 (config/)          → 统一 JSON 配置（AppConfig）
  └── 平台层 (platform/windows/) → Win32 窗口/D2D 渲染/Shell/托盘/拖拽/覆盖层
工具层 (utils/)            → 日志、DPI、自启注册表、分配探针
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
| [Icon Specification](./icon-specification.md) | 图标提取与显示规范 |

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

### Tauri 运行时 / 窗口
- [ ] 阅读 [Tauri Window Contract](../guides/tauri-window-contract.md) 了解共享状态、IPC 与窗口策略
- [ ] 阅读 [Tauri + Svelte Architecture](../frontend/tauri-svelte-architecture.md) 了解前端契约

### Windows Shell / COM / 分层窗口
- [ ] 阅读 [Windows Shell API Guide](../guides/windows-shell-api-guide.md) 了解正确的 Shell API 调用模式
- [ ] 阅读 [Windows Platform Guidelines](./windows-platform-guidelines.md) 与
      [Layered Window Guide](../guides/windows-layered-window-guide.md)
- [ ] 确认 PIDL 类型（绝对 vs 相对）、菜单层级（文件夹 vs 背景）、verb 偏移

---

## Core Dependencies

| Crate | 用途 |
|-------|------|
| `tauri` + `tauri-plugin-opener` + `tauri-plugin-dialog` + `tauri-plugin-updater` | 唯一 WebView shell、IPC、持久窗口、安全外链、文件对话框与在线升级 |
| `anyhow` | 错误处理（统一使用 `anyhow::Result`） |
| `log + env_logger` | 日志系统 |
| `serde + serde_json + serde_yaml` | 配置/规则序列化 |
| `dirs` | 用户目录/配置路径 |
| `uuid` | 分类框 / 图标唯一 ID |
| `chrono` | 时间处理 |
| `reqwest + tokio` | AI 分类 HTTP 调用（rustls） |
| `notify` | 桌面目录变更监听 |
| `windows`（Windows） | Shell API、Direct2D/DirectWrite、D3D11/DComp/DXGI（含 Direct3D_Fxc：运行时 D3DCompile，勿删） |

---

## Key Principles

1. **安全优先**：避免 `unsafe`，不使用 `panic/unwrap/expect`（可恢复错误）
2. **优雅降级**：关键功能失败时使用默认值，不阻塞应用
3. **上下文注入**：默认路径通过 `SharedTauriRuntime` 传递唯一 `AppContext`；命令不得另建配置内存模型
4. **单向依赖**：Svelte → Tauri Commands → AppContext → 业务/平台，禁止反向依赖
5. **具体实现优先**：Windows-only，不引入平台 trait；平台代码全部收敛在 `platform/windows/`
6. **文档完整**：所有公开 API 必须有文档注释（中文）
7. **中文日志**：日志消息使用中文
8. **单文件 ≤ 1500 行**：渲染引擎等复杂模块可适当放宽；可独立测试的纯逻辑应拆到小模块

---

## Code Quality Requirements

**提交前必须通过**：
- `cargo fmt` 格式化通过
- `cargo clippy` 无严重警告（CI allowlist 见 build.yml，只减不增）
- `cargo test` 测试通过（共享全局状态的测试必须带串行锁，见 Quality Guidelines）
- `pnpm -C frontend run check`（前端有改动时）

**禁止模式**（见 [Quality Guidelines](./quality-guidelines.md)）：
- ❌ `unwrap()` / `expect()` 处理可恢复错误
- ❌ 硬编码配置路径
- ❌ 跨层级反向依赖
- ❌ 不必要的克隆
- ❌ 高频路径日志
- ❌ 手动定义 COM vtable（使用 `windows` crate 安全方法）
- ❌ 混合手动 FFI 和 `windows` crate 类型
- ❌ 新增插件注册表 / 为假设性跨平台预留 trait 抽象

**必须模式**：
- ✅ 所有公开 API 有文档注释
- ✅ 所有模块有模块文档
- ✅ 所有数据结构实现 `Default`
- ✅ 错误处理使用 `.context()` 添加上下文
- ✅ 测试共享全局状态时使用进程内串行锁

---

**Language**: 本文档使用中文编写（项目用户为中文用户）。
