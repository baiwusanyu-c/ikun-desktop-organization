# brainstorm: 统一分类框与覆盖区域图标渲染

## 目标
提取公共图标+文本渲染方法，确保分类框（grid cell）和覆盖区域（overlay）的图标渲染、文本、大小完全一致，消除重复代码导致的视觉不一致问题。

## 已知信息
* ce465f02 已将两处文本截断粒度从 WORD 改为 CHARACTER（DWRITE_TRIMMING_GRANULARITY_CHARACTER）
* ender_icon_grid（L635, 分类框）和 ender_drag_overlay_bitmap（L1144, 覆盖区域）都存在独立的图标+文本渲染逻辑
* 两处都独立创建 DirectWrite factory、TextFormat、EllipsisTrimmingSign、TextLayout
* 图标绘制都调用 draw_icon_d2d（已复用），但文本渲染是重复代码
* 字体：Microsoft YaHei UI，大小 scale_for_dpi(13, dpi)，居中对齐
* 阴影：4方向描边，偏移 scale_for_dpi(1, dpi)，颜色 (0, 0, 0, 200)
* 主文字颜色：(250, 250, 255, 255)
* 文本截断：CHARACTER 粒度 + 省略号
* 覆盖区域独立创建 D2D factory + DC render target + DIB section，渲染流程更重

## 临时假设
* 覆盖区域的文本截断"还是存在问题"可能是因为：overlay 的 label_h/total_w 参数与 grid cell 不一致，或 overlay 的 D2D render target 配置差异
* 公共方法应能处理两种场景的参数差异（位置、可用宽度、可用高度），但渲染逻辑完全一致
* 公共方法放在 d2d_render.rs 内部作为 n（非 pub），因为它是渲染引擎内部实现细节

## 待解决问题
* 覆盖区域图标具体存在什么视觉问题？（截断不正确？文字大小不同？渲染模糊？）

## 需求（演进中）
* R1: 提取公共函数 draw_icon_with_label，统一处理图标绘制 + 文本渲染（含截断、阴影）
* R2: ender_icon_grid 调用公共函数替换其图标+文本渲染代码
* R3: ender_drag_overlay_bitmap 调用公共函数替换其图标+文本渲染代码
* R4: 确保两处的字体、字号、截断策略、阴影参数、颜色完全一致

## 验收标准（演进中）
- [ ] 分类框图标和覆盖区域图标视觉完全一致（字体、大小、截断效果、阴影）
- [ ] 无重复的 DirectWrite 文本渲染代码
- [ ] 现有功能不受影响（选中高亮、hover、拖拽插入指示器等）
- [ ] cargo check / clippy 通过

## 完成定义（团队质量门槛）
* 添加/更新测试（单元/集成）
* Lint / typecheck / CI 通过
* 行为变更时更新文档/笔记
* 考虑发布/回滚（如果有风险）

## 明确排除范围
* 不修改 draw_icon_d2d 函数本身（已经正常工作）
* 不修改 icon_extract.rs（图标提取逻辑）
* 不修改分类框的布局计算（grid_layout.rs）
* 不修改覆盖窗口的创建/销毁逻辑

## 技术笔记
* d2d_render.rs 当前 1568 行，两个函数分别位于 L635 和 L1144
* draw_icon_d2d 位于 L935，已作为公共函数复用
* 	arget_inner_create_text_layout 定义在 fi_types.rs:429
* 文本渲染重复代码约 60 行（每处），提取后可减少 ~100 行重复
* 覆盖区域函数签名包含 icon_size, 	otal_w, 	otal_h, gap, label_h, dpi 等参数
* 分类框函数中图标/文本位置由 grid cell 计算得出
