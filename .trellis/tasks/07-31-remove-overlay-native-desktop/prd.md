# 移除覆盖区域窗口，使用原生桌面

## 目标

移除桌面覆盖区域窗口（desktop_overlay），让未分类的桌面图标由 Windows 原生 SysListView32 渲染和交互，同时保留分类框的全部功能。核心收益：大幅减少代码量、获得原生桌面交互体验（右键菜单/框选/多选等），同时维持分类框与桌面之间的双向拖拽能力。

## 已知信息

### 当前架构
* 隐藏方式：`ShowWindow(SysListView32, SW_HIDE)` 一刀切隐藏整个列表视图
* 覆盖区域：全屏 `WS_POPUP + WS_EX_LAYERED` 窗口，自己渲染"未分类"图标
* 覆盖区域需自行实现：图标渲染、点击/选中、右键菜单、OLE 拖放、框选/多选、桌面文件监听
* 文件仓库：`C:\Users\<用户名>\.ikun_organizer\managed_icons\`，集中存储已分类文件
* 分类框窗口：`WS_POPUP + WS_EX_LAYERED + WS_EX_TOOLWINDOW`，D2D 渲染 + 液态玻璃

### 关键技术发现
* 文件移走后 SysListView32 自然不显示 → 不需要覆盖区域来重画
* 不隐藏 SysListView32 → 未分类图标由 Windows 原生渲染，拥有完整交互能力
* 分类框与桌面之间的 OLE 拖放是核心交互需求
* 跨盘移动大文件有性能问题（当前方案用 rename + copy fallback）

### 影响模块（从代码库分析）
* `src/platform/windows/desktop_overlay/` — 整个目录可移除（~3000+ 行）
* `src/platform/windows/icon_hider.rs` — hide_icons_in_vault / restore_icons 逻辑调整
* `src/platform/windows/drag_handler.rs` — 拖入/拖出分类框的文件移动逻辑
* `src/platform/windows/drop_target.rs` — 分类框作为 OLE 拖放目标
* `src/platform/windows/grid_window.rs` — 分类框窗口创建
* `src/platform/windows/wnd_proc.rs` — 分类框消息处理
* `src/gpui_app/app_runtime.rs` — 初始化/关闭流程
* `src/config/app_config.rs` — OverlayConfig 配置段

## 决策记录

### 决策 1：分类框拖出到桌面的位置控制
**选择：方案 B — 放到鼠标释放位置**

技术方案：
* 首选 `IFolderView::SelectAndPositionItems`（微软官方 API，稳定）
* 回退 `LVM_SETITEMPOSITION`（发消息给 SysListView32）
* 两者都要求桌面未开启"自动排列图标"

### 决策 2：桌面开启"自动排列图标"时的处理
**选择：方案 C — 提示用户**

检测到自动排列开启时，弹出提示"桌面开启了自动排列，是否关闭以精确放置图标？"
* 用户选择"是" → 关闭自动排列，精确放置
* 用户选择"否" → 静默降级，Windows 自动排到下一个空位
* 通过检测 SysListView32 的 `LVS_AUTOARRANGE` 样式判断

## 临时假设

* 未分类图标留在桌面原位 → Windows 原生显示（不需要覆盖区域）
* 已分类图标从桌面移入仓库 → SysListView32 自然不显示
* 不隐藏 SysListView32 → 未分类图标保持原生交互
* 分类框与桌面之间的双向拖拽是必须功能
* 已有仓库中的图标（从旧版覆盖区域迁移来的）需要正确处理

## 待解决问题

（已全部解决，见决策记录）

## 需求

### 核心需求
1. **移除覆盖区域窗口** — 删除 `desktop_overlay` 模块，不再创建全屏覆盖窗口
2. **不隐藏 SysListView32** — 让 Windows 原生渲染未分类图标，不再调用 `ShowWindow(SW_HIDE)`
3. **保持文件仓库机制** — 已分类图标仍从桌面移入 `managed_icons/` 仓库
4. **分类框所有功能不变** — 渲染、锁定、排序、视图、样式、右键菜单、液态玻璃等
5. **桌面 → 分类框拖拽** — 从原生桌面拖拽图标（单选/多选）到分类框，文件移入仓库
6. **分类框 → 桌面拖拽** — 从分类框拖拽图标（单选/多选）到原生桌面，文件移回桌面并定位到鼠标位置
7. **自动排列检测与提示** — 检测到桌面开启自动排列时提示用户，尊重用户选择

### 由 Windows 原生提供（无需自己实现）
* 桌面图标右键菜单
* 桌面图标框选/多选（含选区矩形）
* 桌面图标拖拽（含 OLE）
* 双击打开文件
* 图标重命名（F2）
* 桌面文件变化自动更新显示

### 保留的现有能力
* 分类框渲染管线（D2D + 液态玻璃）
* 分类框 Z-order 管理（HWND_BOTTOM）
* 分类框锁定/解锁
* 文件仓库（vault）机制
* 配置持久化（BoxLayoutData）
* 控制面板（GPUI）
* 系统托盘
* 全局热键

## 验收标准

- [ ] 覆盖区域窗口不再创建，`desktop_overlay` 模块已从编译中移除
- [ ] `SysListView32` 不再被 `ShowWindow(SW_HIDE)` 隐藏
- [ ] 桌面未分类图标由 Windows 原生渲染，拥有完整交互能力
- [ ] 分类框功能全部正常（渲染、锁定、排序、视图、样式、右键菜单、液态玻璃）
- [ ] 从桌面拖拽单个图标到分类框：图标进入分类框，文件从桌面移入仓库
- [ ] 从桌面拖拽多个图标到分类框：批量进入分类框，文件批量移入仓库
- [ ] 从分类框拖拽单个图标到桌面：图标回到桌面鼠标位置，文件从仓库移回桌面
- [ ] 从分类框拖拽多个图标到桌面：批量回到桌面，图标聚集在鼠标位置附近
- [ ] 桌面开启自动排列时：弹出提示，用户可选择关闭或接受自动排列
- [ ] 启动时正确加载配置并创建分类框（仓库中已有图标正常显示）
- [ ] 退出时正确恢复所有仓库图标到桌面
- [ ] 崩溃恢复：异常退出后仓库中的图标不丢失

## 完成定义（团队质量门槛）
* 添加/更新测试（单元/集成）
* Lint / typecheck / CI 通过
* 行为变更时更新文档/笔记

## 明确排除范围

* **不需要管理员权限** — 所有功能必须在普通用户权限下运行，所有 API 调用与 Explorer.exe 同完整性级别（Medium Integrity）

* 不改变分类框的渲染方式（仍使用 D2D 自绘 + 液态玻璃）
* 不改变分类框的窗口层级管理（WM_WINDOWPOSCHANGING + HWND_BOTTOM）
* 不改变文件仓库的存储位置（仍在 `~/.ikun_organizer/managed_icons/`）
* 不改变分类框的配置文件结构（BoxLayoutData.icon_names 仍记录原始路径）
* 不改变控制面板、系统托盘、全局热键的功能

## 技术笔记

### 关键文件
* `src/platform/windows/icon_hider.rs` — 仓库管理、隐藏/恢复逻辑（需修改：移除 hide_native_icons 调用）
* `src/platform/windows/desktop_overlay/` — 待移除模块（~3000+ 行）
* `src/platform/windows/drag_handler.rs` — 拖拽处理（需增强：支持拖出到桌面定位）
* `src/platform/windows/drop_target.rs` — OLE 拖放目标（已支持桌面→分类框）
* `src/platform/windows/ole_dnd.rs` — OLE 拖放源（需增强：拖出到桌面时移动文件+定位）
* `src/platform/windows/zorder.rs` — Z-order 管理（无需修改）
* `src/platform/windows/mod.rs` — 模块导出（需移除 desktop_overlay 相关导出）
* `src/gpui_app/app_runtime.rs` — 应用生命周期（需移除 overlay 初始化/销毁调用）

### 拖放技术路径
**桌面 → 分类框**：
* 分类框已注册为 IDropTarget（`drop_target.rs`）
* Windows 原生桌面拖出时携带标准 IDataObject（含 CF_HDROP）
* 分类框的 IDropTarget::Drop 解析文件路径 → 移入仓库 → 更新配置

**分类框 → 桌面**：
* 分类框作为拖放源，DoDragDrop 启动
* 桌面 SysListView32 有原生 IDropTarget 接受文件
* 拖放完成后回调：文件从仓库移回桌面
* 使用 IFolderView::SelectAndPositionItems 或 LVM_SETITEMPOSITION 定位到鼠标位置
* 检测自动排列：GetWindowLong(SysListView32, GWL_STYLE) & LVS_AUTOARRANGE

### 风险点
* OLE 拖放到桌面后，文件需要由我们的代码移回（桌面 IDropTarget 只处理 Shell 对象）
* 多选拖拽的 IDataObject 需要正确封装多个文件
* 桌面自动排列开启时的提示 UX 需要设计
* 从旧版（有覆盖区域）迁移到新版（无覆盖区域）需要确保仓库中已有图标正确处理


## 实施记录

### 已完成的修改

| 文件 | 修改内容 |
|------|----------|
| `src/platform/windows/mod.rs` | 移除 `desktop_overlay` 模块声明和导出，新增 `desktop_utils` 模块 |
| `src/platform/mod.rs` | 移除 overlay 相关的 Windows 和 macOS 导出 |
| `src/gpui_app/app_runtime.rs` | 移除 overlay 初始化/销毁、`hide_native_icons`、`restore_native_icons` 调用链；`ensure_native_icons_hidden` 和 `ensure_desktop_overlay_running` 改为空操作 |
| `src/platform/windows/drag_drop.rs` | 移除 `add_icon_to_overlay` 调用，改用 `icon_manager::position_icon_at_screen` |
| `src/platform/windows/drag_handler.rs` | 移除覆盖窗口隐藏/恢复逻辑（OLE 拖拽前） |
| `src/platform/windows/drag_image.rs` | 将 `dpi_at_screen_point` 引用从 `desktop_overlay` 改为 `monitor_utils` |
| `src/platform/windows/menu_window.rs` | 移除 `request_desktop_overlay_refresh` 调用 |
| `src/platform/windows/wnd_proc.rs` | 移除 `request_desktop_overlay_refresh` 调用 |
| `src/platform/windows/icon_hider.rs` | 移除 `has_desktop_overlay` 引用 |
| `src/platform/windows/monitor_utils.rs` | 新增 `dpi_at_screen_point` 函数（从 overlay 模块迁移） |
| `src/platform/windows/desktop_utils.rs` | **新建** — 桌面工具模块（自动排列检测、图标定位） |
| `src/app.rs` | 移除 overlay 相关导入和调用（eframe legacy 路径） |

### 验证结果
- 编译：✅ 无错误
- 测试：✅ 371 passed, 0 failed, 2 ignored

### 注意事项
- `desktop_overlay/` 目录仍保留在文件系统中（未删除），但已从编译中移除（`mod.rs` 中无 `pub mod desktop_overlay`）
- `icon_hider.rs` 中的 `hide_native_icons()` / `restore_native_icons()` / `native_icons_hidden()` 函数仍定义但不再从 `windows/mod.rs` 导出
- `merge_latest_overlay_config()` 保留（处理配置文件中的 overlay 段，与覆盖窗口无关）
- 分类框→桌面拖拽使用 `icon_manager::position_icon_at_screen`，内部自动处理自动排列
