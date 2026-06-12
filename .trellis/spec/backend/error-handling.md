# Error Handling

> How errors are handled in this project.

---

## Overview

本项目使用 `anyhow` crate 进行错误处理，遵循以下原则：

- **统一错误类型**：使用 `anyhow::Result<T>` 作为返回类型
- **错误链追踪**：使用 `.context()` 添加错误上下文
- **优雅降级**：关键功能失败时使用默认值，不阻塞应用
- **错误记录**：所有错误必须通过日志记录

**为什么选择 anyhow**：
- 简化错误处理，无需定义自定义错误类型
- 自动错误链追踪，便于调试
- 与 `?` 运算符完美配合

---

## Error Types

### 统一使用 `anyhow::Result`

```rust
// ✅ 正确：使用 anyhow::Result
pub fn load() -> anyhow::Result<Self> { ... }

// ❌ 错误：自定义错误类型（增加复杂度）
pub fn load() -> Result<Self, MyCustomError> { ... }
```

### 错误上下文添加

**使用 `.context()` 添加描述性信息**：

```rust
use anyhow::{Context, Result};

pub fn load_config() -> Result<DesktopLayout> {
    let path = get_config_path()
        .context("无法获取配置目录路径")?;

    let content = std::fs::read_to_string(&path)
        .context(format!("无法读取配置文件: {}", path.display()))?;

    let layout: DesktopLayout = serde_json::from_str(&content)
        .context("配置文件格式错误，无法解析 JSON")?;

    Ok(layout)
}
```

---

## Error Handling Patterns

### Pattern 1: 优雅降级（推荐）

**关键功能失败时使用默认值，记录警告**：

```rust
// ✅ 正确：失败时使用默认值
let layout = DesktopLayout::load()
    .unwrap_or_else(|e| {
        log::warn!("加载布局失败: {}, 使用默认布局", e);
        DesktopLayout::default()
    });
```

**适用场景**：
- 配置加载失败
- 桌面图标读取失败
- 分类数据库加载失败

### Pattern 2: 错误传播

**非关键错误直接返回，由调用者处理**：

```rust
// ✅ 正确：非关键错误返回
pub fn save(&self) -> anyhow::Result<()> {
    let path = Self::get_config_path()?;
    std::fs::write(path, content)?;
    Ok(())
}

// 调用者处理
if let Err(e) = layout.save() {
    log::error!("保存失败: {}", e);
}
```

**适用场景**：
- 配置保存失败
- 文件写入失败

### Pattern 3: 致命错误

**应用无法继续运行时，记录错误并退出**：

```rust
// ✅ 正确：致命错误记录并返回
fn main() -> eframe::Result<()> {
    if let Err(e) = init_critical_components() {
        log::error!("初始化失败: {}", e);
        return Err(e.into());
    }
    ...
}
```

**适用场景**：
- egui 窗口创建失败
- 关键依赖缺失

---

## Error Logging

### 错误级别对应

| 错误级别 | 场景 | 示例 |
|---------|------|------|
| `log::error!` | 致命错误，应用无法继续 | 窗口创建失败 |
| `log::warn!` | 非致命错误，使用默认值 | 配置加载失败 |
| `log::info!` | 操作成功提示 | 配置加载成功 |

### 错误日志模式

```rust
// ✅ 正确：包含错误详情
log::warn!("加载布局失败: {}, 使用默认布局", e);

// ✅ 正确：包含上下文信息
log::error!("保存配置失败: {}, 路径: {}", e, path.display());

// ❌ 错误：不记录错误详情
log::warn!("加载失败");  // 缺少具体错误信息
```

---

## Examples

### 示例 1：配置加载（AppContext）

[src/context.rs](src/context.rs):
```rust
pub fn new() -> anyhow::Result<Self> {
    let config = AppConfig::load()?;  // 失败时调用者降级
    let desktop_manager = DesktopManager::new().unwrap_or_default();
    let classifier = AppClassifier::new().unwrap_or_default();
    Ok(Self { config, desktop_manager, classifier, state: AppState::default() })
}
```

### 示例 2：保存配置

[src/context.rs](src/context.rs):
```rust
pub fn save_config(&mut self) -> anyhow::Result<()> {
    self.config.last_modified = chrono::Utc::now();
    self.config.save()
}
```

### 示例 3：App 中的降级处理

[src/app.rs](src/app.rs):
```rust
impl Default for MyApp {
    fn default() -> Self {
        let ctx = AppContext::new().unwrap_or_default();
        Self { ctx, initialized: false }
    }
}
```

---

## Forbidden Patterns

**❌ 不要使用 `panic!` 或 `expect` 处理可恢复错误**

```rust
// ❌ 错误：panic 会终止应用
let layout = DesktopLayout::load().expect("必须加载布局");

// ❌ 错误：unwrap 可能 panic
let layout = DesktopLayout::load().unwrap();

// ✅ 正确：使用 unwrap_or_else 优雅降级
let layout = DesktopLayout::load()
    .unwrap_or_else(|e| {
        log::warn!("加载失败: {}", e);
        DesktopLayout::default()
    });
```

**❌ 不要忽略错误**

```rust
// ❌ 错误：忽略错误
let _ = layout.save();

// ✅ 正确：至少记录错误
if let Err(e) = layout.save() {
    log::error!("保存失败: {}", e);
}
```

**❌ 不要在错误消息中使用硬编码英文**

```rust
// ❌ 错误：硬编码英文消息
.context("Failed to load config")?;

// ✅ 正确：使用中文（本项目用户为中文用户）
.context("无法加载配置文件")?;
```

---

## Common Mistakes

1. **忘记添加 `.context()`** → 错误链断裂，难以调试
2. **使用 `unwrap()` 或 `expect()`** → 应用意外崩溃
3. **忽略错误** → 问题被隐藏，难以发现
4. **错误日志缺少详情** → 无法定位问题根源

---

## Quick Checklist for Error Handling

- [ ] 使用 `anyhow::Result` 作为返回类型
- [ ] 使用 `.context()` 添加错误上下文
- [ ] 关键功能失败时使用 `unwrap_or_else` 降级
- [ ] 所有错误通过 `log::warn!` 或 `log::error!` 记录
- [ ] 不使用 `panic!`, `unwrap()`, `expect()` 处理可恢复错误