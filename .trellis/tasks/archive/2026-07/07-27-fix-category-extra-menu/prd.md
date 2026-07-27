# brainstorm: 修复分类框右键菜单关闭后出现额外菜单的问题

## 目标
修复在分类框中右键图标打开上下文菜单后，左键点击其他区域关闭菜单时出现一个不期望的额外菜单（深色主题自定义菜单）的 bug。

## 已知信息
* Bug 复现步骤：分类框中 -> 右键图标 -> Shell 上下文菜单出现 -> 左键点击分类框其他区域 -> 上下文菜单关闭 -> 出现额外深色主题菜单
* 额外菜单样式为深色主题，来自 menu_window.rs 的自定义分层窗口菜单 (show_menu_window)
* 右键菜单通过 TrackPopupMenu (阻塞式模态循环) 显示，在 wnd_proc.rs WM_RBUTTONUP 中触发
* show_menu_window() 被两处调用: (1) WM_LBUTTONDOWN 中点击更多按钮时 (line 676) (2) WM_USER_OPEN_MENU 消息处理 (line 1048-1049)
* show_menu_window 在发现已有菜单打开时，会 PostMessageW WM_USER_OPEN_MENU 延迟打开新菜单
* 分类框为 WS_EX_LAYERED 分层窗口，使用 D2D 渲染
* 还有桌面覆盖窗口 (overlay) 也有独立的右键菜单处理逻辑

## 临时假设
* 假设 A: TrackPopupMenu 模态循环关闭菜单后，左键点击消息 (WM_LBUTTONDOWN) 泄漏到父窗口，如果点击位置恰好是更多按钮区域，则触发 show_menu_window
* 假设 B: TrackPopupMenu 返回后，Windows 将 dismiss 点击重新发送给 owner 窗口，导致额外的 WM_LBUTTONDOWN 被处理
* 假设 C: 存在某个 PostMessage/状态标志未及时清理，导致菜单在 TrackPopupMenu 返回后被意外打开

## 待解决问题
* 额外菜单是否总是出现? 还是只在特定点击位置出现?
* 额外菜单的内容是什么? (排序/视图选项? 还是桌面上下文菜单?)

## 需求 (演进中)
* 关闭右键上下文菜单后，不应出现任何额外菜单
* 右键菜单的关闭行为保持正常 (左键点击其他区域关闭)
* 更多按钮的左键点击功能不受影响

## 验收标准 (演进中)
- [ ] 右键图标 -> 出现 Shell 上下文菜单
- [ ] 左键点击分类框其他区域 -> 上下文菜单关闭，无额外菜单出现
- [ ] 左键点击更多按钮 -> 正常显示自定义菜单
- [ ] 右键分类框空白区域 -> 正常显示桌面上下文菜单
- [ ] 所有菜单操作功能正常 (选择、取消、子菜单)

## 完成定义 (团队质量门槛)
* Lint / typecheck / CI 通过
* 行为变更时更新文档/笔记

## 明确排除范围
* 不修改右键菜单的内容/样式
* 不修改更多菜单的功能

## 技术笔记
* src/platform/windows/wnd_proc.rs -- 分类框窗口过程 (WM_RBUTTONUP: line 829, WM_LBUTTONDOWN: line 620)
* src/platform/windows/shell_menu.rs -- Shell 上下文菜单实现 (show_shell_context_menu: line 78, TrackPopupMenu: line 208)
* src/platform/windows/menu_window.rs -- 自定义深色主题菜单窗口 (show_menu_window: line 63)
* src/platform/windows/grid_window.rs -- 分类框窗口创建和数据结构
* src/platform/windows/grid_layout.rs -- 布局和更多按钮位置计算
* src/platform/windows/desktop_overlay/overlay_wnd_proc.rs -- 覆盖窗口事件处理