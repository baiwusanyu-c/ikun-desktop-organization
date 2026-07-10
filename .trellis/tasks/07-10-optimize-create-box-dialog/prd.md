# 优化创建分类框弹窗体验

## 目标
对「创建分类框」GPUI 弹窗进行用户体验和样式优化，不影响其他功能。

## 需求
1. 弹窗默认大小改为 600px × 200px（当前 360×180）
2. 弹窗默认出现在主屏幕中央（当前使用鼠标所在显示器坐标，双显示器时可能居中于两屏之间）
3. 按钮和输入框样式使用 gpui-component 主题系统（替代硬编码 rgb() 值），与项目设计风格一致
4. 弹窗支持边缘拖拽改变大小（当前 is_resizable: false）

## 验收标准
- [ ] 弹窗打开时尺寸为 600×200
- [ ] 弹窗居中于主显示器（primary monitor）
- [ ] 弹窗背景色、按钮、输入框使用 gpui-component 的 cx.theme() 主题色
- [ ] 弹窗支持边缘拖拽缩放
- [ ] cargo fmt + cargo clippy 通过
- [ ] 不影响创建分类框的业务逻辑（名称校验、Enter/Escape 快捷键、确认/取消行为）

## 技术笔记
- 文件：src/gpui_app/control_panel.rs（窗口创建参数）、src/gpui_app/create_box_dialog.rs（视图渲染）
- 位置计算：src/gpui_app/app_runtime.rs 中的 calculate_create_box_dialog_position 函数需要同步更新尺寸常量
- 主题：gpui-component 提供 ActiveTheme trait 和 ThemeColor 结构，通过 cx.theme() 访问
- 缩放：WindowOptions 的 is_resizable 改为 true，并设置 min_size
