# 分类框右键菜单和标题优化

## Goal
优化分类框的标题文字大小和右键菜单功能，使分类框内的右键交互体验与 Windows 桌面原生一致。

## Requirements

### R1: 缩小 Header 标题文字
- **现状**: 标题字体大小为 `scale_for_dpi(18, dpi)` (约 18pt)
- **期望**: 缩小到 `scale_for_dpi(14, dpi)` (约 14pt)
- **约束**: 标题栏高度不变（40px），仅缩小文字

### R2: 快捷方式右键菜单增强（Shell IContextMenu）
- **现状**: 右键点击快捷方式显示自定义简单菜单（仅 "打开"、"打开文件所在位置"、"属性" 3项）
- **期望**: 显示 Windows Shell 原生右键菜单（与桌面右键点击该文件完全一致）
- **技术方案**: 通过 `IShellFolder::GetUIObjectOf` 获取 `IContextMenu`，调用 `QueryContextMenu` 填充菜单，`TrackPopupMenu` 显示，`InvokeCommand` 执行命令

### R3: 空白区域右键菜单（桌面背景菜单）
- **现状**: 右键点击分类框空白区域无反应
- **期望**: 显示桌面背景右键菜单（与桌面空白处右键一致：查看、排序方式、刷新、新建等）
- **技术方案**: 通过 `SHGetDesktopFolder` 获取桌面 `IShellFolder`，调用 `CreateViewObject` 获取 `IContextMenu`

## Acceptance Criteria
- [ ] Header 标题文字明显缩小，视觉更协调
- [ ] 右键点击快捷方式显示完整 Shell 上下文菜单（含打开、编辑、发送到、删除、属性等）
- [ ] 右键点击空白区域显示桌面背景菜单（含查看、排序方式、刷新、新建等）
- [ ] Shell 菜单命令可正常执行（打开文件、打开目录、属性等）
- [ ] 所有修改通过 `cargo fmt` 和 `cargo clippy`

## Technical Notes
- 主要修改文件: `src/desktop/grid_window.rs`
- 需要新增 Windows Shell COM API FFI 声明（IContextMenu、IShellFolder 等方法）
- COM 需要先 CoInitializeEx 初始化（可能已在 app 初始化中完成）
- IContextMenu 使用流程: 获取 PIDL → 获取 IShellFolder → GetUIObjectOf → QueryContextMenu → TrackPopupMenu → InvokeCommand
- 空白区域菜单: SHGetDesktopFolder → CreateViewObject(IContextMenu) → QueryContextMenu → TrackPopupMenu → InvokeCommand
- Cargo.toml 已有 `Win32_UI_Shell` feature，可直接使用
