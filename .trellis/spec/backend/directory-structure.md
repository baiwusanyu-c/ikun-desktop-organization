# Directory Structure

> 项目代码组织方式（v2.0 架构）

---

## Overview

本项目使用模块化架构设计。每个功能领域都有独立的模块，平台特定代码通过条件编译隔离。

**核心设计原则**：
- **AppContext 上下文注入**：所有模块通过 `AppContext` 共享状态
- **平台抽象**：`platform/` 通过 trait 统一接口，各平台独立实现
- **业务解耦**：`box_core/` 分类框业务逻辑与平台渲染完全分离
- **插件化**：`plugin/` 功能模块作为插件注册
- **条件编译隔离**：平台特定代码使用 `#[cfg(...)]` 隔离
- **层次清晰**：UI → AppContext → 业务/平台，单向依赖

---

## Directory Layout

```
src/
├── main.rs                    # 入口：初始化日志（文件）、DPI 感知、启动 egui
├── app.rs                     # eframe::App 控制面板（已接入 AppContext）
├── context.rs                 # AppContext 核心上下文对象
│
├── platform/                  # 平台抽象层
│   ├── mod.rs                 #   DesktopManagerTrait + re-exports
│   ├── types.rs               #   跨平台类型：DesktopIcon, IconInfo, ScreenInfo
│   ├── windows/
│   │   ├── mod.rs             #   Windows 平台入口
│   │   ├── ffi_types.rs       #   Win32 FFI 类型/常量
│   │   ├── ffi_imports.rs     #   Win32 extern "system" 声明
│   │   ├── desktop_manager.rs #   桌面图标枚举（文件系统读取）
│   │   ├── desktop_overlay/   #   桌面覆盖窗口（渲染未管理图标 + 桌面接管生命周期 + 文件监听刷新）
│   │   ├── d2d_render.rs      #   D2D/DirectWrite 渲染引擎
│   │   ├── grid_window.rs     #   窗口创建/销毁/布局收集
│   │   ├── wnd_proc.rs        #   窗口消息处理 wnd_proc
│   │   ├── menu_window.rs     #   自定义分层窗口深色菜单（D2D/DirectWrite）
│   │   ├── shell_menu.rs      #   Shell 右键菜单 + 注册表管理（创建分类框）
│   │   ├── icon_extract.rs    #   图标提取（SHGetFileInfoW + .lnk 解析）
│   │   ├── icon_hider.rs      #   图标隐藏/恢复（SysListView32）
│   │   └── sys_tray.rs        #   系统托盘（TODO 占位）
│   └── macos/
│       ├── mod.rs             #   macOS 平台入口
│       └── desktop_manager.rs #   桌面图标枚举（返回模拟数据）
│
├── box_core/                  # 分类框核心业务逻辑（平台无关）
│   ├── mod.rs                 #   模块导出
│   ├── box_model.rs           #   ClassificationBox 数据模型 + BoxItem
│   ├── box_manager.rs         #   分类框管理器（CRUD + 持久化）
│   ├── sort_engine.rs         #   排序引擎：8 种维度 × 升降序
│   ├── view_engine.rs         #   视图引擎：8 种视图模式布局参数
│   ├── group_engine.rs        #   分组引擎：按名称/类型/时间等
│   ├── layout_calc.rs         #   网格布局计算 + 命中测试
│   └── interaction.rs         #   交互状态：选中/hover/锁定/拖拽
│
├── classification/            # 分类系统
│   ├── mod.rs                 #   AppClassifier
│   ├── database.rs            #   AppCategoryDB（YAML 兼容 + Default 内嵌规则）
│   ├── matcher.rs             #   AppMatcher 通配符匹配算法
│   └── ai_classifier.rs       #   AI 分类器（TODO：LLM API 调用）
│
├── config/                    # 统一配置系统
│   ├── mod.rs                 #   模块导出
│   └── app_config.rs          #   统一 JSON 配置结构（AppConfig）+ 单元测试
│
├── plugin/                    # 插件系统
│   ├── mod.rs                 #   Plugin trait + PluginRegistry
│   └── builtin/
│       ├── mod.rs             #   register_builtin_plugins()
│       ├── classification_plugin.rs  #   分类功能插件
│       └── style_plugin.rs    #   样式设置插件
│
├── ui/                        # UI 层（egui 组件）
│   ├── mod.rs                 #   模块导出
│   ├── control_panel.rs       #   控制面板主界面
│   ├── settings_panel.rs      #   设置面板（样式/AI/热键/调试）
│   └── tray_menu.rs           #   托盘菜单逻辑
│
├── utils/                     # 工具层
│   ├── mod.rs                 #   模块导出
│   ├── logger.rs              #   日志初始化（文件输出）+ 自定义宏
│   ├── platform.rs            #   跨平台工具（开机自启、配置目录）
│   ├── dpi.rs                 #   DPI 缩放工具
│   └── string_utils.rs        #   UTF-16 字符串转换
│
└── ipc/                       # 进程间通信（TODO 占位）
    └── mod.rs                 #   热键唤醒、单实例检测
```

**非代码目录**：
```
build/
├── build.rs                   # 嵌入 manifest 到 Windows 可执行文件
└── app.manifest               # Per-Monitor DPI V2 感知清单
```

**配置存储位置**（用户主目录）：
- 所有平台: `~/.ikun_organizer.json`

---

## Module Organization Rules

### 新功能模块组织规则

