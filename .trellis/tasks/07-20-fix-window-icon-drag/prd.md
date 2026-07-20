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

## Bug Analysis: Window 下图标拖拽系统修复

### 1. Root Cause Category

| 问题 | 类别 | 描述 |
|------|------|------|
| **残影位置偏移** | **E. 隐式假设** | 假设窗口原点=图标绘制位置，但实际图标在窗口内居中绘制，产生 `(cell_w - icon_size)/2` 的隐式偏移 |
| **残影卡死不消失** | **B. 跨层契约失败** | 桌面文件监听器刷新时重置 `drag_active` 状态，但拖拽系统不知道状态被外部重置 |
| **拖拽跟随掉帧** | **C. 变更传播失败** | 从 SetCapture 改为 Timer 时，Timer 间隔(16ms)和 DPI 检测开销未同步优化 |
| **插入位置错误** | **A. 缺失规范** | 没有定义"拖拽释放时应该放在哪个位置"，默认追加到列表末尾 |
| **图标重叠** | **D. 测试覆盖缺口** | 添加新项时没有检测目标位置是否已被占用 |

### 2. Why Fixes Failed (修复失败分析)

**残影偏移问题：**
1. **第一次失败**：移除固定的 `+8px` 偏移后仍有偏移
   - **原因**：只看到了表面偏移，没发现图标在窗口内居中绘制的隐式偏移
   - **思维模型错误**：以为"窗口位置=图标位置"，实际是"窗口位置+居中偏移=图标位置"

2. **第二次失败**：尝试传递点击坐标计算偏移
   - **原因**：坐标系混乱（屏幕坐标 vs 客户区坐标 vs 逻辑像素 vs 物理像素）
   - **过度设计**：引入了复杂的偏移计算逻辑，但没解决根本问题

3. **最终成功**：在 `update_drag_overlay` 中减去居中偏移量
   - **为什么成功**：直接在渲染层补偿偏移，简单直接

**残影卡死问题：**
1. **第一次失败**：在 `WM_CAPTURECHANGED` 中重新 SetCapture
   - **原因**：WS_EX_LAYERED 窗口上 SetCapture 会无限触发 WM_CAPTURECHANGED
   - **工具限制**：不知道 layered 窗口的这个限制

2. **第二次失败**：移除 SetCapture 依赖 WM_MOUSEMOVE
   - **原因**：layered 窗口在 Z-order 底层收不到鼠标消息
   - **思维模型错误**：以为"移除限制=问题解决"，实际是"失去了唯一的消息来源"

3. **第三次失败**：用 Timer 轮询
   - **原因**：Timer 正常工作，但桌面监听器刷新时重置了 `drag_active`
   - **不完整范围**：修复了 Timer 逻辑，但没考虑其他模块会修改拖拽状态

4. **最终成功**：Timer 检测到 `drag_active=false` 时强制清理残影 + 刷新时跳过拖拽中状态
   - **为什么成功**：双保险——防止状态被重置 + 即使被重置也能正确清理

### 3. Prevention Mechanisms (预防机制)

| 优先级 | 机制 | 具体行动 | 状态 |
|--------|------|----------|------|
| **P0** | **文档** | 在 `windows-layered-window-guide.md` 中添加"SetCapture 限制"和"Timer 轮询模式"章节 | DONE |
| **P0** | **文档** | 在 `cross-layer-thinking-guide.md` 中添加"拖拽状态跨层失败"案例 | DONE |
| **P0** | **架构** | 创建 `DragStateManager`，集中管理拖拽状态，防止外部直接修改 `drag_active` | TODO |
| **P1** | **编译时** | 给 `drag_active` 字段添加 setter 方法，记录所有修改的调用栈 | TODO |
| **P1** | **测试** | 添加拖拽状态一致性测试：确保 Timer 和 drag_active 状态同步 | TODO |
| **P2** | **代码审查** | 添加 checklist："修改 layered 窗口行为前是否检查了 Z-order 和消息路由？" | TODO |
| **P2** | **运行时** | 添加拖拽状态监控日志，检测异常状态转换 | DONE（本次添加的 DIAG 日志） |

