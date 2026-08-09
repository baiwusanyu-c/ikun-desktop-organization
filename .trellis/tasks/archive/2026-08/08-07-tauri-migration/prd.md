# Tauri 2.0 + Svelte 5 迁移

## 目标
将编辑分类框、创建分类框、控制面板三个窗口从 GPUI 迁移到 Tauri 2.0 + Svelte 5 架构。
只重写三个窗口的 UI 层，窗口如何被打开/关闭的交互机制不变。

## 已知信息
* 现有 Rust 代码在 `src/` 目录下，不需迁移到 `src-tauri/`
* Tauri 作为依赖集成到现有 Rust 项目中
* 前端使用 Svelte 5 + Vite，放在 `frontend/` 目录
* 样式方案：原生 CSS + CSS Variables，匹配毛玻璃风格
* 图标库：Lucide Svelte
* 6 个 Phase：初始化 → Commands 层 → 创建/编辑框 → 控制面板风格 → AI+关于 → 集成测试

## 技术选型
| 项目 | 选型 |
|------|------|
| 框架 | Tauri 2.0 (Rust + WebView) |
| 前端 | Svelte 5 + Vite |
| 样式 | 原生 CSS + CSS Variables |
| 图标 | Lucide Svelte |
| 路由 | svelte-spa-router (hash) |

## 分步实施
- Phase 1: 项目初始化 (Tauri + Svelte 5 可运行, 3窗口配置, 基础组件库)
- Phase 2: Rust Commands 层 (config_cmd, box_cmd, app_cmd, 信号文件监听)
- Phase 3: 创建分类框 + 编辑分类框 (Svelte 页面 + Commands 对接)
- Phase 4: 控制面板 - 风格设置 Tab
- Phase 5: 控制面板 - AI 设置 + 关于 Tab
- Phase 6: 集成测试与废弃清理

## 需求
* 保留所有现有窗口交互机制（托盘、右键、信号文件、快捷键）
* 保留 overlay 原生渲染（桌面覆盖层不改造）
* 配置向后兼容（serde default）
* 所有中文 UTF-8 编码

## 验收标准
- [x] Phase 1: Tauri + Svelte 5 项目可运行
- [x] Phase 1: 三个窗口可在浏览器中预览
- [x] Phase 2: 所有 Tauri Commands 可被前端调用
- [x] Phase 3-5: 各页面功能完整
- [x] Phase 6: 集成测试通过，旧代码清理

## 明确排除范围
* 不改造 overlay（桌面覆盖层保持原生 Win32 + D2D）
* 不改变窗口开关交互机制
* 不破坏现有配置兼容性

## 技术笔记
* 参考: ~/Desktop/Tauri_Migration_Plan.md (完整迁移计划)
* Cargo.toml: 现有依赖包括 gpui, windows API, serde 等
* main.rs: 当前从 GPUI 启动，需切换为 Tauri Builder
* 现有 HTML: control_panel.html, create_category_box.html, edit_category_box.html
