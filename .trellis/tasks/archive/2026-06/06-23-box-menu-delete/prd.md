# 实现分类框更多菜单的删除功能

## Goal
实现分类框 ⋮ "更多"菜单中的"删除"功能，使用户可以删除（销毁）一个分类框窗口。

## Requirements
- 点击菜单中的"删除"项后，分类框窗口被销毁
- 分类框从全局 GRID_WINDOWS 列表中移除
- 配置文件 (~/.ikun_organizer.json) 中的 layout 同步更新（移除已删除的分类框）
- 删除操作的资源清理正确（图标句柄、D2D/DW 资源通过 WM_DESTROY 释放）

## Acceptance Criteria
- [ ] 点击"删除"后分类框窗口消失
- [ ] 控制面板显示格子数减少
- [ ] 配置文件中不再包含已删除的分类框
- [ ] 不影响其他分类框
- [ ] 编译通过 (cargo check)
- [ ] 测试通过 (cargo test)

## Technical Notes
- `handle_menu_command` 中 CMD_DELETE 当前只打印日志（预留）
- `show_menu_window` 在菜单关闭后调用 `handle_menu_command`，需要判断是否为删除操作以跳过 `render_layered_window`
- 需要新增 `destroy_grid_window(hwnd)` 公共函数来安全销毁单个窗口
- 配置持久化通过直接 load → 移除匹配项 → save 实现