### 4. Systematic Expansion (系统性扩展)

**相似问题：**
- 覆盖窗口的 hover 状态管理：是否也有类似的"状态被外部重置"问题？
- 分类框的缩放状态：`is_resizing` 是否也可能被意外重置？
- 菜单打开状态：`menu_open` 是否会被其他模块修改？

**设计缺陷：**
- **状态分散**：`drag_active`、`drag_pending`、`drag_item_index` 分散在多个地方，缺乏统一管理
- **缺乏状态机**：拖拽流程没有明确的状态机定义，依赖隐式条件判断
- **跨模块耦合**：桌面监听器直接修改 overlay 数据，违反了单一职责原则

**流程改进：**
- **拖拽功能开发规范**：编写"拖拽系统开发指南"，定义状态机、消息流、状态修改权限
- **Layered 窗口开发规范**：扩展现有指南，添加 SetCapture、Timer、Z-order 的最佳实践

**知识缺口：**
- 团队对 WS_EX_LAYERED 窗口的消息路由机制理解不足
- 对 Timer vs SetCapture 的适用场景缺乏清晰判断标准

### 5. Knowledge Capture (知识捕获)

**已更新的文档：**
- [x] `.trellis/spec/guides/windows-layered-window-guide.md` - 添加"Drag and Drop on Layered Windows"章节
- [x] `.trellis/spec/guides/cross-layer-thinking-guide.md` - 添加"拖拽状态跨层失败"案例研究
- [x] `.trellis/tasks/07-20-fix-window-icon-drag/prd.md` - 记录完整修复过程和技术决策

**待创建的文档：**
- [ ] `.trellis/spec/guides/drag-system-guide.md` - 拖拽系统开发指南（状态机、消息流、最佳实践）
- [ ] `.trellis/spec/guides/timer-vs-setcapture-guide.md` - Timer 和 SetCapture 的选择指南

**架构改进提案：**
- [ ] 创建 `DragStateManager` 模块，集中管理拖拽状态
- [ ] 定义拖拽状态机：`Idle -> Pending -> Active -> Ending -> Idle`
- [ ] 添加状态转换验证：防止非法状态转换
- [ ] 添加状态监控：记录所有状态变更的调用栈

---

## 核心教训

> **调试的价值不在于修复 bug，而在于让这类 bug 永远不再发生。**

### 三个层次的洞察

1. **战术层**：如何修复这个 bug
   - 减去图标居中偏移量
   - Timer 检测状态一致性并强制清理
   - 刷新时跳过拖拽中状态

2. **战略层**：如何预防这类 bug
   - 文档化 WS_EX_LAYERED 窗口的特殊行为
   - 集中管理拖拽状态，防止外部修改
   - 添加状态一致性检测和自动恢复

3. **哲学层**：如何扩展思维模式
   - **隐式假设检测**：总是问"我假设了什么？这个假设总是成立吗？"
   - **跨层依赖分析**：修改一个模块时，问"哪些其他模块依赖这个状态？"
   - **失败模式思考**：设计系统时，问"如果这个操作失败会怎样？系统能恢复吗？"

### 30 分钟分析节省 30 小时调试

本次 bug 修复过程中，如果没有系统性的 break-loop 分析：
- 下次遇到 layered 窗口问题，会再次尝试 SetCapture → 再次失败
- 下次遇到状态被外部重置，会再次困惑"为什么状态变了？"
- 下次遇到坐标偏移，会再次尝试复杂的坐标转换 → 再次失败

通过文档化和知识捕获：
- 团队成员可以直接查阅指南，避免重复踩坑
- 新成员可以快速了解 layered 窗口的特殊行为
- 未来的代码审查可以基于文档中的 checklist

**结论**：调试的真正产出不是修复的代码，而是更新的文档和扩展的知识库。