**步骤 1：确定模块层级**

| 层级 | 模块 | 职责 |
|------|------|------|
| **上下文** | `context.rs` | 中心依赖注入，串联所有子系统 |
| **平台层** | `platform/` | 操作系统 API：渲染、窗口、图标、托盘 |
| **业务层** | `box_core/` | 分类框业务逻辑（排序/视图/分组/布局/交互） |
| **分类层** | `classification/` | 分类规则匹配、AI 分类 |
| **配置层** | `config/` | 统一 JSON 配置管理 |
| **插件层** | `plugin/` | 功能插件注册和分发 |
| **UI 层** | `ui/`, `app.rs` | egui 控制面板渲染 |
| **工具层** | `utils/` | 通用辅助函数 |

**步骤 2：创建模块结构**

每个模块必须包含：
- `mod.rs`：模块入口和导出
- 具体实现文件（按功能拆分，单文件 ≤ 1000 行）

### 平台特定代码规则

**规则 1：使用 Trait 定义统一接口**

```rust
// src/platform/mod.rs
pub trait DesktopManagerTrait {
    fn new() -> anyhow::Result<Self> where Self: Sized;
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;
    fn watch_desktop(&self) -> anyhow::Result<()>;
}
```

**规则 2：条件编译隔离平台实现**

```rust
// src/platform/mod.rs
#[cfg(windows)]
pub mod windows;
#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(windows)]
pub use windows::DesktopManager;
#[cfg(target_os = "macos")]
pub use macos::DesktopManager;
```

**规则 3：Windows 平台实现放在 `platform/windows/` 子模块**

```
platform/windows/
├── mod.rs             # 入口 + re-exports
├── ffi_types.rs       # FFI 类型定义
├── ffi_imports.rs     # FFI 函数声明
├── desktop_manager.rs # DesktopManagerTrait 实现
├── d2d_render.rs      # D2D/DirectWrite 渲染
├── grid_window.rs     # 窗口管理
├── wnd_proc.rs        # 窗口消息处理
├── menu_window.rs     # 自定义分层窗口深色菜单（D2D/DirectWrite）
├── shell_menu.rs      # Shell 右键菜单 + 注册表管理
├── icon_extract.rs    # 图标提取
├── icon_hider.rs      # 图标隐藏
└── sys_tray.rs        # 系统托盘
```

---

## Naming Conventions

### 文件命名

| 类型 | 规则 | 示例 |
|------|------|------|
| 模块入口 | `mod.rs` | `platform/mod.rs` |
| 业务实现 | 功能名称小写 | `sort_engine.rs`, `box_model.rs` |
| UI 组件 | 功能名称 | `control_panel.rs`, `settings_panel.rs` |
| 平台实现 | 功能名称 | `desktop_manager.rs`, `d2d_render.rs` |
| FFI 定义 | `ffi_` 前缀 | `ffi_types.rs`, `ffi_imports.rs` |
| 插件 | `_plugin` 后缀 | `classification_plugin.rs` |

### Rust 代码命名

| 类型 | 规则 | 示例 |
|------|------|------|
| Trait | `<Name>Trait` | `DesktopManagerTrait`, `Plugin` |
| Struct | PascalCase | `AppContext`, `ClassificationBox` |
| Function | snake_case | `get_desktop_icons`, `classify` |
| Module | snake_case | `box_core`, `classification` |
| 常量 | UPPER_CASE | `SM_CXSCREEN` |
| 枚举 | PascalCase | `SortMode`, `ViewMode` |

---

## Dependency Rules

**允许的依赖方向**：

```
main.rs → 所有模块
app.rs → context, platform, ui
context.rs → config, platform, classification, (plugin, box_core)
ui/ → context (通过参数传递)
plugin/ → context (通过参数传递)
box_core/ → config (枚举类型)
classification/ → platform (仅 DesktopIcon 类型)
platform/ → (无内部依赖，仅外部 crate)
utils/ → (无内部依赖)
```

**禁止的依赖方向**：

```
❌ platform/ → ui/         (平台层不依赖 UI)
❌ box_core/ → platform/   (业务层不直接调用平台 API)
❌ config/ → context/       (配置层不依赖上下文)
❌ classification/ → ui/   (分类层不依赖 UI)
```

---

## Forbidden Patterns

**❌ 不要在平台通用代码中硬编码平台特定逻辑**

```rust
// ❌ 错误
pub fn get_icons() -> Vec<DesktopIcon> {
    if cfg!(windows) { /* Windows */ } else { /* macOS */ }
}

// ✅ 正确：通过 trait 抽象
pub fn get_icons(manager: &dyn DesktopManagerTrait) -> Vec<DesktopIcon> {
    manager.get_desktop_icons()
}
```

**❌ 不要跨层级反向依赖**

```rust
// ❌ 错误：业务层直接依赖 UI
use crate::ui::control_panel;

// ✅ 正确：UI 层通过 AppContext 与业务交互
```

---

## Quick Checklist for New Module

- [ ] 确定模块层级（platform/box_core/classification/config/plugin/ui/utils）
- [ ] 创建 `mod.rs` 入口文件
- [ ] 定义公开 API（struct/trait/function）
- [ ] 添加必要的 `pub use` 导出
- [ ] 如果跨平台，定义 Trait 并条件编译实现
- [ ] 如果是新功能，更新 `src/main.rs` 的 `mod` 声明
- [ ] 单文件不超过 1000 行
