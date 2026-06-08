# Directory Structure

> How backend code is organized in this project.

---

## Overview

本项目是一个 Rust 桌面应用，使用模块化架构设计。每个功能领域都有独立的模块，
平台特定代码通过条件编译隔离。

**核心设计原则**：
- **模块化分离**：每个功能领域独立模块
- **Trait 统一接口**：跨平台功能通过 trait 定义统一接口
- **条件编译隔离**：平台特定代码使用 `#[cfg(...)]` 隔离
- **层次清晰**：UI 层 → 业务层 → 平台层，单向依赖

---

## Directory Layout

```
src/
├── main.rs              # 应用入口，初始化 egui 窗口
├── app.rs               # egui 应用主入口，实现 eframe::App trait
│
├── desktop/             # 桌面系统集成（跨平台）
│   ├── mod.rs           # DesktopManagerTrait trait 定义
│   ├── icons.rs         # DesktopIcon 数据结构
│   ├── windows.rs       # Windows 平台实现 #[cfg(windows)]
│   └── macos.rs         # macOS 平台实现 #[cfg(target_os = "macos")]
│
├── classification/      # 智能分类系统
│   ├── mod.rs           # AppClassifier 入口，组合 database + matcher
│   ├── database.rs      # AppCategoryDB 分类数据库（YAML 加载）
│   └── matcher.rs       # AppMatcher 匹配算法
│
├── ui/                  # egui UI 组件
│   ├── mod.rs           # UI 模块导出
│   ├── box_widget.rs    # ClassificationBoxWidget 分类框组件
│   └── icon_widget.rs   # IconWidget 图标组件
│
├── config/              # 配置管理
│   ├── mod.rs           # Config 模块导出
│   ├── layout.rs        # DesktopLayout 布局持久化（JSON）
│   └── settings.rs      # AppSettings 应用设置
│
└── utils/               # 工具函数
    ├── mod.rs           # Utils 模块导出
    └── platform.rs      # 平台相关工具函数
```

**数据文件目录**：
```
data/
└── categories.yaml      # 分类规则数据库
```

**配置存储位置**（用户目录）：
- macOS/Linux: `~/.config/ikun-desktop-organization/`
- Windows: `%APPDATA%/ikun-desktop-organization/`

---

## Module Organization

### 新功能模块组织规则

**步骤 1：确定模块层级**
- **平台层**：涉及操作系统 API 的功能（放在 `desktop/`）
- **业务层**：纯 Rust 业务逻辑（新建独立模块或放入现有模块）
- **UI 层**：egui 组件（放在 `ui/`）
- **配置层**：持久化配置（放在 `config/`）

**步骤 2：创建模块结构**
每个模块必须包含：
- `mod.rs`：模块入口和导出
- 具体实现文件（按功能拆分）

**示例：添加新的统计功能**

```rust
// src/stats/mod.rs
//! 统计模块
//!
//! 提供桌面图标使用统计功能

mod collector;
mod analyzer;

pub use collector::StatsCollector;
pub use analyzer::StatsAnalyzer;
```

### 平台特定代码规则

**规则 1：使用 Trait 定义统一接口**
```rust
// src/desktop/mod.rs
pub trait DesktopManagerTrait {
    fn new() -> anyhow::Result<Self> where Self: Sized;
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;
}
```

**规则 2：条件编译隔离平台实现**
```rust
// src/desktop/mod.rs
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

// 平台特定的导出
#[cfg(windows)]
pub use windows::DesktopManager;

#[cfg(target_os = "macos")]
pub use macos::DesktopManager;
```

**规则 3：平台实现文件中使用平台 API**
```rust
// src/desktop/windows.rs
#[cfg(windows)]
use windows::Win32::UI::Shell::*;

#[cfg(windows)]
impl DesktopManagerTrait for DesktopManager {
    fn new() -> anyhow::Result<Self> { ... }
}
```

---

## Naming Conventions

### 文件命名

| 类型 | 规则 | 示例 |
|------|------|------|
| 模块入口 | `mod.rs` | `src/desktop/mod.rs` |
| 业务实现 | 功能名称小写 | `matcher.rs`, `database.rs` |
| UI 组件 | `<name>_widget.rs` | `box_widget.rs`, `icon_widget.rs` |
| 平台实现 | 平台名称 | `windows.rs`, `macos.rs` |

### Rust 代码命名

| 类型 | 规则 | 示例 |
|------|------|------|
| Trait | `<Name>Trait` | `DesktopManagerTrait` |
| Struct | PascalCase | `DesktopIcon`, `AppCategory` |
| Function | snake_case | `get_desktop_icons`, `classify` |
| Module | snake_case | `desktop`, `classification` |
| 常量 | UPPER_CASE | `DEFAULT_LAYOUT_VERSION` |

---

## Examples

### 示例 1：模块入口文件

[src/desktop/mod.rs](src/desktop/mod.rs) - 完整的模块入口示例：
- Trait 定义
- 条件编译
- 公开导出

### 示例 2：业务实现模块

[src/classification/mod.rs](src/classification/mod.rs) - 业务模块示例：
- 组合多个子模块
- 提供统一接口
- Default 实现

### 示例 3：平台特定实现

[src/desktop/macos.rs](src/desktop/macos.rs) - 平台实现示例：
- 条件编译标记
- Trait 实现
- 平台 API 使用

---

## Forbidden Patterns

**❌ 不要在平台通用代码中硬编码平台特定逻辑**

```rust
// ❌ 错误：直接在通用代码中判断平台
pub fn get_icons() -> Vec<DesktopIcon> {
    if cfg!(windows) {
        // Windows 代码
    } else {
        // macOS 代码
    }
}

// ✅ 正确：通过 trait 抽象
pub fn get_icons(manager: &dyn DesktopManagerTrait) -> Vec<DesktopIcon> {
    manager.get_desktop_icons()
}
```

**❌ 不要跨层级反向依赖**

```rust
// ❌ 错误：业务层直接依赖 UI
use crate::ui::ClassificationBoxWidget;

// ✅ 正确：UI 层依赖业务层
// 在 ui/ 模块中：use crate::classification::AppClassifier;
```

---

## Common Mistakes

1. **忘记添加 `#[cfg(...)]` 标记** → 编译错误（找不到平台 API）
2. **在 `mod.rs` 中忘记 `pub use`** → 外部无法访问模块内容
3. **模块层级混乱** → UI 层直接调用平台 API，绕过业务层
4. **文件命名不一致** → `desktop_icons.rs` 应改为 `icons.rs`（模块内文件不需要重复模块名）

---

## Quick Checklist for New Module

- [ ] 确定模块层级（platform/business/ui/config）
- [ ] 创建 `mod.rs` 入口文件
- [ ] 定义公开 API（struct/trait/function）
- [ ] 添加必要的 `pub use` 导出
- [ ] 如果跨平台，定义 Trait 并条件编译实现
- [ ] 如果是新功能，更新 `src/main.rs` 的 `mod` 声明