# 修复分类框删除后图标消失bug

## 目标
修复当分类框被删除时，里面的图标消失而不是返回主屏幕覆盖区域的问题。同时将覆盖区域的自动布局改为纵向优先（列优先）。

## 需求
- 删除分类框后，框内图标应返回桌面覆盖窗口显示
- 返回覆盖窗口的图标应以纵向优先（列优先）布局排列
- 已有自定义位置的图标保持不变

## 验收标准
- [ ] 删除分类框后，其包含的图标出现在桌面覆盖窗口上
- [ ] 无保存位置的图标按列优先方式自动排列（先填满一列再换下一列）
- [ ] 已有自定义位置的图标不受影响
- [ ] cargo build 通过
- [ ] cargo test 通过

## 技术笔记
- 根本原因：wnd_proc.rs 中 WM_USER_DELETE_BOX 处理函数删除分类框后未调用 request_desktop_overlay_refresh()
- 布局问题：overlay_window.rs 中 resolved_icon_position、overlay_icon_cell_x、overlay_icon_cell_y 使用行优先布局（index % cols = col），需改为列优先
