# Database Guidelines

> Data persistence patterns in this project.

---

## Overview

本项目不使用传统关系型数据库，而是使用**文件持久化**方式：

- **配置数据**：YAML 格式（`data/categories.yaml`）
- **用户布局**：JSON 格式（`~/.config/.../layout.json`）
- **应用设置**：JSON 格式（未来扩展）

**数据持久化原则**：
- 使用 `serde` 进行序列化/反序列化
- 配置文件优先级：用户配置 > 项目默认 > 系统默认
- 失败时使用默认值，不阻塞应用启动

---

## Data Storage Locations

| 数据类型 | 格式 | 存储位置 | 加载时机 |
|---------|------|----------|----------|
| 分类规则数据库 | YAML | `data/categories.yaml` | 启动时 |
| 用户自定义分类 | YAML | `~/.config/.../categories.yaml` | 启动时 |
| 桌面布局 | JSON | `~/.config/.../layout.json` | 启动时 |
| 应用设置 | JSON | `~/.config/.../settings.json` | 启动时 |

**配置目录路径获取**（使用 `dirs` crate）：
```rust
// macOS/Linux: ~/.config/ikun-desktop-organization/
// Windows: %APPDATA%/ikun-desktop-organization/
let config_dir = dirs::config_dir()
    .map(|p| p.join("ikun-desktop-organization"))
    .ok_or_else(|| anyhow::anyhow!("无法获取配置目录"))?;
```

---

## Data Model Patterns

### 数据结构定义

**规则 1：使用 `serde` derive macros**

```rust
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopLayout {
    pub version: String,
    pub last_modified: chrono::DateTime<chrono::Utc>,
    pub boxes: Vec<BoxData>,
}
```

**规则 2：为所有数据结构提供 `Default` 实现**

```rust
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

### 加载模式

**优先级加载策略**：

```rust
pub fn load() -> anyhow::Result<Self> {
    // 尝试多个路径，按优先级排序
    let paths: Vec<PathBuf> = vec![
        PathBuf::from("data/categories.yaml"),           // 项目默认
        dirs::config_dir()
            .map(|p| p.join(".../categories.yaml"))      // 用户配置
            .unwrap_or_else(|| PathBuf::from("")),
    ];

    for path in paths.iter() {
        if !path.as_os_str().is_empty() && path.exists() {
            let content = std::fs::read_to_string(path)?;
            let data: Self = serde_yaml::from_str(&content)?;
            log::info!("加载成功，路径: {}", path.display());
            return Ok(data);
        }
    }

    // 所有路径都失败，返回默认值
    log::info!("使用默认数据");
    Ok(Self::default())
}
```

### 保存模式

**原子写入策略**：

```rust
pub fn save(&self) -> anyhow::Result<()> {
    let path = Self::get_config_path()?;

    // 创建目录（如果不存在）
    std::fs::create_dir_all(path.parent().unwrap())?;

    // 序列化数据
    let content = serde_json::to_string_pretty(self)?;

    // 写入文件
    std::fs::write(path, content)?;

    log::info!("保存成功，路径: {}", path.display());
    Ok(())
}
```

---

## Serialization Format Choice

| 格式 | 使用场景 | Rust crate |
|------|----------|-----------|
| **YAML** | 可读配置、用户可编辑 | `serde_yaml` |
| **JSON** | 结构化数据、程序内部 | `serde_json` |

**YAML 适用场景**：
- 分类规则数据库（用户可手动调整）
- 配置文件（需要注释和可读性）

**JSON 适用场景**：
- 布局数据（结构复杂，包含 UUID、时间戳）
- 应用设置（机器生成，用户不直接编辑）

---

## Error Handling in Data Operations

**规则：失败时使用默认值，不阻塞应用**

```rust
// ✅ 正确：失败时使用默认值，记录警告
let layout = DesktopLayout::load()
    .unwrap_or_else(|e| {
        log::warn!("加载布局失败: {}, 使用默认布局", e);
        DesktopLayout::default()
    });

// ❌ 错误：失败时 panic
let layout = DesktopLayout::load().expect("必须加载布局");
```

---

## Examples

### 示例 1：YAML 数据结构

[src/classification/database.rs](src/classification/database.rs):
- `AppCategoryDB` 和 `AppCategory` 定义
- `load()` 多路径优先级加载
- `save()` 用户配置保存
- `Default` 默认分类数据

### 示例 2：JSON 数据结构

[src/config/layout.rs](src/config/layout.rs):
- `DesktopLayout` 和 `BoxData` 定义
- 使用 `chrono::DateTime` 和 `uuid::Uuid`
- `load()` 和 `save()` 实现

---

## Forbidden Patterns

**❌ 不要硬编码配置路径**

```rust
// ❌ 错误：硬编码路径
let path = "/Users/username/.config/...";

// ✅ 正确：使用 dirs crate
let path = dirs::config_dir()
    .map(|p| p.join("ikun-desktop-organization/layout.json"))
    .ok_or_else(|| anyhow::anyhow!("无法获取配置目录"))?;
```

**❌ 不要在加载失败时阻塞应用**

```rust
// ❌ 错误：返回错误，应用无法启动
fn new() -> anyhow::Result<Self> {
    let layout = DesktopLayout::load()?;  // 失败时直接返回错误
    Ok(Self { layout })
}

// ✅ 正确：失败时使用默认值
fn new() -> anyhow::Result<Self> {
    let layout = DesktopLayout::load()
        .unwrap_or_else(|e| {
            log::warn!("加载失败: {}", e);
            DesktopLayout::default()
        });
    Ok(Self { layout })
}
```

**❌ 不要混用序列化格式**

```rust
// ❌ 错误：YAML 数据用 JSON 加载
let data: AppCategoryDB = serde_json::from_str(&content)?;

// ✅ 正确：YAML 数据用 YAML 加载
let data: AppCategoryDB = serde_yaml::from_str(&content)?;
```

---

## Common Mistakes

1. **忘记 `Serialize, Deserialize` derive** → 编译错误
2. **忘记 `Default` 实现** → 加载失败时无法回退
3. **路径优先级错误** → 用户配置被项目默认覆盖
4. **忘记创建目录** → 保存失败（`create_dir_all` 缺失）

---

## Quick Checklist for New Data Structure

- [ ] 定义 struct 并添加 `Serialize, Deserialize`
- [ ] 实现 `Default` trait（提供合理的默认值）
- [ ] 实现 `load()` 方法（多路径优先级）
- [ ] 实现 `save()` 方法（原子写入）
- [ ] 选择合适的序列化格式（YAML vs JSON）
- [ ] 在失败场景使用 `unwrap_or_else` 处理