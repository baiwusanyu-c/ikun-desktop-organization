# bugfix: 修改 logo.ico 后应用启动图标未更新

## 目标
修复用户替换项目根目录 logo.ico 后，应用启动时窗口/任务栏/托盘图标仍然显示旧图标的 bug。

## 已知信息
* 项目是 Windows 桌面应用，基于 GPUI 框架 (Rust)
* logo.ico 位于项目根目录，是应用唯一的图标文件
* build/build.rs 通过 .rc 文件将 logo.ico 嵌入到可执行文件资源中 (resource ID=1)
* build.rs 有 cargo:rerun-if-changed=logo.ico 触发增量编译
* src/platform/windows/app_icon.rs 使用 LoadIconW 从嵌入资源 (ID=1) 加载图标用于系统托盘和菜单
* src/platform/windows/grid_window.rs 创建分类框窗口时 WNDCLASSEXW 的 h_icon 和 h_icon_sm 均为 null（未设置窗口图标）
* GPUI 控制面板窗口 (control_panel.rs) 和创建对话框窗口的 WindowOptions 均未设置 icon
* Windows 资源管理器对 .exe 图标有缓存机制，替换图标后可能需要清除缓存才能看到变化

## 临时假设
* 用户已经正确替换了 logo.ico 文件并重新编译了项目
* "启动图标" 可能指：任务栏图标、窗口标题栏图标、系统托盘图标、资源管理器中 .exe 的图标

## 待解决问题
* "启动图标" 具体指哪个位置的图标？（任务栏 / 窗口标题栏 / 系统托盘 / .exe 文件图标）

## 需求（演进中）
* 确保 logo.ico 被正确嵌入到编译后的可执行文件
* 确保所有窗口（控制面板、分类框、对话框）正确显示新图标
* 确保系统托盘图标正确更新

## 验收标准（演进中）
- [ ] 替换 logo.ico 后重新编译，应用启动后图标显示为新图标
- [ ] 任务栏、窗口标题栏、系统托盘的图标均正确更新

## 完成定义（团队质量门槛）
* Lint / typecheck / CI 通过
* 手动测试验证图标更新

## 明确排除范围
* 不涉及动态更换图标（运行时热替换）
* 不涉及 macOS 平台

## 技术笔记
* build/build.rs: 使用 windres 或 rc.exe 编译 .rc 文件，将 manifest 和 icon 嵌入
* src/platform/windows/app_icon.rs: LoadIconW + 资源 ID=1，缓存到 CACHED_ICON
* src/platform/windows/grid_window.rs: WNDCLASSEXW h_icon=null, h_icon_sm=null
* src/gpui_app/control_panel.rs: WindowOptions 未设置 icon
* Windows 图标缓存: 可能需要 InvalidateCache / 重建 icon cache
