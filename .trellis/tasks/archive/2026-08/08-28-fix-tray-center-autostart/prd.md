# 修复托盘图标缺失、弹窗居中与开机自启失效

## 目标
修复三个影响日常使用的 bug：家里电脑 cargo run 时托盘不显示 logo 图标；Tauri 弹窗默认不居中；开机自启经常不生效。

## 背景与根因分析

### Bug 1：系统托盘没有 logo 图标
- 托盘图标来自编译时嵌入的 Windows 资源（resource ID=1），由 build/build.rs 生成 .rc 并调用 windres / rc.exe 编译。
- 现有 build.rs 只在 PATH 中查找 windres / rc.exe。MSVC 工具链下 rc.exe 位于 Windows SDK 目录（不在 PATH），导致家里电脑编译时图标未嵌入（tauri-ui feature 下仅 warning 不 panic）。
- 对比：tauri-build 通过 tauri-winres -> embed-resource 查找资源编译器，能通过注册表（KitsRoot10 等）找到 SDK 中的 rc.exe，因此 tauri 自身图标（ID 32512）嵌入成功、整体构建也成功，唯独我们的 ID=1 图标缺失。
- 运行时 app_icon.rs 用 LoadIconW 加载 ID=1 失败后回退系统默认图标。

### Bug 2：Tauri 弹窗不居中
- tauri.conf.json 四个窗口均未设置 center，也没有任何定位代码，窗口位置由 OS 默认级联摆放。

### Bug 3：开机自启经常不生效
- 注册表 Run 值写入的是未加引号的 exe 路径，路径含空格时 Windows 无法正确启动。
- Windows 的 StartupApproved\Run 机制（任务管理器/启动影响评估/安全软件）可能把条目标记为禁用，应用从不修复该标记。
- 启动时 set_auto_start 的错误被 let _ = 静默吞掉，无法排查。
- 缺少开机启动诊断日志，无法区分“Windows 没拉起”还是“拉起后崩溃”。

## 需求

### R1 图标嵌入健壮性
- build.rs 改用 embed-resource crate（与 tauri-build 相同的资源编译器发现机制：PATH + Windows SDK 注册表 + vswhom；GNU 目标自动找 windres）编译 manifest + logo.ico（ID=1）。
- tauri-build 使用 window_icon_path("logo.ico")，并用 new_without_app_manifest() 避免双重 manifest；把 Common Controls v6 依赖合并进 build/app.manifest。
- 嵌入失败仍保持 warning 不中断构建（tauri-ui 模式）。

### R2 运行时图标加载健壮性
- app_icon.rs 改用 LoadImageW 按请求尺寸加载（托盘 24、菜单 16），高 DPI 更清晰。
- 加载回退链：ID=1 -> ID=32512（tauri 嵌入）-> IDI_APPLICATION。
- 保留 diagnose_icon_availability 诊断。

### R3 弹窗主屏幕居中
- tauri.conf.json 四个窗口加 center: true。
- window_cmd.rs 新增主屏幕居中辅助函数；show_managed_window 每次显示前把窗口居中到主屏幕（物理坐标，水平+垂直）。

### R4 开机自启健壮性
- Run 值写入带引号的 exe 路径。
- 开启自启时同时写 HKCU\...\Explorer\StartupApproved\Run 的启用标记（REG_BINARY 12 字节，首字节 0x02）；关闭时删除该标记。
- 所有 set_auto_start 调用点的错误必须记录日志。
- main.rs 启动时记录开机上下文（系统运行时长、exe 路径；运行时长 < 180s 提示“可能由开机自启触发”），便于在家里电脑排查。

## 验收标准
- [ ] cargo check / cargo clippy / cargo test 通过；npm run check / npm run build 通过。
- [ ] 本机构建输出包含图标嵌入成功信息，exe 资源含 ID=1 图标。
- [ ] 四个 Tauri 窗口每次 show 后位于主屏幕水平垂直居中。
- [ ] 注册表 Run 值带引号；开启自启后 StartupApproved\Run\IkunOrganizer 存在且首字节 0x02；关闭后两者清除。
- [ ] 启动日志包含开机上下文行。

## 技术笔记
- embed-resource 3.x 已在依赖图中（tauri-build -> tauri-winres -> embed-resource），新增 build-dependency 不增加额外编译成本。
- embed_resource::compile(rc, NONE) 后需调用 .manifest_optional()（3.x API）。
- StartupApproved 值格式：REG_BINARY，12 字节；0x02=启用，0x03=禁用，其余为时间戳。
- 窗口居中用 app.primary_monitor() 的 position+size 与 window.outer_size() 计算物理坐标。