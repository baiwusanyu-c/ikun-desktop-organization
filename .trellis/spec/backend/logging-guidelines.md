# Logging Guidelines

> How logging is done in this project.

---

## Overview

本项目使用 `log` crate（facade）+ `env_logger`（实现）进行日志记录。
日志写入 `~/.ikun_organizer_log/` 目录，按日期和序号分文件存储。
所有构建均使用 `windows_subsystem = "windows"`，无控制台输出。

**日志文件规则**：
- **存储目录**：`~/.ikun_organizer_log/`
- **文件命名**：`.ikun_organizer_log_YYYYMMDD_N`（如 `.ikun_organizer_log_20260722_1`）
- **大小限制**：单文件超过 2MB 自动切换到下一个序号文件
- **自动清理**：启动时删除超过 7 天的日志文件

**日志原则**：
- **信息丰富**：包含操作详情、路径、错误信息
- **级别合理**：根据重要性选择正确的日志级别
- **中文友好**：日志消息使用中文（本项目用户为中文用户）
- **性能优先**：关键路径避免频繁日志
- **发行版标记**：生产环境关键日志使用 `[RELEASE]` 前缀宏

---

## Logging Libraries

```toml
[dependencies]
log = "0.4"           # 日志 facade
env_logger = "0.11"   # 日志实现
chrono = "0.4"        # 时间处理（日志日期格式化）
dirs = "5.0"          # 用户目录/配置路径获取
```

**初始化日志系统**（main.rs）：

```rust
fn main() -> anyhow::Result<()> {
    // 初始化日志（使用工具函数）
    crate::utils::init_logger();
    
    log::info!("========================================");
    log::info!("启动 Desktop Organizer...");
    log::info!("========================================");
    ...
}
```

**日志工具模块**（src/utils/logger.rs）：

项目提供了两类日志宏：

### 开发日志宏

用于开发阶段的调试和一般日志：

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

### 发行版日志宏

用于生产环境问题排查，带有 `[RELEASE]` 前缀。**这些日志调用是永久性代码，不应作为调试代码或死代码删除。**

```rust
use crate::utils::{release_info, release_warn, release_error};

// 发行版信息日志
release_info!("应用启动，版本: {}", env!("CARGO_PKG_VERSION"));
// 输出: [RELEASE] [ikun_organizer::main] 应用启动，版本: 0.1.0

// 发行版警告日志
release_warn!("配置加载失败: {}，使用默认配置", e);
// 输出: [RELEASE] [ikun_organizer::config] 配置加载失败: ...

// 发行版错误日志
release_error!("关键操作失败: {}", e);
// 输出: [RELEASE] [ikun_organizer::platform] 关键操作失败: ...
```

**何时使用发行版日志**：
- 应用启动/退出事件
- 配置加载/保存结果
- 关键功能初始化结果（OLE、DPI、窗口创建等）
- 用户可见操作的开始和结果（AI 分类、布局恢复等）
- 非预期的异常和降级处理

**运行时控制日志级别**：

```powershell
# PowerShell 设置日志级别
$env:RUST_LOG="debug"; cargo run

# 查看日志文件
Get-ChildItem ~\.ikun_organizer_log\ | Sort-Object LastWriteTime -Descending | Select-Object -First 5
Get-Content ~\.ikun_organizer_log\.ikun_organizer_log_20260722_1 -Tail 50
```

---


---

## Window Context Logging

Tauri 应用包含三个 WebView 窗口（control-panel, create-box, edit-box），日志系统通过线程局部存储自动标记窗口来源。

### WindowLogGuard

在 Tauri Command 入口使用 RAII Guard 自动管理窗口标签：

```rust
use crate::utils::WindowLogGuard;

#[tauri::command]
pub fn get_config(window: tauri::Window, state: ...) -> ... {
    let _guard = WindowLogGuard::new(window.label());
    // All log::xxx! calls now include [window:control-panel]
    ...
}
```

### 日志格式

```
// 有窗口上下文:
[2026-08-12 14:30:22 INFO config_cmd] [window:control-panel] 加载配置成功

// 无窗口上下文（启动时、后台线程等）:
[2026-08-12 14:30:22 INFO main] Starting Desktop Organizer
```

### 前端 IPC 日志

前端 Svelte 页面可通过 `logToRust(level, message)` 将日志上报到 Rust 后端：

