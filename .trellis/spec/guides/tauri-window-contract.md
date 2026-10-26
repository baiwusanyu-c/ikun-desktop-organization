# Tauri 窗口与原生分类框契约

## 适用范围

本契约覆盖 `frontend/src/lib/api/tauri-api.ts`、`frontend/src/lib/window-lifecycle.ts`、
`src/tauri_commands/config_cmd.rs`、`src/tauri_commands/box_cmd.rs` 与
`src/tauri_commands/platform_runtime.rs` 与 `src/tauri_commands/window_cmd.rs`。WebView 窗口只负责 UI，分类框仍由 Win32/D2D 创建和渲染。

## 共享状态契约

`main.rs` 在 Tauri `setup` 中创建一个 `SharedTauriRuntime = Arc<Mutex<TauriRuntime>>`，先通过 `app.manage(...)` 注入 Commands，再把同一个实例交给 `start_platform_runtime`。`TauriRuntime` 持有唯一的 `AppContext`。

- `config_cmd.rs` 和 `box_cmd.rs` 必须使用 `State<'_, SharedTauriRuntime>` 读写配置。
- 成功保存配置后必须调用 `TauriRuntime::replace_config`，同步 `ctx.config` 与依赖分类规则创建的 `ctx.classifier`。
- 禁止在普通更新命令中重新 `AppConfig::load()` 并维护第二份状态；`load_config` 是用户显式要求从磁盘重载时的唯一例外。
- 互斥锁中毒返回中文错误，不得 `unwrap()` 导致 Tauri 事件循环崩溃。

## IPC 契约

| 命令 | 参数 | 成功结果 | 失败行为 |
| --- | --- | --- | --- |
| `get_config` | 无 | `AppConfig` 对象 | 返回中文错误字符串，页面显示加载错误 |
| `update_theme_color` | `{ color: string }` | 最新 `AppConfig` | 不写入不完整颜色 |
| `update_box_style` | `{ style: BoxStyleConfig }` | 最新 `AppConfig` | Rust `normalize` 后持久化并重绘所有分类框 |
| `update_liquid_glass` | `{ config: LiquidGlassConfig }`（嵌套：liquid 37 项 / transparent / resize / engine） | 最新 `AppConfig` | 归一化后实时下发玻璃引擎样式；引擎级参数变化触发引擎自动重建 |
| `update_ai_config` | `{ config: AiConfig }` | 最新 `AppConfig` | 保留其它配置字段，仅更新 AI 部分 |
| `create_box` | `{ name: string, description: string }` | `BoxData` | 空名、超过 16 字符、同名或原生创建失败时返回错误 |
| `edit_box` | `{ id: string, name: string, description: string }` | `BoxData` | 无效 UUID、空名、同名或不存在时返回错误 |
| `get_pending_edit_box` | 无 | `EditBoxPayload` 或 `null` | 无 pending 数据时返回 `null` |
| `show_window` / `hide_window` / `close_window` | `{ label: string }` | `()` | 仅接受三个固定标签；`close_window` 对兼容调用也执行 hide |
| `open_url` | `{ url: string }` | `()` | 仅允许带 host 的 HTTP/HTTPS URL，由 `tauri-plugin-opener` 打开 |

`capabilities/default.json` 只授予 `opener:allow-open-url`，不得恢复通用 shell 执行或文件打开权限。

`EditBoxPayload` 的字段固定为 `{ config_id: string, current_name: string }`。
双击原生分类框标题后，Rust 先写入 pending，再发送 `edit-box-opened` 事件；编辑页先注册监听，再读取 pending，以覆盖首次事件竞态。

## 窗口居中契约

所有 Tauri 绘制的弹窗（`control-panel`、`create-box`、`edit-box`、`ai-classify`）
默认位置必须是**主屏幕水平垂直居中**：

- `tauri.conf.json` 中每个窗口设置 `center: true`，保证创建时即居中。
- `window_cmd.rs::center_on_primary_monitor` 使用 `app.primary_monitor()` 的
  position + size 与 `window.outer_size()`（均为物理像素）计算居中坐标。
- `show_managed_window` 在每次 `show()` 前调用居中，保证“默认位置”契约；
  居中失败只记 `warn!`，不阻断显示。

## 窗口生命周期

`control-panel`、`create-box`、`edit-box` 的关闭都必须是 `prevent_close + hide`，不能调用 `close()`。
Rust `on_window_event` 和前端 `installHideOnClose` 双重拦截；托盘/信号再次打开时使用同一个 WebView 实例并 `show + set_focus`。

真正退出应用必须设置共享运行时的 `quit_requested`，由平台轮询线程保存分类框布局、清理注册表/托盘、销毁原生分类框并恢复桌面图标后再执行 `app.exit(0)`。Tauri Command 不得直接退出进程而绕过这条清理路径。
所有 Rust 入口必须复用 `window_cmd::{show_managed_window, hide_managed_window, toggle_managed_window}`，不得在轮询线程复制标签和窗口操作。

## Svelte 配置状态

`ControlPanel.svelte` 将 `AppConfig` 存入 Svelte 5 `$state` 后，子组件收到的是响应式代理。
`StyleSettings.svelte` 与 `AiSettings.svelte` 必须使用 `$state.snapshot(...)` 生成传给 Tauri Command 的普通对象；
不得对 `$state` 代理直接调用 `structuredClone`，否则 WebView 会抛出 `DataCloneError`，页面因初始化 effect 中断而显示为空。

主题色更新还必须通过 `theme-color-changed` 事件广播到持久的 `create-box` 和 `edit-box` WebView；
两个窗口重新收到 `window-opened` 时应再次调用 `get_config`，作为事件丢失或外部配置变更时的兜底。

## 液态玻璃引擎帧（liquid-verre-rs）

液态玻璃由 `liquid-verre-rs` 引擎（`src/platform/windows/glass_engine.rs` 适配层）在独立渲染线程持续采集/合成；平台层不得自行启动第二个采集器或把玻璃像素画回内容 DIB。分类框内容窗口只是玻璃之上的前景层。
DXGI 初始化阶段可能返回平均 RGB 接近 `(1,1,1)` 的不透明占位黑帧；渲染器最多跳过前三帧，并由
`src/platform/windows/d2d_render.rs` 使用白色半透明回退，取得真实桌面帧后立即缓存。单帧捕获失败继续使用上一成功帧。

## 验证用例

- Good：修改圆角/背景色/玻璃开关后，`update_box_style` 或 `update_liquid_glass` 返回成功，现有分类框立即重绘。
- Good：关闭任意 WebView 后再次通过托盘或信号打开，窗口仍可见且编辑页数据正确。
- Good：UI 更新配置后，下一次托盘 AI 分类和原生恢复布局读取同一个 `AppContext` 中的新值。
- Base：DXGI `WAIT_TIMEOUT`、`ACCESS_LOST` 或单帧 D2D 错误只保留上一成功帧，不进入黑色回退；显示器 DPI 变化会使快照失效并重新采集。
- Base：Svelte 配置使用 `$state.snapshot` 后再跨 IPC；初始化 effect 不得抛出 `DataCloneError`。
- Bad：DXGI 占位黑帧不得进入成功帧缓存，也不得作为分类框首次可见画面提交。
- Bad：缺失命令参数、非法名称或不存在的分类框不得静默成功，前端必须显示错误状态。
- Bad：未知窗口标签、`file:`/`javascript:` URL 和锁中毒不得触发窗口或进程级副作用。

## 必需检查

```text
npm run check
npm run build
cargo check
cargo test
```
