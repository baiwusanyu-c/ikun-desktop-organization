# Database Guidelines

> 数据持久化模式（v2.0 统一配置架构）

---

## Overview

本项目使用**统一 JSON 配置**进行数据持久化：

- **统一配置**：单一 `config.json` 管理所有设置（`src/config/app_config.rs`）
- **分类规则**：内嵌在配置的 `classification.categories` 字段中
- **布局数据**：内嵌在配置的 `layout.boxes` 字段中

**数据持久化原则**：
- 使用 `serde` 进行序列化/反序列化
- 配置文件不存在时使用 Default，不阻塞应用启动
- 支持导入/导出（用户可分享配置）

---

## Data Storage

### 统一配置文件

| 配置段 | 内容 | 对应结构体 |
|--------|------|-----------|
| `general` | 开机自启、语言、单实例 | `GeneralConfig` |
| `box_style` | 背景色/透明度、圆角、字号、图标大小 | `BoxStyleConfig` |
| `ai` | enabled、base_url、api_key、model | `AiConfig` |
| `hotkeys` | toggle_panel、toggle_icons、auto_classify | `HotkeyConfig` |
| `tray` | enabled、show_on_start、double_click_action | `TrayConfig` |
| `layout` | 分类框列表（位置/尺寸/锁定/排序/视图） | `LayoutConfig` |
| `classification` | 分类规则（内嵌或引用外部） | `ClassificationConfig` |
| `debug` | log_level、log_to_file、show_fps | `DebugConfig` |

### 存储位置

```
Windows: %APPDATA%/ikun-desktop-organization/config.json
macOS:   ~/.config/ikun-desktop-organization/config.json
```

获取路径：
```rust
fn get_config_path() -> anyhow::Result<PathBuf> {
    dirs::config_dir()
        .map(|p| p.join("ikun-desktop-organization/config.json"))
        .ok_or_else(|| anyhow::anyhow!("无法获取配置目录"))
}
```

---

## Data Model Patterns

### 数据结构定义

**规则 1：使用 `serde` derive macros**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: String,
    pub last_modified: chrono::DateTime<chrono::Utc>,
    pub general: GeneralConfig,
    pub box_style: BoxStyleConfig,
    // ...
}
```

**规则 2：为所有数据结构提供 `Default` 实现**

```rust
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: "2.0".to_string(),
            last_modified: chrono::Utc::now(),
            general: GeneralConfig::default(),
            box_style: BoxStyleConfig::default(),
            // ...
        }
    }
}
```

**规则 3：枚举使用 `#[serde(rename_all = "snake_case")]`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    Name, Type, ModifiedTime, Author,
    CreatedTime, Size, Tag, Title,
}
```

### 加载模式

```rust
pub fn load() -> anyhow::Result<Self> {
    let path = Self::get_config_path()?;
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        let config: AppConfig = serde_json::from_str(&content)?;
        log::info!("加载配置成功: {:?}", path);
        Ok(config)
    } else {
        log::info!("配置文件不存在，使用默认配置");
        Ok(Self::default())
    }
}
```

**调用者降级处理**：
```rust
// context.rs
let config = AppConfig::load().unwrap_or_else(|e| {
    log::warn!("加载配置失败: {}, 使用默认配置", e);
    AppConfig::default()
});
```

### 保存模式

```rust
pub fn save(&self) -> anyhow::Result<()> {
    let path = Self::get_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(self)?;
    std::fs::write(&path, content)?;
    log::info!("保存配置成功: {:?}", path);
    Ok(())
}
```

### 导入/导出模式

```rust
pub fn export_to(&self, path: &Path) -> anyhow::Result<()> {
    let content = serde_json::to_string_pretty(self)?;
    std::fs::write(path, content)?;
    Ok(())
}

pub fn import_from(path: &Path) -> anyhow::Result<Self> {
    let content = std::fs::read_to_string(path)?;
    let config: AppConfig = serde_json::from_str(&content)?;
    Ok(config)
}
```

---

## Error Handling in Data Operations

**规则：失败时使用默认值，不阻塞应用**

```rust
// ✅ 正确：失败时使用默认值
let config = AppConfig::load()
    .unwrap_or_else(|e| {
        log::warn!("加载配置失败: {}, 使用默认配置", e);
        AppConfig::default()
    });

// ❌ 错误：失败时 panic
let config = AppConfig::load().expect("必须加载配置");
```

---

## Forbidden Patterns

**❌ 不要硬编码配置路径**

```rust
// ❌ 错误
let path = "C:\\Users\\xxx\\config.json";

// ✅ 正确：使用 dirs crate
let path = dirs::config_dir()
    .map(|p| p.join("ikun-desktop-organization/config.json"))
    .ok_or_else(|| anyhow::anyhow!("无法获取配置目录"))?;
```

**❌ 不要在加载失败时阻塞应用**

```rust
// ❌ 错误
fn new() -> anyhow::Result<Self> {
    let config = AppConfig::load()?;  // 失败时应用无法启动
    Ok(Self { config })
}

// ✅ 正确
fn new() -> anyhow::Result<Self> {
    let config = AppConfig::load().unwrap_or_default();
    Ok(Self { config })
}
```

---

## Quick Checklist for New Config Field

- [ ] 在 `AppConfig` 或子结构体中添加字段
- [ ] 添加 `Serialize, Deserialize` derive
- [ ] 在 `Default` 实现中提供合理默认值
- [ ] 如果是枚举，添加 `#[serde(rename_all = "snake_case")]`
- [ ] 验证 `cargo run` 能正常加载/保存
