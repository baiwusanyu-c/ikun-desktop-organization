# Tauri + Svelte 5 前端架构

## 目录职责

| 路径 | 职责 |
| --- | --- |
| `frontend/src/App.svelte` | 根据 hash route 选择三个窗口页面 |
| `frontend/src/lib/pages/` | 页面状态、表单编排和用户错误提示 |
| `frontend/src/lib/components/` | 无业务持久化职责的可复用控件 |
| `frontend/src/lib/api/tauri-api.ts` | 唯一 Tauri `invoke` adapter 与 IPC TypeScript 类型 |
| `frontend/src/lib/window-lifecycle.ts` | 当前窗口 hide 与 close 拦截 |
| `frontend/src/lib/styles/` | 全局主题和毛玻璃窗口样式 |

依赖方向固定为：`page -> component/api/window-lifecycle`。组件不得直接调用 Rust Command；Rust 不依赖前端源码。

## IPC 状态

Rust `AppConfig` 是协议源，TypeScript 接口必须保持字段名和类型一致。`get_config` 返回对象；`parseConfig` 仅兼容迁移早期可能返回的 JSON 字符串。

Svelte 5 `$state` 会生成 Proxy。以下写法是强制契约：

```ts
const payload = $state.snapshot(boxStyle);
await updateBoxStyle(payload);
```

禁止对 Proxy 直接调用 `structuredClone` 或传给 `invoke`，否则 WebView 产生 `DataCloneError`。表单更新按 Promise 队列串行提交，避免高频 slider 更新乱序覆盖。

## 窗口生命周期

- 页面挂载时调用 `installHideOnClose`。
- 标题栏 X、Escape、取消和原生 CloseRequested 都执行 hide。
- `EditBox.svelte` 先监听 `edit-box-opened`，再调用 `get_pending_edit_box`，覆盖首次打开竞态。
- 页面卸载必须释放 event 与 close listener。

## 错误状态

| 边界 | UI 行为 |
| --- | --- |
| `get_config` 失败 | 控制面板显示加载错误，不渲染空表单 |
| 配置保存失败 | 当前分组显示错误，保留用户输入 |
| 名称校验失败 | 对话框内显示服务端消息，不隐藏窗口 |
| 编辑 payload 缺失 | 禁用提交并提示重新打开编辑窗口 |

## Good / Base / Bad

- Good：`StyleSettings` 与 `AiSettings` 用 `$state.snapshot` 生成 IPC payload。
- Good：关闭任一窗口后可从托盘、热键或标题双击再次打开。
- Base：初始化数据未到达时显示加载/等待状态，布局尺寸不跳变。
- Bad：页面或通用组件中散落裸 `invoke()`。
- Bad：X 按钮调用 `close()` 导致 WebView 被销毁。
- Bad：Rust/TypeScript 配置字段只改一侧。


## 窗口样式约定

### `.window-shadow`

`App.svelte` 根节点使用 `.window-shadow` 包裹层实现 CSS drop-shadow，配合 `tauri.conf.json` 的 `"shadow": false` 关闭原生阴影。

### `.dialog-window`

`create-box` 和 `edit-box` 页面使用 `.dialog-window` CSS 类，样式统一定义在 `glass.css` 中。组件页面不内联重复样式。

### 跨窗口主题色同步

`lib/theme.ts` 的 `applyThemeColorAcrossWindows(color)` 通过 Tauri 事件 `theme-color-changed` 广播主题色变更到所有持久 WebView 窗口。
`installThemeColorSync()` 在页面挂载时注册监听，返回 cleanup 函数在卸载时释放。

`lib/utils/color.ts` 提供 `hexToRgb`、`adjustBrightness`、`applyThemeColor` 工具函数。

## 前端 IPC 日志

`lib/api/tauri-api.ts` 导出 `logToRust(level, message)` 函数，将前端日志上报到 Rust 后端的 `log_message` 命令。

## 必需检查

```text
npm --prefix frontend run check
npm --prefix frontend run build
cargo test
```
