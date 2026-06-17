# Logging Guidelines

> How logging is done in this project.

---

## Overview

本项目使用 `log` crate（ facade）+ `env_logger`（实现）进行日志记录。
日志写入 `~/.ikun_organizer.log` 文件（无控制台输出，所有构建均使用 `windows_subsystem = "windows"`）。

**日志原则**：
- **信息丰富**：包含操作详情、路径、错误信息
- **级别合理**：根据重要性选择正确的日志级别
- **中文友好**：日志消息使用中文（本项目用户为中文用户）
- **性能优先**：关键路径避免频繁日志
- **文件输出**：日志写入文件，不使用控制台

---

## Logging Libraries

```toml
[dependencies]
log = "0.4"           # 日志 facade
env_logger = "0.11"   # 日志实现
```

**初始化日志系统**（main.rs）：

```rust
fn main() -> eframe::Result<()> {
    // 初始化日志（使用工具函数）
    crate::utils::init_logger();
    
    log::info!("========================================");
    log::info!("启动 Desktop Organizer...");
    log::info!("========================================");
    ...
}
```

**日志工具模块**（src/utils/logger.rs）：

项目提供了便捷的日志宏：

```rust
use crate::utils::{log_info, log_debug, log_warn, log_error};

// 自动包含模块路径
log_info!("应用启动成功");  // 输出: [ikun_desktop_organization] 应用启动成功

// 自动包含文件和行号
log_debug!("变量值: {}", value);  // 输出: [src/app.rs:42] 变量值: 123

// 性能日志
log_perf("图标分类", elapsed_ms);  // 输出: [性能] 图标分类 耗时: 123ms

// 分隔线
log_separator("开始处理桌面图标");
```

**运行时控制日志级别**：

```powershell
# PowerShell 设置日志级别
$env:RUST_LOG="debug"; cargo run

# 查看日志文件
Get-Content ~\.ikun_organizer.log -Tail 50
```

---

## Log Levels

### 级别定义和使用场景

| 级别 | 使用场景 | 示例 |
|------|----------|------|
| **ERROR** | 致命错误，应用无法继续运行 | 窗口创建失败、关键依赖缺失 |
| **WARN** | 非致命错误，使用降级策略 | 配置加载失败、桌面图标读取失败 |
| **INFO** | 重要操作成功提示 | 应用启动、分类完成、配置保存成功 |
| **DEBUG** | 调试信息，开发时使用 | 分类匹配详情、图标位置变化 |

### 级别选择规则

```rust
// ✅ 正确：致命错误使用 ERROR
log::error!("保存配置失败: {}, 路径: {}", e, path.display());

// ✅ 正确：降级错误使用 WARN
log::warn!("加载布局失败: {}, 使用默认布局", e);

// ✅ 正确：操作成功使用 INFO
log::info!("加载分类数据库成功，版本: {}", db.version);

// ✅ 正确：调试信息使用 DEBUG
log::debug!("匹配图标 '{}' 到分类 '{}'", icon_name, category_name);
```

---

## What to Log

### 必须记录的事件

| 事件 | 级别 | 包含信息 |
|------|------|----------|
| 应用启动 | INFO | 版本、平台 |
| 配置加载成功 | INFO | 路径、版本 |
| 配置加载失败 | WARN | 错误、降级策略 |
| 分类完成 | INFO | 分类框数量 |
| 配置保存成功 | INFO | 路径 |
| 配置保存失败 | ERROR | 错误、路径 |
| 桌面图标读取失败 | WARN | 错误、降级策略 |

### 日志格式规范

**规则 1：包含操作结果**

```rust
// ✅ 正确：包含成功/失败状态
log::info!("加载分类数据库成功，版本: {}", db.version);
log::warn!("加载布局失败: {}", e);
```

**规则 2：包含关键路径**

```rust
// ✅ 正确：包含文件路径
log::info!("加载成功，路径: {}", path.display());
log::error!("保存失败，路径: {}", path.display());
```

