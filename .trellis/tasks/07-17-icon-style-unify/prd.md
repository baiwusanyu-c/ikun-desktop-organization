# brainstorm: 统一图标选中/hover背景色样式与分类框滚动条

## 目标
统一桌面覆盖窗口中"覆盖区域图标"和"分类框内图标"的 hover/选中背景色样式，使两者视觉一致，并为分类框图标区域添加 Y 轴滚动支持。

## 已知信息

### 覆盖区域图标 (overlay_render.rs)
* hover 背景色: `d2d_color(255, 255, 255, 34)` — 白色半透明
* 选中背景色: `d2d_color(51, 153, 255, 90)` — 蓝色半透明
* 圆角: `radiusX/Y: scale_for_dpi(6, icon_dpi)` — 6px DPI 自适应
* 填充方式: `FillRoundedRectangle`
* 位置文件: `src/platform/windows/desktop_overlay/overlay_render.rs` (lines 238-290)

### 分类框内图标 (d2d_render.rs, render_icon_grid)
* hover 背景色: `d2d_color(70, 90, 130, 70)` — 暗蓝灰色半透明
* 选中背景色: `d2d_color(60, 80, 120, 120)` — 深蓝灰色半透明
* 圆角: **无** — 使用 `FillRectangle`（直角矩形）
* 位置文件: `src/platform/windows/d2d_render.rs` (lines 680-722)

### 分类框布局
* 布局计算: `src/box_core/layout_calc.rs` + `src/platform/windows/grid_layout.rs`
* 交互状态: `src/box_core/interaction.rs` (selected_item, hovered_item)
* 消息处理: `src/platform/windows/wnd_proc.rs`

### 滚动支持
* 当前覆盖区域和分类框**均无**滚动实现
* 无 `scroll_offset` / `scroll_y` 字段

## 临时假设
* 用户主要在 Windows 平台使用
* "分类框"指桌面覆盖窗口中的分类框（D2D 渲染），非 GPUI 控制面板
* 滚动条样式应与整体 UI 风格一致（半透明、低调）

## 待解决问题
* 滚动条的视觉样式偏好？（细窄滚动条 vs 标准宽度？hover 时才显示 vs 始终显示？）
* 鼠标滚轮滚动是否也需要支持？

## 需求（演进中）
1. 覆盖区域图标使用分类框图标的 hover/选中颜色（深色方案）
2. 分类框图标 hover/选中背景使用圆角（与覆盖区域一致，6px DPI 自适应）
3. 分类框图标 Y 轴超出时显示滚动条

## 验收标准（演进中）
- [ ] 覆盖区域图标 hover 色变为 `d2d_color(70, 90, 130, 70)`
- [ ] 覆盖区域图标选中色变为 `d2d_color(60, 80, 120, 120)`
- [ ] 分类框图标 hover/选中背景使用 `FillRoundedRectangle`，radius 6px DPI 自适应
- [ ] 分类框图标超出 Y 轴时有滚动条
- [ ] 编译通过，无 warning

## 完成定义（团队质量门槛）
* Lint / typecheck / CI 通过 (cargo build)
* 行为变更时更新文档/笔记

## 明确排除范围
* 不修改 GPUI 控制面板样式
* 不修改 macOS 渲染
* 不实现 X 轴滚动

## 技术笔记
* overlay_render.rs: 覆盖区域图标渲染（D2D + 圆角矩形）
* d2d_render.rs: 分类框渲染（包含 render_icon_grid 函数）
* grid_layout.rs: 分类框布局计算（calculate_grid）
* interaction.rs: 交互状态管理
* wnd_proc.rs: 窗口消息处理（需要添加 WM_MOUSEWHEEL 处理）
