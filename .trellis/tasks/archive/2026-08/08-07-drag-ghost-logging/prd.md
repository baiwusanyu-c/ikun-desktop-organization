# 添加拖拽残影调试日志

## 目标
在拖拽相关模块添加详细的调试日志，帮助定位"从桌面拖拽图标到分类框时残影图标卡死"的 bug。

## 需求
- 在拖拽生命周期关键节点添加日志
- 特别关注 IDropTargetHelper 的 Show/DragEnter/DragLeave 调用
- 记录 overlay 窗口的创建和销毁
- 记录 end_drag 的所有退出路径

## 验收标准
- [ ] 编译通过
- [ ] 日志足够详细，能定位残影卡死问题

## 技术笔记
- 残影来源：自定义 overlay 窗口 + Shell IDropTargetHelper
- 关键文件：drag_handler.rs, drop_target.rs, drag_image.rs

