# brainstorm: 图标多选与多选拖拽

## 目标
为覆盖区域（桌面图标）和分类框实现完整的多选交互：支持 Shift+点击、Ctrl+点击、鼠标框选三种多选方式，支持跨分类框的 Ctrl+点击追加选中。多选后的图标可作为一个整体进行拖拽，释放行为与单选拖拽一致。

## 已知信息
* 当前选中模型为单选：GridWindowData.selected_item: i32（分类框）和 OverlayWindowData.selected_item: i32（覆盖区域）
* 当前拖拽模型为单项：drag_item_index: i32
* OLE 拖拽数据格式 IkunBoxDragData 已支持多项目（items: Vec<IkunDragItemData>）
* 拖拽残影使用 create_drag_overlay() 创建单个半透明图标窗口（D2D渲染）
* 分类框渲染在 d2d_render.rs，覆盖区域渲染在 overlay_render.rs
* 分类框命中检测：grid_hit_test()；覆盖区域命中检测：overlay_hit_test()
* 分类框消息处理：wnd_proc.rs；覆盖区域消息处理：overlay_wnd_proc.rs
* 拖拽生命周期：drag_handler.rs（阈值检测→拖拽激活→Timer轮询→释放处理）
* 跨框拖拽使用自定义消息（WM_USER_DRAG_ENTER/LEAVE/OVER_UPDATE）

## 已确认决策
* 跨框多选：Ctrl+点击可跨不同分类框追加选中（全局多选集合）
* 拖拽残影：网格排列，最多显示前N个图标+数量角标
* 拖拽到资源管理器：复制操作（CF_HDROP）
* 框选样式：半透明蓝色填充 + 蓝色实线边框

## 需求
1. 分类框和覆盖区域内支持 Shift+点击连续多选
2. 分类框和覆盖区域内支持 Ctrl+点击切换选中
3. 支持 Ctrl+点击跨不同分类框追加选中
4. 分类框和覆盖区域内支持鼠标框选（空白处按下拖动）
5. 多选图标显示选中样式（蓝色半透明背景高亮）
6. 多选图标支持同框内拖拽重排
7. 多选图标支持跨框拖拽移动
8. 多选图标支持拖拽到覆盖区域
9. 多选图标支持拖拽到资源管理器（OLE CF_HDROP 复制）
10. 多选拖拽过程显示网格排列的残影图标+数量角标

## 验收标准
- [ ] Shift+点击可选中连续范围的图标（分类框和覆盖区域）
- [ ] Ctrl+点击可切换单个图标的选中状态（分类框和覆盖区域）
- [ ] Ctrl+点击可跨不同分类框追加选中项
- [ ] 鼠标框选可选中范围内的所有图标（分类框和覆盖区域）
- [ ] 所有多选图标均有可见的选中样式
- [ ] 多选图标拖拽释放后行为与单选拖拽一致
- [ ] 多选拖拽过程中有网格排列的残影显示+数量角标
- [ ] 多选拖拽到资源管理器成功（OLE CF_HDROP 复制）
- [ ] 点击空白处/Escape 清除多选

## 完成定义
* 添加/更新测试
* Lint / typecheck / CI 通过

## 明确排除范围
* 不改变现有单选拖拽的行为
* 不修改 OLE 数据格式定义（已支持多项目）
* 不实现覆盖区域和分类框之间的同时多选（覆盖区域与分类框之间不支持Ctrl跨框追加）

## 技术笔记
* grid_window.rs — GridWindowData（需添加多选状态字段）
* overlay_window.rs — OverlayWindowData（需添加多选状态字段）
* wnd_proc.rs — 分类框消息处理（需添加多选/框选逻辑）
* overlay_wnd_proc.rs — 覆盖区域消息处理（需添加多选/框选逻辑）
* d2d_render.rs — 分类框渲染（需渲染多选样式和框选矩形）
* overlay_render.rs — 覆盖区域渲染（需渲染多选样式和框选矩形）
* drag_handler.rs — 拖拽协调器（需支持多项拖拽）
* drag_drop.rs — 拖拽释放处理（需支持多项释放）
* drag_image.rs — 拖拽残影窗口（需支持多图网格残影）
* ole_dnd.rs — OLE 拖拽源（IkunBoxDragData 已支持多项）
