# fix: 修复从资源管理器拖拽文件到覆盖区域和分类框的 Bug

## 目标
修复从 Windows 资源管理器拖拽文件到 ikun_organizer 覆盖区域和分类框时的多个 bug，
使行为与原生桌面拖拽一致，并遵循分类框/覆盖区域的栅格布局规则。

## 实现方案

### 问题 1: 覆盖区域拖拽 - 图标出现在左上角
**根因**：覆盖窗口（WS_EX_LAYERED）背景 alpha=0 时鼠标事件穿透到桌面。
**修复**：
- 为覆盖窗口注册 OLE IDropTarget
- DragEnter/DragOver 时设置 external_drag_hover = true，背景 alpha 设为 1
- Drop 时将文件复制到桌面，图标 snap 到鼠标释放位置
- DragLeave 时清除悬停状态

### 问题 2: 分类框文件消失
**根因**：handle_ole_drop 对非桌面文件调用 ingest_icon，将文件从原始位置移动到仓库。
**修复**：对非桌面文件先复制到用户桌面，再对桌面副本执行正常 ingest 流程。

### 问题 3: 分类框不显示分割线/位置不对
**根因**：
- drop_target_drag_over 没有调用 handle_cross_box_drag_over 显示分割线/高亮
- handle_ole_drop 没有使用 compute_drop_grid_position 计算正确位置
- 不支持插入模式（分割线位置）
**修复**：
- drop_target_drag_over 调用 handle_cross_box_drag_over（仅在无内部拖拽时）
- handle_ole_drop 使用 compute_drop_grid_position 计算网格坐标
- PendingDrop 新增 insert_col/insert_row 字段，支持插入模式

### 问题 4: 覆盖区域不显示栅格预览/图标不显示
**根因**：
- update_overlay_drag_over_visual 在 drag_item_index == -1 时提前返回
- Drop 后没有强制刷新覆盖窗口
**修复**：
- 移除 drag_item_index < 0 的提前返回，支持外部拖拽
- 修正 item_at_cell 条件：外部拖拽时不排除任何项
- handle_overlay_ole_drop 末尾调用 equest_desktop_overlay_refresh() 强制刷新

### 核心设计原则
**模拟原生桌面拖拽行为**：从资源管理器拖拽文件 = 复制文件到桌面，图标出现在鼠标释放位置。

## 修改文件清单

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| src/platform/windows/ffi_types.rs | 修改 | 添加 WM_USER_OVERLAY_DROP 常量 |
| src/platform/windows/desktop_overlay/overlay_drop_target.rs | 新建 | 覆盖窗口 OLE IDropTarget 实现 |
| src/platform/windows/desktop_overlay/mod.rs | 修改 | 添加 mod overlay_drop_target |
| src/platform/windows/desktop_overlay/overlay_window.rs | 修改 | 添加 drop_target_ptr、external_drag_hover 字段 |
| src/platform/windows/desktop_overlay/overlay_wnd_proc.rs | 修改 | 处理 WM_USER_OVERLAY_DROP、修复 update_overlay_drag_over_visual |
| src/platform/windows/desktop_overlay/overlay_render.rs | 修改 | g_alpha 考虑 external_drag_hover |
| src/platform/windows/drag_drop.rs | 修改 | handle_ole_drop 使用网格布局、支持插入模式 |
| src/platform/windows/drop_target.rs | 修改 | PendingDrop 新增 insert 字段、drag_over 可视化反馈 |
| src/platform/windows/icon_hider.rs | 修改 | 
otify_shell_path 改为 pub(crate) |

## 验收标准
- [x] 从资源管理器拖文件到覆盖区域，图标出现在鼠标释放位置（而非左上角）
- [x] 从资源管理器拖文件到分类框，文件在框中正常显示/打开/右键
- [x] 退出程序后，从资源管理器拖入的文件仍在原始位置（文件不被移动）
- [x] 从桌面拖文件到分类框的行为不受影响（仍然收入仓库）
- [x] 从桌面拖文件到覆盖区域的行为不受影响
- [x] 框→框拖拽、框→覆盖拖拽、框→桌面拖拽不受影响
- [x] 分类框 OLE 拖拽时显示分割线和高亮效果（遵循栅格布局）
- [x] 分类框 OLE drop 后图标出现在正确的网格位置（支持插入模式）
- [x] 覆盖区域 OLE 拖拽时显示 snap-to-grid 预览效果
- [x] 覆盖区域 OLE drop 后图标立即显示（强制刷新）
- [x] 编译通过（0 error, 11 pre-existing warnings）

## 完成定义
* 编译通过 ✓
* 无新增 warning ✓
* 行为与原生桌面拖拽一致 ✓
* 遵循分类框/覆盖区域的栅格布局规则 ✓
* 拖拽过程中有可视化反馈（分割线、高亮、snap-to-grid） ✓

## 技术笔记
- 覆盖窗口使用 WS_EX_LAYERED，alpha=0 时鼠标事件穿透
- OLE IDropTarget 的 DragEnter/DragOver 在 UI 线程执行，可安全操作窗口数据
- GetCursorPos 返回物理像素坐标，GetWindowRect 也返回物理像素
- 分类框的栅格布局通过 calculate_grid 计算，支持列优先/行优先布局
- 覆盖区域的栅格布局通过 overlay_layout_params 计算，使用列优先布局
