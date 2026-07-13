# Quality Guidelines

> Code quality standards for backend development.

---

## Overview

本项目遵循 Rust 最佳实践，注重代码质量、可维护性和性能。

**核心原则**：
- **安全优先**：避免 `unsafe` 代码，除非绝对必要
- **错误处理**：使用 `anyhow::Result`，不使用 `panic/unwrap/expect`
- **代码复用**：通过 Trait 抽象共性，避免重复
- **性能考虑**：避免不必要的克隆、高频路径优化
- **文档完整**：所有公开 API 必须有文档注释

---

## Linting and Formatting

### Rust Formatter

**使用 `cargo fmt` 自动格式化**：

```bash
# 格式化代码
cargo fmt

# 检查格式（不修改）
cargo fmt --check
```

**规则**：
- 所有代码必须通过 `cargo fmt` 格式化
- 提交前必须运行 `cargo fmt`

### Rust Linter

**使用 `cargo clippy` 检查代码质量**：

```bash
# 运行 clippy
cargo clippy

# 严格模式（警告视为错误）
cargo clippy -- -D warnings
```

**规则**：
- 所有代码必须通过 `cargo clippy` 检查
- Clippy 警告必须在提交前修复

---

## Forbidden Patterns

### 1. 禁止使用 `unwrap()` / `expect()` 处理可恢复错误

```rust
// ❌ 错误：unwrap 可能 panic
let layout = DesktopLayout::load().unwrap();

// ❌ 错误：expect 可能 panic
let layout = DesktopLayout::load().expect("必须加载");

// ✅ 正确：使用 unwrap_or_else 优雅降级
let layout = DesktopLayout::load()
    .unwrap_or_else(|e| {
        log::warn!("加载失败: {}", e);
        DesktopLayout::default()
    });
```

**例外**：测试代码可以使用 `unwrap()`

### 2. 禁止硬编码配置路径

```rust
// ❌ 错误：硬编码路径
let path = "/Users/username/.config/...";

// ✅ 正确：使用 dirs crate
let path = dirs::home_dir()
    .map(|p| p.join(".ikun_organizer.json"))
    .ok_or_else(|| anyhow::anyhow!("无法获取用户主目录"))?;
```

### 3. 禁止跨层级反向依赖

```rust
// ❌ 错误：业务层依赖 UI 层
use crate::ui::ClassificationBoxWidget;

// ❌ 错误：平台层依赖业务层
use crate::classification::AppClassifier;

// ✅ 正确：单向依赖
// UI → 业务 → 平台
// 业务层调用平台层 Trait
```

### 4. 禁止不必要的克隆（性能问题）

```rust
// ❌ 错误：不必要的克隆
fn get_categories(&self) -> Vec<AppCategory> {
    self.db.categories.clone()  // 每次调用都克隆
}

// ✅ 正确：返回引用或迭代器
fn get_categories(&self) -> &[AppCategory] {
    &self.db.categories
}
```

**例外**：必要时（如跨线程、所有权转移）可以克隆

### 5. 禁止高频路径日志（性能问题）

```rust
// ❌ 错误：每帧都记录
fn update(&mut self) {
    log::debug!("更新帧");  // 每秒 60 次
}

// ✅ 正确：只在重要事件时记录
fn update(&mut self) {
    if self.changed {
        log::info!("状态已更新");
    }
}
```

### 6. 禁止手动定义 COM vtable 和混合 FFI 类型

```rust
// ❌ 错误：手动定义 COM vtable（ABI 不匹配风险）
#[repr(C)]
struct IContextMenuVtbl {
    QueryInterface: unsafe extern "system" fn(...) -> HRESULT,
    AddRef: unsafe extern "system" fn(...) -> u32,
    // ...
}
let raw = cm.as_raw() as *mut MyVtable;
((*(*raw).lpVtbl).QueryContextMenu)(...);  // 💥 STATUS_ACCESS_VIOLATION

// ✅ 正确：使用 windows crate 的安全方法
cm.QueryContextMenu(hmenu, 0, first, last, flags)?;
cm.InvokeCommand(&invoke_info)?;
```

**原因**：`windows` crate 自动生成的 vtable 经过验证，手动定义容易出现参数类型不匹配和对齐问题。

→ 详见 [Windows Shell API Guide](../guides/windows-shell-api-guide.md)

### 7. 禁止混合手动 FFI 和 windows crate 类型