```typescript
import { logToRust } from "$lib/api/tauri-api";

// 在页面中使用
await logToRust('info', '用户点击了保存按钮');
await logToRust('error', '表单验证失败: ' + errorMessage);
```

Rust 端自动标记窗口来源：`[frontend:control-panel]`、`[frontend:create-box]`、`[frontend:edit-box]`。

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

## Log File System

### 文件轮转机制

日志系统使用 `RollingFileWriter` 实现自动文件轮转：

1. **日期检测**：每次写入时检查当前日期是否变化
   - 日期变化 → 切换到新日期的文件，序号从 1 开始
2. **大小检测**：每次写入前检查文件大小
   - 超过 2MB → 序号+1，打开新文件继续写入
3. **启动时序号选择**：扫描当日已有文件，找到最大序号
   - 如果最后一个文件未满 2MB → 继续使用该序号
   - 如果最后一个文件已满 → 使用下一个序号

### 日志清理机制

应用启动时自动清理过期日志：

- 扫描 `~/.ikun_organizer_log/` 目录中的所有日志文件
- 从文件名中解析日期（格式：`YYYYMMDD`）
- 删除日期距今超过 7 天的文件
- 清理失败不会阻塞应用启动

### 文件命名规范

```
.ikun_organizer_log_YYYYMMDD_N
```

- `YYYYMMDD`：日期（如 `20260722`）
- `N`：当日序号（从 1 开始，按需递增）
- 示例：`.ikun_organizer_log_20260722_1`、`.ikun_organizer_log_20260722_2`

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

### 发行版日志事件（推荐记录）

| 事件 | 宏 | 说明 |
|------|------|------|
| 应用启动 | `release_info!` | 版本、平台、构建类型 |
| 应用退出 | `release_info!` | 退出原因 |
| 配置加载结果 | `release_info!` / `release_warn!` | 成功/失败及降级策略 |
| 关键初始化结果 | `release_info!` / `release_warn!` | OLE、DPI、窗口创建 |
| 用户操作结果 | `release_info!` | AI 分类、布局恢复等 |
| 未预期异常 | `release_error!` | 错误详情和上下文 |

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
fn on_paint(&mut self) {
    log::debug!("更新帧");  // 每秒数十次！
}

// ✅ 正确：只在重要事件时记录
fn on_paint(&mut self) {
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
release_info!("启动 Desktop Organizer v{}", env!("CARGO_PKG_VERSION"));
log::info!("启动 Desktop Organizer (重写版)");
```

### 示例 2：配置加载

[src/config/app_config.rs](src/config/app_config.rs):
```rust
release_info!("加载配置: {:?}", path);
log::info!("加载配置成功: {:?}", path);
log::info!("配置文件不存在，使用默认配置");
```

### 示例 3：分类完成

[src/app.rs](src/app.rs):
```rust
release_info!("AI 分类完成: 游戏={}, 开发工具={}, 办公={}", games.len(), dev_tools.len(), office.len());
log::info!("分类结果: 游戏={}, 开发工具={}, 办公={}", games.len(), dev_tools.len(), office.len());
```

### 示例 4：错误处理

```rust
// ✅ 正确：使用 release_error 记录生产环境错误
if let Err(e) = save_config(&config) {
    release_error!("保存配置失败: {}, 路径: {}", e, path.display());
    log::error!("详细错误: {:?}", e);
}
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

**❌ 不要删除发行版日志**

```rust
// ❌ 错误：将发行版日志当作调试代码删除
// release_info!("应用启动");  // 已删除

// ✅ 正确：发行版日志是永久性代码，保留在代码中
release_info!("应用启动，版本: {}", env!("CARGO_PKG_VERSION"));
```

---

## Common Mistakes

1. **日志级别错误** → ERROR 用于非致命错误，INFO 用于调试信息
2. **缺少关键信息** → "加载失败" 而不是 "加载失败: ..."
3. **高频路径日志** → 每帧记录日志导致性能下降
4. **硬编码英文** → 中文用户看到的日志是英文
5. **删除发行版日志** → 将 `[RELEASE]` 宏调用误认为调试代码而删除

---

## Quick Checklist for Adding Log

- [ ] 选择正确的日志级别（ERROR/WARN/INFO/DEBUG）
- [ ] 使用中文日志消息
- [ ] 包含操作详情（路径、数量、错误信息）
- [ ] 非高频路径（避免每帧/每秒多次）
- [ ] 不记录敏感信息（隐私、安全）
- [ ] 生产环境关键日志使用 `release_*!` 宏