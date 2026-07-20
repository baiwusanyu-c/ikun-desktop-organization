# brainstorm: 修复 Window 下图标拖拽问题

## 目标
修复 Windows 平台下从覆盖区域或分类框拖拽图标到分类框时，出现的三个间歇性 Bug：残影覆盖、残影卡死、插入位置错误。

## 已知信息
* 项目是 Rust + Windows API 的桌面图标管理器（ikun_organizer）
* 使用自定义拖拽跟踪方案：`SetCapture` + `WM_MOUSEMOVE` + `WS_EX_LAYERED` 覆盖窗口
* 两套拖拽入口：
  - 分类框（grid_window）：`check_drag_threshold` → `update_drag` → `end_drag`
  - 覆盖窗口（overlay_wnd_proc）：`start_overlay_drag` → `handle_mouse_move` → `end_overlay_drag`
* 拖拽残影由 `drag_image.rs` 的 `OVERLAY_HWND`（WS_EX_LAYERED 窗口）渲染
* 目标检测使用 `zorder::find_grid_window_at_point` 替代 `WindowFromPoint`
* 项添加到目标框时，使用 `data.items.push()` 追加到末尾，`grid_col: -1, grid_row: -1`（自动布局）
* 渲染时自动布局项使用 `(i as i32) % cols` 确定位置，可能与已手动定位的项重叠

### 初步分析的三个 Bug 根因

#### Bug 1: 拖拽图标覆盖目标框图标
- `handle_drag_item_added` 将新项 push 到 `data.items` 末尾
- 如果框内已有手动定位项（`grid_col >= 0`），新项的自动布局位置（基于索引）可能与已有项重叠
- 渲染逻辑中 `cell_x = grid_left + col * cell_w`，两个项可能算出相同坐标

#### Bug 2: 残影卡死不消失
- 两套拖拽入口都使用 `SetCapture(hwnd)` 捕获鼠标
- **缺失 `WM_CAPTURECHANGED` 处理**：Windows 可能因系统对话框、Alt-Tab 等原因剥夺捕获
- 捕获丢失后 `drag_active` 仍为 true，`destroy_drag_overlay()` 不会被调用
- `WM_LBUTTONUP` 可能被发往其他窗口，永远不会触发 `end_drag`

#### Bug 3: 插入位置不对
- `handle_drag_item_added` 只追加到末尾，不计算释放位置
- 自动布局 `col = index % cols` 导致新项位置取决于列表长度
- 例如：框有 A(0,0) B(0,1) 已手动定位，追加 C 得到索引 2，自动布局到 (0,2)
- 但期望是 C 放在释放位置，可能需要插入到特定索引或计算目标 grid 坐标

## 临时假设
* Bug 2 的主因是缺失 WM_CAPTURECHANGED 处理
* Bug 3 的主因是 handle_drag_item_added 不计算释放位置
* Bug 1 可能与 Bug 3 相关：位置计算不正确导致视觉重叠
* 三个 Bug 的"非必现"特性可能与拖拽速度、跨窗口消息时序、系统干扰有关

## 待解决问题
* 用户确认：Bug 3 的期望行为是——拖拽到目标框的什么位置？
  - A) 追加到末尾（但位置正确，不与其他项重叠）
  - B) 插入到释放位置对应的 grid 坐标
  - C) 其他行为

## 需求（演进中）
* 第一步：添加详细诊断日志到拖拽流程关键节点
* 第二步：用户操作复现，收集日志
* 第三步：根据日志确认根因，修复代码

## 验收标准（演进中）
- [ ] 拖拽到分类框时，新图标不与已有图标重叠
- [ ] 拖拽释放后，残影图标正确消失
- [ ] 拖拽到分类框时，图标出现在正确的网格位置
- [ ] 添加的日志不影响正常拖拽性能

## 完成定义（团队质量门槛）
* 手动测试：覆盖区域→分类框、分类框→分类框 各 10 次以上无复现
* Lint / typecheck 通过
* 日志级别合理（debug 级别，不影响 release 性能）

## 明确排除范围
* 不修改 OLE 拖拽路径（当前使用自定义拖拽跟踪方案）
* 不改变分类框的 Z-order 管理逻辑
* 不修改覆盖窗口（desktop_overlay）的图标布局算法

## 技术笔记
* `src/platform/windows/drag_handler.rs` — 分类框拖拽处理（check/update/end_drag）
* `src/platform/windows/drag_image.rs` — 拖拽残影覆盖窗口
* `src/platform/windows/drop_target.rs` — OLE IDropTarget 实现
* `src/platform/windows/ole_dnd.rs` — OLE 拖拽源
* `src/platform/windows/wnd_proc.rs` — 分类框窗口消息处理
* `src/platform/windows/desktop_overlay/overlay_wnd_proc.rs` — 覆盖窗口消息处理
* `src/platform/windows/zorder.rs` — Z-order 管理和窗口查找
* `src/platform/windows/d2d_render.rs` — D2D 渲染引擎
* `src/platform/windows/grid_window.rs` — 分类框数据结构和窗口管理
* `src/box_core/interaction.rs` — 交互状态管理
* 关键消息：WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_CAPTURECHANGED（缺失）
* 关键自定义消息：WM_USER_DRAG_ENTER, WM_USER_DRAG_LEAVE, WM_USER_DRAG_ITEM_ADDED, WM_USER_DRAG_DROP