**规则 3：包含数量统计**

```rust
// ✅ 正确：包含数量
log::info!("分类完成，创建了 {} 个分类框", self.classification_boxes.len());
log::info!("加载了 {} 个桌面图标", icons.len());
```

---

## What NOT to Log

### 禁止记录的信息

| 信息类型 | 原因 |
|---------|------|
| 用户文件路径细节 | 隐私问题 |
| 个人信息（PII） | 隐私问题 |
| 系统密码/密钥 | 安全问题 |
| 频繁的状态更新 | 性能问题 |

### 性能考虑

**避免频繁日志**：

```rust
// ❌ 错误：每帧都记录日志（性能问题）
fn update(&mut self, ctx: &egui::Context) {
    log::debug!("更新帧");  // 每秒 60 次！
}

// ✅ 正确：只在重要事件时记录
fn update(&mut self, ctx: &egui::Context) {
    if self.classification_changed {
        log::info!("分类已更新");
    }
}
```

---

## Examples

### 示例 1：应用启动

[src/main.rs](src/main.rs):
```rust
utils::init_logger();
log::info!("启动 Desktop Organizer (重写版)");
```

### 示例 2：配置加载

[src/config/app_config.rs](src/config/app_config.rs):
```rust
log::info!("加载配置成功: {:?}", path);
log::info!("配置文件不存在，使用默认配置");
```

### 示例 3：分类完成

[src/app.rs](src/app.rs):
```rust
log::info!("分类结果: 游戏={}, 开发工具={}, 办公={}", games.len(), dev_tools.len(), office.len());
```

### 示例 4：数据加载成功

[src/classification/database.rs](src/classification/database.rs):
```rust
log::info!("使用默认分类数据库");
```

---

## Structured Logging (Future)

未来可以考虑使用结构化日志（如 `tracing` crate）：

```rust
// 未来改进：使用 tracing
use tracing::{info, warn, error};

#[instrument(skip(self))]
pub fn classify(&self, icon: &DesktopIcon) -> Option<AppCategory> {
    info!("开始分类图标: {}", icon.name);
    let result = self.matcher.classify(icon);
    if let Some(category) = &result {
        info!("分类结果: {}", category.name);
    }
    result
}
```

**优势**：
- 自动添加函数名和参数
- 更好的性能（异步日志）
- 可集成到监控系统

---

## Forbidden Patterns

**❌ 不要在日志中使用英文（本项目用户为中文用户）**

```rust
// ❌ 错误：英文日志
log::info!("Loaded config successfully");

// ✅ 正确：中文日志
log::info!("加载配置成功");
```

**❌ 不要缺少关键信息**

```rust
// ❌ 错误：缺少错误详情
log::warn!("加载失败");

// ✅ 正确：包含错误详情
log::warn!("加载布局失败: {}, 使用默认布局", e);
```

**❌ 不要在高频路径使用 DEBUG 日志**

```rust
// ❌ 错误：每帧都记录（性能问题）
fn update(&mut self) {
    log::debug!("更新 UI");
}

// ✅ 正确：只在重要事件时记录
fn perform_classification(&mut self) {
    log::info!("执行一键智能分类...");
}
```

---

## Common Mistakes

1. **日志级别错误** → ERROR 用于非致命错误，INFO 用于调试信息
2. **缺少关键信息** → "加载失败" 而不是 "加载失败: ..."
3. **高频路径日志** → 每帧记录日志导致性能下降
4. **硬编码英文** → 中文用户看到的日志是英文

---

## Quick Checklist for Adding Log

- [ ] 选择正确的日志级别（ERROR/WARN/INFO/DEBUG）
- [ ] 使用中文日志消息
- [ ] 包含操作详情（路径、数量、错误信息）
- [ ] 非高频路径（避免每帧/每秒多次）
- [ ] 不记录敏感信息（隐私、安全）