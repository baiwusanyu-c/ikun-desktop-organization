# Fix control panel startup flash

## 目标
修复启动时控制面板窗口先出现再消失的闪烁问题，使控制面板在启动时默认不显示。

## 需求
- 启动时控制面板窗口不可见
- 不出现窗口闪烁（先显示后隐藏）
- 系统托盘菜单仍可正常打开控制面板

## 验收标准
- [x] 启动时控制面板窗口不显示
- [x] 无窗口闪烁现象
- [x] 托盘菜单打开控制面板功能正常
- [x] cargo fmt 通过
- [x] cargo clippy 无警告

## 技术笔记
- GPUI WindowOptions 有 `show: bool` 字段，默认为 true
- 设置 `show: false` 可使窗口创建时不可见
- 当前代码在 windows_main.rs 中创建窗口后延迟 1 秒调用 hide_control_panel_window
- 修复方案：设置 show: false，移除延迟隐藏逻辑