```rust
// ❌ 错误：手动结构体传给 crate 方法
let info: MyCMINVOKECOMMANDINFO = ...;
cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO);

// ✅ 正确：全部使用 windows crate 类型
let info: windows::Win32::UI::Shell::CMINVOKECOMMANDINFO = ...;
cm.InvokeCommand(&info)?;
```

---

## Required Patterns

### 1. 所有公开 API 必须有文档注释

```rust
// ✅ 正确：完整的文档注释
/// 应用分类数据库
///
/// 从 YAML 文件加载应用分类数据，提供分类规则匹配
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppCategoryDB { ... }

/// 加载分类数据库
///
/// # Returns
/// - `Ok(AppCategoryDB)` - 加载成功
/// - `Err(anyhow::Error)` - 加载失败
pub fn load() -> anyhow::Result<Self> { ... }
```

### 2. 所有模块必须有模块文档

```rust
// ✅ 正确：模块文档
//! 桌面管理模块
//!
//! 负责与平台桌面系统集成，读取桌面图标信息

mod icons;
```

### 3. 所有数据结构必须实现 `Default`

```rust
// ✅ 正确：Default 实现
impl Default for DesktopLayout {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            last_modified: chrono::Utc::now(),
            boxes: Vec::new(),
        }
    }
}
```

### 4. 错误处理必须使用 `.context()`

```rust
// ✅ 正确：添加错误上下文
use anyhow::Context;

let content = std::fs::read_to_string(&path)
    .context(format!("无法读取文件: {}", path.display()))?;
```

### 5. 平台特定代码必须使用条件编译

```rust
// ✅ 正确：条件编译
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
mod macos;
```

---

## Testing Requirements

### 测试命令

```bash
# 运行所有测试
cargo test

# 运行特定测试
cargo test <test_name>

# 显示测试输出
cargo test -- --nocapture
```

### 测试要求

| 类型 | 要求 |
|------|------|
| **单元测试** | 核心业务逻辑必须有单元测试 |
| **集成测试** | 关键流程必须有集成测试 |
| **文档测试** | 文档中的代码示例必须可运行 |

### 测试模式

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_game_icon() {
        let db = AppCategoryDB::default();
        let matcher = AppMatcher::new(db);
        
        let icon = DesktopIcon {
            name: "steam.exe".to_string(),
            ...
        };
        
        let result = matcher.classify(&icon);
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "games");
    }
}
```

---

## Code Review Checklist

### 提交前必须检查

- [ ] **格式化**：`cargo fmt` 已运行
- [ ] **Lint**：`cargo clippy` 无警告
- [ ] **测试**：`cargo test` 全部通过
- [ ] **文档**：新 API 有文档注释
- [ ] **错误处理**：无 `unwrap/expect`（测试除外）
- [ ] **日志**：关键操作有日志记录
- [ ] **性能**：无高频路径日志、无不必要克隆

### Reviewer 检查项

- [ ] 代码符合项目架构（模块分层、trait 抽象）
- [ ] 错误处理遵循规范（anyhow::Result + .context()）
- [ ] 平台特定代码使用条件编译
- [ ] 无反向依赖（UI → 业务 → 平台）
- [ ] 文档完整且准确
- [ ] 性能考虑（避免克隆、高频日志）

---

## Performance Guidelines

### 性能优化规则

**规则 1：避免不必要的克隆**

```rust
// ❌ 错误：频繁克隆
for icon in &icons {
    let cloned = icon.clone();  // 不必要的克隆
    classify(cloned);
}

// ✅ 正确：使用引用
for icon in &icons {
    classify(icon);
}
```

**规则 2：高频路径优化**

```rust
// ❌ 错误：UI 更新路径有日志
fn update(&mut self) {
    log::debug!("更新 UI");  // 每帧都执行
    self.render();
}

// ✅ 正确：只在必要时记录
fn update(&mut self) {
    self.render();
}
```

**规则 3：延迟初始化**

```rust
// ✅ 正确：延迟初始化
pub struct MyApp {
    classifier: Option<AppClassifier>,  // 延迟加载
}

