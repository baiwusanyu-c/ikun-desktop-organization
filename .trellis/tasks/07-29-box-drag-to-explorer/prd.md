# 分类框拖拽文件到资源管理器

## 目标
实现从分类框直接拖拽图标到资源管理器（Explorer）的功能，行为与覆盖区域拖拽到资源管理器一致：
- 将桌面原始文件复制到目标资源管理器位置
- 分类框内图标不删除、位置不变（复制操作，非移动）

## 需求
- 从分类框拖拽图标时，当光标悬停在资源管理器窗口上，自动启动 OLE 拖拽
- OLE 拖拽提供 CF_HDROP 格式（文件路径列表），使资源管理器能接收文件
- 拖拽放置后，文件被复制到资源管理器目标位置
- 分类框内图标保持原位（复制操作）
- 拖拽视觉反馈（残影）在 OLE 拖拽启动前正常显示

## 验收标准
- [x] 从分类框拖拽图标到 Explorer 窗口，文件被成功复制到 Explorer 的目标文件夹
- [x] 拖拽完成后分类框内图标保持原位，不被删除
- [x] 拖拽视觉反馈（残影）在拖拽过程中正常显示
- [x] 拖拽到非 Explorer 窗口区域时行为不变（仍然执行原有的框到桌面/覆盖逻辑）
- [x] 同框拖拽、跨框拖拽行为不受影响

## 技术笔记
- 参考覆盖区域拖拽到 Explorer 的实现：overlay_wnd_proc.rs 中的 start_overlay_ole_drag 和 handle_overlay_drag_timer
- 在 drag_handler.rs 的 handle_grid_drag_timer 中添加外部窗口检测逻辑
- 新增 start_grid_ole_drag 函数，在检测到 Explorer 时启动 OLE 拖拽
- 使用 zorder::find_external_window_at_point 检测 Explorer 窗口
- OLE 拖拽使用 ole_dnd::start_box_drag 提供 CF_HDROP + IkunBoxDragData 格式
