# Frontend Development Guidelines

## Overview

`frontend/` 是 Tauri 2 默认 UI，使用 Svelte 5、TypeScript、Vite 和原生 CSS。三个页面分别对应 `control-panel`、`create-box`、`edit-box`，不负责原生分类框渲染。

## Guidelines Index

| Guide | Description |
| --- | --- |
| [Tauri + Svelte Architecture](./tauri-svelte-architecture.md) | 页面组织、IPC adapter、Svelte 代理和窗口生命周期 |
| [Tauri Window Contract](../guides/tauri-window-contract.md) | Rust/前端命令 payload、共享状态与 DXGI 跨层契约 |

## Pre-Development Checklist

- [ ] 阅读 `tauri-svelte-architecture.md`
- [ ] 阅读 `../guides/tauri-window-contract.md`
- [ ] 新增配置字段时同步 `src/config/app_config.rs` 与 `frontend/src/lib/api/tauri-api.ts`
- [ ] 跨 IPC 发送 `$state` 数据前使用 `$state.snapshot(...)`
- [ ] 确认关闭行为是 prevent + hide，不销毁 WebView

## Required Checks

```text
npm --prefix frontend run check
npm --prefix frontend run build
```
