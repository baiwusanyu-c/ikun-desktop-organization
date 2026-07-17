# 分类框内 Grid 吸附布局 - 拖拽图标松手后吸附到格子

## 目标
在分类框内部实现拖拽重排序功能：用户在分类框内拖拽图标，松手后图标自动吸附到最近的网格格子位置。如果目标格子有图标，则交换两个图标的位置。

## 已知信息（代码库分析）

### 当前架构
- **GridFileItem** 没有 per-item x/y 坐标，纯粹基于索引在 grid 中定位
- **布局计算**：layout_calc::calculate_grid() 根据索引和窗口宽度计算每列/每行的位置
- **渲染**：d2d_render::render_icon_grid() 按索引遍历 items，使用 grid_hit_test() 做命中测试
- **当前拖拽行为**：
  - drag_handler.rs 的 check_drag_threshold() 检测拖拽阈值
  - end_drag() 只处理拖出分类框的场景（拖到其他框或桌面）
  - 同一分类框内拖拽释放 = 取消（无操作）
- **排序/分组**：sort_engine 和 group_engine 会重排 items 向量，改变索引顺序

### 与覆盖区域的区别
| 维度 | 覆盖区域 (overlay) | 分类框 (grid_window) |
|------|-------------------|---------------------|
| 位置存储 | per-item x/y 逻辑像素 | 纯索引，无坐标 |
| 吸附方式 | 修改 x/y 到格子坐标 | 交换 items 向量中的索引 |
| 排序影响 | 无（无排序功能） | 排序会重置自定义顺序 |
| 渲染引擎 | overlay_render.rs | d2d_render.rs |

## 设计方案

### 方案 A：索引交换（推荐）
**核心思路**：拖拽释放时，计算光标所在格子的索引，将拖拽项与目标项在 items 向量中交换位置。

**优点**：
- 不需要修改 GridFileItem 结构（无需 x/y 字段）
- 与现有渲染管线完全兼容（按索引渲染）
- 实现简单，改动最小
- 排序/分组重置顺序是符合预期的行为

**缺点**：
- 不支持将图标拖到空白格子（只能在有图标的格子间交换）

### 方案 B：per-item 坐标
**核心思路**：给 GridFileItem 添加 grid_col / grid_row 字段，类似 overlay 的实现。

**优点**：
- 支持空白格子放置
- 与覆盖区域实现一致

**缺点**：
- 改动大，需要修改渲染、命中测试、排序等所有相关逻辑
- 排序/分组时需要额外处理自定义位置
- 增加复杂度

## 选择：方案 A（索引交换）

## 需求
1. 分类框内拖拽图标，松手后：
   - 若光标在另一个图标所在的格子上 → 交换两个图标位置
   - 若光标在同一格或空白区域 → 取消（保持原位）
2. 拖拽到分类框外 → 保持现有行为（移动到其他框/桌面）
3. 排序/分组操作会重置自定义排列顺序（符合预期）

## 验收标准
- [ ] 分类框内拖拽图标到另一个图标上松手 → 两个图标位置交换
- [ ] 拖拽到分类框外 → 现有行为不变（拖到其他框/桌面）
- [ ] 拖拽到空白区域松手 → 图标回到原位
- [ ] 多 DPI 下拖拽命中测试准确
- [ ] cargo build / cargo test 通过

## 明确排除范围
- 不支持拖拽到空白格子（只在有图标处交换）
- 不实现拖拽过程中的视觉格子高亮
- 不修改排序/分组引擎

## 技术笔记
- src/platform/windows/drag_handler.rs — end_drag() 需要增加同框内交换逻辑
- src/platform/windows/grid_layout.rs — grid_hit_test() 已可用于检测光标下哪个格子
- src/platform/windows/grid_window.rs — GridWindowData.items 向量交换
- src/platform/windows/wnd_proc.rs — WM_LBUTTONUP 调用 end_drag()
- 关键函数链：WM_LBUTTONUP → end_drag() → grid_hit_test() → swap items → 
ender_layered_window()
