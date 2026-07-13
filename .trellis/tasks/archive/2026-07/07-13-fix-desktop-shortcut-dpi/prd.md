# 修复桌面快捷方式泄漏和跨显示器DPI缩放

## 目标
修复两个 bug：
1. 运行测试后桌面出现 nonexistent_app.lnk 快捷方式
2. 覆盖窗口图标从高DPI屏幕拖拽到低DPI屏幕时，图标尺寸不随分辨率变化

## 需求

### Bug 1: 测试创建幽灵快捷方式
- 现象：桌面多了一个 nonexistent_app.lnk
- 根因：icon_hider.rs 的 test_restore_icons_handles_rename_failure_gracefully 测试在 temp 目录创建了 user__nonexistent_app.lnk 仓库文件，restore_icons_from_dir 将其还原到用户桌面。如果 temp 目录与桌面在同一卷上，rename 成功但测试只清理了 temp 目录，没有清理桌面上的文件。
- 修复方案：测试结束后清理桌面上可能被创建的文件

### Bug 2: 跨显示器图标DPI不缩放
- 现象：从高分辨率显示器拖拽图标到低分辨率显示器，图标在低分屏上显得很大
- 根因：WM_DPICHANGED 处理器没有更新 data.dpi、data.width、data.height；WM_SIZE 处理器没有更新 data.width、data.height；覆盖窗口跨越整个虚拟桌面，不同显示器有不同DPI，但渲染时只用单一 data.dpi 计算图标大小
- 修复方案：WM_DPICHANGED 时更新 DPI 和窗口尺寸；WM_SIZE 时更新窗口尺寸；渲染图标时根据图标所在位置的显示器 DPI 动态计算图标大小

## 验收标准
- [ ] 运行 cargo test 后桌面不再出现 nonexistent_app.lnk
- [ ] 跨显示器拖拽图标时，图标大小随目标显示器的 DPI 自动调整
- [ ] cargo clippy 无警告
- [ ] cargo test 通过

## 技术笔记
- Rust + Windows API
- 使用 GetDpiForWindow 和 MonitorFromPoint + GetScaleFactorForMonitor 获取显示器 DPI
- 覆盖窗口使用 UpdateLayeredWindow 渲染，D2D 绘制图标
- 图标位置存储在逻辑坐标（96 DPI 基准），渲染时转换为物理坐标