impl MyApp {
    fn get_classifier(&mut self) -> &AppClassifier {
        if self.classifier.is_none() {
            self.classifier = Some(AppClassifier::new()
                .unwrap_or_else(|e| {
                    log::warn!("初始化失败: {}", e);
                    AppClassifier::default()
                }));
        }
        self.classifier.as_ref().unwrap()
    }
}
```

---

## Examples

### 示例 1：错误处理

[src/context.rs](src/context.rs):
- `AppContext::new()` 中使用 `unwrap_or_default()` 优雅降级

### 示例 2：模块文档

[src/platform/mod.rs](src/platform/mod.rs):
- 完整的模块文档注释

### 示例 3：Default 实现

[src/config/app_config.rs](src/config/app_config.rs):
- `Default` trait 实现
- 所有子结构体都有合理的默认值

---

## Common Mistakes

1. **忘记运行 `cargo fmt`** → 提交未格式化代码
2. **忘记运行 `cargo clippy`** → 提交有警告的代码
3. **忘记添加文档** → 公开 API 无文档注释
4. **忘记实现 `Default`** → 加载失败无法降级
5. **忘记 `.context()`** → 错误信息缺少上下文

---

## Quick Checklist for New Code

- [ ] 代码已格式化（`cargo fmt`）
- [ ] Clippy 无警告（`cargo clippy`）
- [ ] 测试已通过（`cargo test`）
- [ ] 公开 API 有文档注释
- [ ] 使用 `anyhow::Result` 处理错误
- [ ] 关键操作有日志记录
- [ ] 无反向依赖
- [ ] 无性能问题（克隆、高频日志）
---

## DPI and Pixel Handling

### 多显示器 DPI 适配

本项目支持多显示器环境，不同显示器可能有不同的 DPI 设置。

**核心概念**：
- **物理像素（Physical Pixels）**：显示器实际像素，由 GetSystemMetrics 等 API 返回
- **逻辑像素（Logical Pixels）**：96 DPI 基准下的像素值，用于配置存储和跨 DPI 计算

**转换公式**：
`ust
// 物理像素 → 逻辑像素
let logical_px = (physical_px * 96) / dpi;

// 逻辑像素 → 物理像素
let physical_px = (logical_px * dpi) / 96;
`

### 窗口定位规范

**规则 1：GridCreateParams 使用逻辑像素**

`ust
// ✅ 正确：x/y 是逻辑像素（96 DPI 基准）
let params = GridCreateParams {
    x: logical_x,
    y: logical_y,
    width: logical_w,
    height: logical_h,
    ...
};

// create_grid_window 内部会按实际 DPI 缩放
`

**规则 2：GetSystemMetrics 返回物理像素**

`ust
// ❌ 错误：直接使用物理像素计算居中
let (screen_w, screen_h) = get_screen_resolution();  // 物理像素
let x = (screen_w - box_w) / 2;  // box_w 是逻辑像素，单位不匹配！

// ✅ 正确：先转换为逻辑像素
let (screen_w_phys, screen_h_phys) = get_screen_resolution();
let dpi = get_primary_monitor_dpi();
let screen_w = (screen_w_phys * 96) / dpi as i32;  // 逻辑像素
let screen_h = (screen_h_phys * 96) / dpi as i32;  // 逻辑像素
let x = (screen_w - box_w) / 2;  // 现在单位一致
`

### 拖拽残影 DPI 适配

**规则 3：拖拽残影使用光标所在显示器的 DPI**

`ust
// ✅ 正确：获取光标位置的 DPI
let cursor_dpi = dpi_at_screen_point(cursor_x, cursor_y);

// 按光标 DPI 渲染残影（而非源窗口 DPI）
let icon_size = scale_dpi(base_size, cursor_dpi, 96);
`

**原因**：残影跟随光标移动，可能跨越不同 DPI 的显示器。使用光标 DPI 确保残影大小与目标显示器匹配。

### 相关函数

| 函数 | 用途 | 返回值类型 |
|------|------|-----------|
| get_screen_resolution() | 获取主屏物理像素尺寸 | (i32, i32) 物理像素 |
| get_primary_monitor_dpi() | 获取主屏 DPI | u32 |
| dpi_at_screen_point(x, y) | 获取指定坐标的 DPI | u32 |
| scale_for_dpi(px, dpi) | 逻辑像素 → 物理像素 | i32 |

### 单元测试要求

所有涉及 DPI 转换的纯逻辑函数必须有单元测试。

---

## Common Mistakes (补充)

6. **混淆物理像素和逻辑像素** → 窗口位置偏移或大小异常
7. **拖拽残影使用源窗口 DPI** → 跨显示器时残影大小不匹配
8. **忘记 DPI 转换直接计算居中** → 高 DPI 屏幕下位置错误
