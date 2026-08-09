# Desktop Organizer Frontend

Desktop Organizer 的 Tauri 2 + Svelte 5 窗口前端，提供以下三个 hash route：

- `#/control-panel`：控制面板
- `#/create-box`：创建分类框
- `#/edit-box`：编辑分类框

Rust IPC 统一封装在 `src/lib/api/tauri-api.ts`，窗口关闭行为统一由
`src/lib/window-lifecycle.ts` 转换为隐藏。

```powershell
npm run check
npm run build
```
