# Windows 系统托盘入口

## Goal
在 Windows 下将应用入口迁移到系统托盘，避免控制面板窗口启动后出现在任务栏。

## Requirements
- Windows 启动时控制面板默认不显示。
- 控制面板窗口不显示任务栏图标。
- 系统托盘图标双击打开控制面板。
- 系统托盘图标单击或右键单击打开菜单。
- 托盘菜单包含：打开控制面板、创建分类框、一键AI分类、退出应用。
- 一键AI分类当前只显示菜单项，点击后不执行具体分类功能。
- 托盘菜单除“一键AI分类”外，其余菜单项都要执行对应功能。
- 控制面板窗口关闭后只隐藏，不退出应用。

## Acceptance Criteria
- [x] `cargo check` 通过。
- [x] `cargo test` 通过。
- [ ] 启动应用后默认只出现系统托盘图标，不弹出控制面板。
- [ ] 双击托盘图标会显示并聚焦控制面板。
- [ ] 单击或右键托盘图标会显示菜单。
- [ ] 菜单“打开控制面板”会打开 `Desktop Organizer 控制面板` 窗口。
- [ ] 菜单“创建分类框”会打开现有创建分类框对话框。
- [ ] 菜单“退出应用”会清理托盘图标并退出应用。
- [ ] 关闭 `Desktop Organizer 控制面板` 窗口只隐藏窗口，不退出应用。

## Technical Notes
- 托盘窗口实现放在 `src/platform/windows/sys_tray.rs`。
- 托盘动作通过平台层状态队列传给 `app.rs`，避免 Win32 消息回调直接依赖 UI 层。
- 主 viewport 使用 `with_visible(false)` 和 `with_taskbar(false)`。
- `src/platform/windows/control_panel_window.rs` 使用 Win32 窗口样式兜底隐藏/显示控制面板，并移除任务栏入口。
