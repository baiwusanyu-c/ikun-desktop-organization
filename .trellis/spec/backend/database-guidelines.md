# Database Guidelines

> 数据持久化模式（v2.0 统一配置架构）

---

## Overview

本项目使用**统一 JSON 配置**进行数据持久化：

- **统一配置**：单一 `~/.ikun_organizer.json` 管理所有设置（`src/config/app_config.rs`）
- **分类规则**：`classification` 字段，`Vec<CategoryRule>`（AI 失败时的兜底规则）
- **布局数据**：`layout.boxes` 字段，`Vec<BoxLayoutData>`（分类框位置/尺寸/内容）

**数据持久化原则**：
- 使用 `serde` 进行序列化/反序列化
- 配置文件不存在时使用 Default，不阻塞应用启动
- 支持导入/导出（用户可分享配置）

---

## Data Storage

### 统一配置文件

| 配置段 | 内容 | 对应结构体 |
|--------|------|-----------|
| `general` | 开机自启、语言、单实例、启动恢复 | `GeneralConfig` |
| `hotkeys` | toggle_panel、auto_classify | `HotkeyConfig` |
| `box_style` | 背景色/透明度、圆角、字号、图标大小 | `BoxStyleConfig` |
| `box_defaults` | 新建分类框默认视图/排序/分组/尺寸 | `BoxDefaultsConfig` |
| `ai` | enabled、base_url、api_key、model、timeout | `AiConfig` |
| `classification` | 分类规则列表（兜底规则） | `Vec<CategoryRule>` |
| `layout` | 分类框布局（位置/尺寸/锁定/排序/视图） | `LayoutConfig` |
| `overlay` | 桌面覆盖窗口（未管理图标位置、图标尺寸、排列方向） | `OverlayConfig` |
| `debug` | log_level、log_to_file、show_fps | `DebugConfig` |

### 存储位置

```
所有平台: ~/.ikun_organizer.json（用户主目录）
```

获取路径：
```rust
fn get_config_path() -> anyhow::Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".ikun_organizer.json"))
        .ok_or_else(|| anyhow::anyhow!("无法获取用户主目录"))
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

### 保存模式（原子写入）

配置保存必须使用**原子写入**（临时文件 + `rename` 替换），避免进程在写入中途被杀
（强制关机/断电）产生半截配置文件——配置文件损坏会在下次启动时触发"加载失败回退默认值"，
进而引发连锁故障（如开机自启条目被误删）。见 `atomic_write_file`（app_config.rs）。

```rust
pub fn save(&self) -> anyhow::Result<()> {
    let path = Self::get_config_path()?;
    let content = serde_json::to_string_pretty(self)?;
    atomic_write_file(&path, &content)?;   // 临时文件 + rename 原子替换
    log::info!("保存配置成功: {:?}", path);
    Ok(())
}

/// 先写临时文件再 rename；rename 失败时清理临时文件
fn atomic_write_file(path: &Path, content: &str) -> anyhow::Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, content)?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }
    Ok(())
}
```

### 加载失败防护

加载失败回退默认值时，调用方必须能区分"真实配置"与"回退默认值"：
`AppContext.config_loaded = false` 时，禁止按配置同步系统状态（开机自启注册表等），
否则默认值会覆盖用户的真实系统状态。

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
