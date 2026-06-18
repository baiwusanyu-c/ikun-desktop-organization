# 分类框标题栏优化：去图标/数量 + 添加锁定按钮

## Goal
优化分类框标题栏的视觉布局和功能：精简不需要的元素，增加锁定功能。

## Requirements

### R1: 删除名称前的图标
- 分类框标题栏不再显示分类图标
- 名称直接从标题栏左侧开始显示

### R2: 删除图标相关配置项
- 从 AppConfig 中移除分类框图标的配置字段（如 icon、icon_name 等）
- 从 BoxModel / ClassificationBox 中移除图标相关数据字段
- 配置文件加载时忽略旧的图标字段（向后兼容）

### R3: 删除名称后的数量显示
- 标题栏不再显示 "文件夹名称 (N)" 的数量信息
- 只显示纯名称文本

### R4: 添加锁定/解锁按钮
- 在分类框标题栏右侧添加锁定/解锁图标按钮
- 点击按钮切换锁定状态
- 锁定状态持久化到配置文件
- 锁定状态下的分类框：
  - 不允许拖拽移动位置
  - 不允许边缘拖拽修改大小
- 未锁定状态下可正常拖拽和缩放

## Acceptance Criteria
- [ ] 标题栏只显示纯文本名称（无图标、无数量）
- [ ] 配置项中无图标相关字段
- [ ] 锁定按钮正确显示锁定/解锁图标
- [ ] 锁定后无法拖拽移动
- [ ] 锁定后无法边缘缩放
- [ ] 锁定状态持久化（重启后保持）
- [ ] `cargo check` 编译通过
- [ ] `cargo clippy` 无严重警告

## Technical Notes
- 锁定功能在 `box_core/interaction.rs` 中已有 `is_locked` 字段
- 渲染在 `platform/windows/d2d_render.rs`
- 窗口拖拽/缩放在 `platform/windows/grid_window.rs` 的 wnd_proc 中处理
- 配置在 `config/app_config.rs`
