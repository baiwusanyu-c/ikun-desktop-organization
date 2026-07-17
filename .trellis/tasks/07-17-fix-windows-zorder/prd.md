# brainstorm: 修复 Windows 分类框窗口层级（Z-order）问题

## 目标
修复 Windows 版本中分类框（grid window）的窗口层级不符合预期的问题：当前点击分类框时，它会覆盖其他应用窗口；期望分类框只高于桌面，不高于其他程序窗口，点击时也不会覆盖其他程序。

## 已知信息
* 项目使用 Rust + Win32 API (D2D/DirectWrite) 实现自定义分层窗口 (WS_EX_LAYERED)
* 分类框窗口创建参数: `WS_EX_LAYERED | WS_EX_TOOLWINDOW`, 样式: `WS_POPUP | WS_VISIBLE | WS_THICKFRAME`
* 桌面覆盖窗口 (overlay) 使用 `HWND_BOTTOM` 放在最底层，z-order 正常
* 分类框窗口 (grid_window.rs) 创建时**没有**显式 z-order 控制
* wnd_proc.rs 中**没有**处理 `WM_ACTIVATE`、`WM_WINDOWPOSCHANGING`、`WM_MOUSEACTIVATE` 等 z-order 相关消息
* 菜单窗口 (menu_window.rs) 也没有显式 z-order 控制，但作为弹出菜单可以接受在最上层
* 控制面板窗口 (control_panel_window.rs) 使用 `SetForegroundWindow` 主动获取焦点，行为正确
* 当前仅处理了 `WM_KILLFOCUS`（返回 0），没有任何窗口激活/层级管理逻辑
* 问题复现：程序A覆盖分类框 → 点击分类框露出部分 → 分类框跳到最前覆盖程序A

## 临时假设
* 分类框应该位于"桌面覆盖层之上、普通应用窗口之下"的中间层级
* 菜单窗口（右键菜单）作为弹出菜单可以暂时保持在最上层
* 控制面板窗口不需要修改（用户主动打开，正常窗口行为）

## 待解决问题
* ~~分类框窗口应该放在哪个 z-order 层级？~~ → 已确认：仅高于桌面覆盖层，低于其他应用，分类框之间点击的在最前

## 需求（已确认）
* 分类框窗口始终位于普通应用窗口之下
* 分类框窗口始终位于桌面覆盖层之上
* 点击分类框时不会跳到其他应用窗口之上
* 分类框仍然可以正常接收鼠标事件（点击、拖拽、resize）

## 验收标准（已确认）
- [ ] 打开程序A覆盖分类框，点击分类框露出部分，分类框不覆盖程序A
- [ ] 分类框仍然可见（在桌面上方）
- [ ] 分类框的点击、拖拽、resize 功能正常
- [ ] 右键菜单仍正常弹出并可交互
- [ ] 控制面板窗口行为不受影响
- [ ] 编译无 warning/error

## 完成定义（团队质量门槛）
* 添加/更新测试（单元/集成）
* Lint / typecheck / CI 通过
* 行为变更时更新文档/笔记
* 考虑发布/回滚（如果有风险）

## 明确排除范围
* 不修改 macOS 版本的窗口层级行为
* 不修改控制面板窗口行为
* 不修改桌面覆盖窗口（overlay）的层级行为

## 技术笔记
* 关键文件:
  - `src/platform/windows/grid_window.rs` — 分类框窗口创建 (line 322: CreateWindowExW)
  - `src/platform/windows/wnd_proc.rs` — 分类框窗口消息处理 (867 行)
  - `src/platform/windows/menu_window.rs` — 右键菜单窗口 (弹出式)
  - `src/platform/windows/ffi_types.rs` — Win32 常量定义
  - `src/platform/windows/desktop_overlay/overlay_window.rs` — 桌面覆盖窗口 (已用 HWND_BOTTOM)
* 候选方案:
  1. WM_WINDOWPOSCHANGING 拦截：阻止窗口被提升到最前，将其限制在 HWND_BOTTOM 之上
  2. WM_ACTIVATE 回调：激活后立即 SetWindowPos 到合适层级
  3. WS_EX_NOACTIVATE 扩展样式：阻止窗口获取焦点，但可能影响交互


## 技术方案

### 核心思路
拦截 `WM_WINDOWPOSCHANGING` 消息，当 Windows 试图改变分类框 Z-order 时（如点击激活），
将分类框强制放到 `HWND_BOTTOM`（Z-order 底部），然后通过延迟消息将其他分类框也推到底部。

### Z-order 层级（从上到下）
`
[其他应用窗口]
  ← 分类框永远不会到这里之上
[被点击的分类框]    ← 分类框组最前
[其他分类框]        ← 按创建顺序排列
[桌面覆盖层 overlay] ← HWND_BOTTOM（已在 overlay_window.rs 中设置）
[桌面]
`

### 修改文件
1. `ffi_types.rs` — 添加 `WM_WINDOWPOSCHANGING`(0x0046)、`WINDOWPOS` 结构体、`WM_USER_PUSH_GRIDS_BELOW`(WM_APP+15)
2. `grid_window.rs` — 添加 `push_other_grids_below(active_hwnd)` 公共函数
3. `wnd_proc.rs` — 添加 `WM_WINDOWPOSCHANGING` 和 `WM_USER_PUSH_GRIDS_BELOW` 处理器

### 关键设计
- **PostMessage 延迟执行**：在 `WM_WINDOWPOSCHANGING` 中修改当前窗口 Z-order 后，通过 PostMessage 延迟推送其他分类框。
  避免在 SetWindowPos 嵌套调用中产生竞争条件。
- **遍历顺序**：从 GRID_WINDOWS 列表末尾向前遍历（最旧的先 push），确保最终 Z-order 正确。
- **不影响 resize/move**：仅当 `SWP_NOZORDER` 标志未设置时干预，窗口拖拽和缩放正常工作。