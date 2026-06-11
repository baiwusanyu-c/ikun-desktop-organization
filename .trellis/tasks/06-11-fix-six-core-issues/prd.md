# 修复桌面整理工具 6 大核心问题

## Goal
修复当前桌面整理工具中的 6 个关键问题，使应用基本可用。

## Requirements

### 1. 一键分类简化
- 暂时不做智能分类，固定创建 3 个分类框：游戏 / 开发工具 / 办公
- 未匹配游戏和开发工具的图标全部归入"办公"

### 2. 分类框改进
- 提高透明度（alpha 230 → 180）
- 支持边缘拖拽调整大小（WM_NCHITTEST + HTLEFT~HTBOTTOMRIGHT）
- 支持标题栏拖拽移动（WM_NCHITTEST + HTCAPTION）

### 3. 图标不全
- 修复公共桌面路径拼写错误："Deskkktop" → "Desktop"

### 4. 分辨率自适应
- DPI 感知缩放（GetDpiForWindow）
- 图标大小从 32px 提升到 48px（@96DPI）

### 5. 快捷方式交互
- 嵌入 SysListView32 控件替代 GDI 自绘
- 支持双击打开（NM_DBLCLK → ShellExecuteW）
- 支持右键菜单（NM_RCLICK → TrackPopupMenu）
- 支持框选（ListView 原生功能）

### 6. 闪烁 + 图标残留
- ShowWindow(SW_HIDE) 整体隐藏 SysListView32
- 替代逐个 LVM_SETITEMPOSITION 移动图标

## Acceptance Criteria
- [x] cargo build 编译通过
- [x] cargo clippy 无错误
- [ ] 点击"一键分类"：桌面图标瞬间隐藏（无闪烁）
- [ ] 创建固定 3 个分类框（游戏/开发工具/办公）
- [ ] 分类框内图标完整（包含公共桌面快捷方式）
- [ ] 双击格子内图标 → 打开对应应用
- [ ] 右键格子内图标 → 弹出菜单
- [ ] 框选格子内多个图标
- [ ] 拖动分类框标题栏 → 移动
- [ ] 拖动分类框边缘 → 调整大小
- [ ] 点击"清除 & 恢复" → 桌面图标恢复

## Technical Notes
- 核心架构变更：格子窗口从 GDI 自绘改为嵌入 SysListView32 子窗口
- 需要 comctl32.dll 链接（#[link(name = "comctl32")]）
- 图标隐藏从跨进程内存操作简化为 ShowWindow 调用

## Files Modified
- src/desktop/windows.rs (bug fix: path typo)
- src/desktop/icon_hider.rs (rewrite: ShowWindow approach)
- src/desktop/grid_window.rs (major rewrite: SysListView32 embedded)
- src/app.rs (simplify: fixed 3 boxes)
