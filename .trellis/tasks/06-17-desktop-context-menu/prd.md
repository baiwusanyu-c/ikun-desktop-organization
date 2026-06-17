# 桌面右键菜单 & 创建分类框对话框

## Goal

在桌面空白区域和分类框区域右键时添加"创建分类框"选项，覆盖初始菜单和"显示更多选项"legacy 菜单，并用独立 egui viewport 窗口实现创建对话框，支持多屏居中定位。

## Requirements

- [x] 注册表方式注入（`setup_desktop_registry`），启动时注册、退出时清理
- [x] 桌面空白区域右键：注册表注入覆盖初始菜单 + "显示更多选项"legacy 菜单
- [x] 格子窗口空白区域右键：QueryContextMenu 自动合并注册表项
- [x] 文件项右键：保持现有 Shell 上下文菜单不变
- [x] 程序退出后注册表清理，桌面右键菜单完全恢复原生
- [x] egui viewport 对话框（immediate 模式，三态状态机管理生命周期）
- [x] 信号文件 IPC（--create-box → 信号文件 → 运行实例检测 → 打开对话框）
- [x] 创建对话框：16 字符必填输入框，确定后创建空分类框
- [x] 多屏居中：Win32 FindWindowW + SetWindowPos 物理像素定位
- [x] X 按钮/ESC/取消/确定 均正确关闭弹窗（三态状态机 Closed/Open/Closing）
- [x] 注册表管理合并到 shell_menu.rs（移除 desktop_hook.rs 模块）
- [x] 全构建 windows_subsystem="windows"，日志写入 ~/.ikun_organizer.log
- [x] 单元测试覆盖信号文件、名称验证、DialogState、PENDING_CREATE_BOX

## Acceptance Criteria

- [x] 程序运行时，桌面空白区域右键显示原生桌面菜单 + "创建分类框"
- [x] 程序运行时，分类框空白区域右键显示桌面菜单 + "创建分类框"
- [x] 程序运行时，分类框内文件项右键显示 Shell 上下文菜单（不变）
- [x] "显示更多选项"内容含"创建分类框"（注册表覆盖 legacy 菜单）
- [x] 程序退出后，桌面右键菜单完全恢复原生行为（无残留）
- [x] 点击"创建分类框"弹出独立 egui viewport 窗口，输入名称后创建分类框
- [x] 弹窗居中对齐到当前鼠标所在显示器（多屏支持）
- [x] X 按钮/ESC/取消/确定 均正确关闭弹窗
- [x] 无控制台窗口闪烁（所有构建均为 GUI 子系统）

## Decision (ADR-lite)

**Context**: 需要同时覆盖初始菜单和"显示更多选项"legacy 菜单，编程式 InsertMenuW 无法注入 legacy 菜单。egui viewport 的 `with_position` 在高 DPI + 多屏下坐标转换有缺陷。

**Decision**:
1. 使用注册表注入（启动 setup / 退出 cleanup），覆盖两种菜单
2. 信号文件 IPC 实现跨进程通信（--create-box → 运行实例）
3. egui viewport immediate 模式渲染对话框
4. Win32 SetWindowPos 绕过 egui 坐标缺陷实现多屏定位
5. 三态状态机管理对话框生命周期（解决 close_requested 永不重置问题）
6. 全构建 windows_subsystem="windows" + 文件日志消除终端闪烁

**Consequences**:
- 注册表 + 生命周期管理确保不运行时无残留
- Win32 定位在多屏 + 任意 DPI 组合下均可靠
- 文件日志（~/.ikun_organizer.log）替代控制台输出
- 代码简洁（desktop_hook.rs 合并到 shell_menu.rs）

## Technical Approach

### 数据流

```
注册表 HKCU\...\Directory\Background\shell\IkunCreateBox
→ 用户右键桌面/分类框空白区域 → 菜单含"创建分类框"
→ 点击 → InvokeCommand → exe --create-box → 写信号文件
→ 运行实例 app.rs 检测信号文件 → open_create_box_dialog()
→ render_create_box_dialog() 每帧渲染 egui viewport (immediate)
→ Win32 FindWindowW("创建分类框") + SetWindowPos 定位到目标显示器
→ 用户输入名称 → 确定 → create_empty_box() → 保存配置
```

### 1. shell_menu.rs — 注册表管理

```rust
pub fn setup_desktop_registry()   // 启动时注册
pub fn cleanup_desktop_registry() // 启动清理残留 + 退出清理
pub fn take_pending_create_box()   // 消费 PENDING_CREATE_BOX 原子标志（fallback 菜单用）
```

### 2. grid_window.rs — 多屏定位

```rust
pub fn get_cursor_monitor_work_area_phys() -> (i32, i32, i32, i32)
    // MonitorFromPoint + GetMonitorInfoW → 物理像素虚拟桌面坐标

pub fn reposition_window_by_title(title: &str, x: i32, y: i32) -> bool
    // FindWindowW + GetWindowRect + SetWindowPos → 可靠定位
```

### 3. app.rs — 三态状态机

```rust
enum DialogOpenState { Closed, Open, Closing }

// Closed → 不渲染 viewport
// Open → 渲染 viewport，监听用户操作
// Closing → 停止渲染 → egui 自动销毁 viewport → 转 Closed
```

### 4. main.rs — 无控制台

```rust
#![windows_subsystem = "windows"]  // 所有构建均无控制台
// --create-box: 写信号文件后立即退出（无 FreeConsole 需要）
```

### 5. logger.rs — 文件日志

日志写入 `~/.ikun_organizer.log`，使用 `env_logger::Target::Pipe` + `chrono` 时间戳。

## Out of Scope

- 拖拽文件到分类框
- 分类框锁定/删除按钮
- 系统托盘菜单修改
- Shell 扩展 DLL
- 对话框 UI 美化（当前使用 egui 默认样式）

## Technical Notes

### 涉及文件变更

| 文件 | 变更 |
|------|------|
| `shell_menu.rs` | 注册表管理（setup/cleanup）+ fallback 菜单 |
| `grid_window.rs` | 多屏检测 + Win32 窗口定位 |
| `app.rs` | 三态状态机 + egui viewport immediate + 信号文件检测 |
| `main.rs` | windows_subsystem + --create-box 信号文件 |
| `logger.rs` | 文件日志输出 |
| `desktop_hook.rs` | 空文件（已合并到 shell_menu.rs） |
| `ffi_imports.rs` | FindWindowW 声明 |
| `ffi_types.rs` | HMONITOR / MONITORINFO / MONITOR_DEFAULTTONEAREST |

### 关键技术决策

1. **为什么用注册表而非编程式注入**：InsertMenuW 只影响 QueryContextMenu 返回的初始菜单，无法覆盖"显示更多选项"legacy 菜单
2. **为什么用 Win32 SetWindowPos**：egui 0.27 的 `with_position` 在高 DPI (200%) 主屏 + 100% 副屏组合下坐标转换错误
3. **为什么用 immediate viewport**：deferred viewport 的 `ViewportCommand::Close` 与 `close_requested()` 交互不可靠
4. **为什么用三态状态机**：`close_requested()` 一旦被设置永不重置，导致死循环；Closing 态通过停止渲染让 egui 自动销毁 viewport
5. **为什么用文件日志**：`windows_subsystem = "windows"` 无控制台，需要替代输出通道

### 架构约束

- `platform/` 负责所有 Win32 API 调用
- `app.rs` 通过 `crate::platform::*` 调用平台函数，不直接 FFI
- 单文件 ≤ 1000 行
- 统一 anyhow::Result
