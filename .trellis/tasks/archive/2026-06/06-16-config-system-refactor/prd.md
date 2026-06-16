# 配置系统重构：用户目录 JSON 配置驱动启动

## Goal

将配置文件从 `%APPDATA%/ikun-desktop-organization/config.json` 迁移到用户主目录 `~/.ikun_organizer.json`，重新设计配置字段 schema，实现程序启动时读取配置并根据配置初始化各子系统。

## 核心设计

### 配置文件位置

- **路径**：`~/.ikun_organizer.json`（用户主目录）
- **获取方式**：`dirs::home_dir().map(|p| p.join(".ikun_organizer.json"))`
- **行为**：
  - 启动时读取此文件
  - 文件不存在则使用默认配置并自动创建
  - 运行时修改配置后写回此文件
  - 未来：导出 = 复制此文件；导入 = 覆盖此文件

### 配置文件 Schema（完整 JSON 示例）

```json
{
  "version": "2.0",
  "last_modified": "2026-06-16T10:00:00Z",

  "general": {
    "auto_start": false,
    "language": "zh-CN",
    "single_instance": true,
    "restore_layout_on_start": true,
    "auto_classify_on_start": false
  },

  "window": {
    "show_in_taskbar": false,
    "always_on_top": true,
    "remember_position": true,
    "last_position": null,
    "last_size": [320, 380]
  },

  "tray": {
    "enabled": true,
    "show_on_start": true,
    "left_click_action": "toggle_panel",
    "double_click_action": "show_panel"
  },

  "hotkeys": {
    "toggle_panel": "Ctrl+Alt+D",
    "toggle_icons": "Ctrl+Alt+H",
    "auto_classify": "Ctrl+Alt+C",
    "create_box": ""
  },

  "box_style": {
    "background_color": [1, 1, 1],
    "background_alpha": 38,
    "border_radius": 16.0,
    "border_color": [100, 100, 100, 100],
    "border_width": 1.0,
    "title_font_size": 15.0,
    "title_font_family": "Microsoft YaHei UI",
    "title_font_color": [255, 255, 255, 255],
    "label_font_size": 13.0,
    "label_font_color": [255, 255, 255, 230],
    "icon_size": 48,
    "cell_spacing": 16,
    "title_bar_height": 40,
    "title_bar_color": [30, 30, 30, 200],
    "highlight_color": [60, 80, 120, 120],
    "hover_color": [70, 90, 130, 70],
    "shadow_enabled": true,
    "shadow_color": [0, 0, 0, 200],
    "columns": 0,
    "lock_button_visible": true,
    "close_button_visible": true
  },

  "box_defaults": {
    "view_mode": "medium",
    "sort_mode": "name",
    "sort_ascending": true,
    "group_by": "none",
    "default_width": 680,
    "default_height": 800
  },

  "ai": {
    "enabled": false,
    "base_url": "https://api.openai.com/v1",
    "api_key": "",
    "model": "gpt-4o-mini",
    "max_tokens": 2048,
    "temperature": 0.7,
    "system_prompt": "你是一个桌面图标分类助手，请根据应用名称和路径判断其分类。",
    "timeout_seconds": 30,
    "max_retries": 3
  },

  "classification": {
    "source": "embedded",
    "file_path": null,
    "auto_match_on_drop": true,
    "categories": [
      {
        "id": "games",
        "name": "游戏",
        "icon": "🎮",
        "exe_patterns": ["steam.exe", "epicgameslauncher.exe"],
        "path_patterns": ["\\Games\\", "\\Steam\\"],
        "keywords": ["game", "play", "游戏"]
      }
    ]
  },

  "layout": {
    "boxes": [
      {
        "id": "uuid-here",
        "name": "游戏",
        "position": [100, 100],
        "size": [680, 800],
        "locked": false,
        "sort_mode": "name",
        "sort_ascending": true,
        "view_mode": "medium",
        "group_by": "none",
        "icon_names": ["steam.exe", "epic.exe"],
        "style_overrides": null
      }
    ]
  },

  "debug": {
    "log_level": "info",
    "log_to_file": false,
    "log_file_path": null,
    "log_max_size_mb": 10,
    "show_fps": false,
    "render_wireframe": false
  }
}
```

### 新增/变更字段说明

| 配置段 | 字段 | 类型 | 说明 | 状态 |
|--------|------|------|------|------|
| **general** | `restore_layout_on_start` | bool | 启动时恢复上次布局 | 新增 |
| **window** | 整段 | struct | 控制面板窗口设置 | 新增 |
| **window** | `show_in_taskbar` | bool | 不在任务栏显示 | 新增（PRD） |
| **window** | `always_on_top` | bool | 控制面板置顶 | 新增 |
| **window** | `remember_position` | bool | 记忆窗口位置 | 新增 |
| **window** | `last_position` | Option<[f32;2]> | 上次窗口位置 | 新增 |
| **window** | `last_size` | Option<[f32;2]> | 上次窗口尺寸 | 新增 |
| **tray** | `left_click_action` | String | 托盘左键行为 | 新增 |
| **hotkeys** | `create_box` | String | 创建分类框快捷键 | 新增（PRD） |
| **box_style** | `title_font_color` | [u8;4] | 标题字体颜色 | 新增 |
| **box_style** | `label_font_color` | [u8;4] | 标签字体颜色 | 新增 |
| **box_style** | `title_bar_color` | [u8;4] | 标题栏背景色 | 新增 |
| **box_style** | `columns` | i32 | 列数(0=自动) | 新增 |
| **box_style** | `lock_button_visible` | bool | 显示锁定按钮 | 新增 |
| **box_style** | `close_button_visible` | bool | 显示关闭按钮 | 新增 |
| **box_defaults** | 整段 | struct | 新分类框默认值 | 新增 |
| **ai** | `timeout_seconds` | u32 | API 超时秒数 | 新增 |
| **ai** | `max_retries` | u32 | 最大重试次数 | 新增 |
| **classification** | `auto_match_on_drop` | bool | 拖入时自动匹配分类 | 新增 |
| **layout.boxes** | `style_overrides` | Option | 每框样式覆盖 | 新增 |

## Requirements

1. **配置文件位置变更**：从 `%APPDATA%/ikun-desktop-organization/config.json` → `~/.ikun_organizer.json`
2. **Schema 完整设计**：覆盖 PRD 所有功能点，预留未来字段
3. **Default 全覆盖**：所有结构体实现 `Default`，所有字段有 `#[serde(default)]`
4. **启动配置驱动**：程序启动读取配置，根据 general/window/tray/debug 等段初始化
5. **旧配置兼容**：清理旧版 layout.rs 和 settings.rs 模块
6. **不实现导入/导出功能**（保留方法签名供后续使用）

## Acceptance Criteria

- [ ] 配置文件路径为 `~/.ikun_organizer.json`
- [ ] 首次运行自动生成默认配置文件
- [ ] JSON schema 包含所有 PRD 功能点对应的字段
- [ ] 所有结构体有 `Default` 实现和 `#[serde(default)]`
- [ ] `AppConfig::load()` 从用户主目录读取
- [ ] `AppConfig::save()` 写入用户主目录
- [ ] `AppContext::new()` 根据配置初始化各子系统
- [ ] 旧版 `layout.rs` 和 `settings.rs` 清理
- [ ] `cargo fmt` + `cargo clippy` 通过
- [ ] 配置加载失败时优雅降级到默认值

## Technical Notes

### 关键文件修改

| 文件 | 修改内容 |
|------|---------|
| `src/config/app_config.rs` | 重设计 schema、改路径、新增字段 |
| `src/config/mod.rs` | 清理旧模块导出 |
| `src/config/layout.rs` | 删除（功能合并到 app_config.rs） |
| `src/config/settings.rs` | 删除（功能合并到 app_config.rs） |
| `src/context.rs` | 根据配置初始化子系统 |
| `src/app.rs` | 根据 window 配置设置窗口行为 |
| `src/main.rs` | 根据配置初始化日志级别等 |

### 启动流程

```
main.rs
  1. init_logger() ← 从 DebugConfig 读取日志级别
  2. AppConfig::load() ← 从 ~/.ikun_organizer.json 读取
  3. AppContext::new(config) ← 注入配置，初始化各子系统
  4. 根据 window 配置设置 eframe::NativeOptions
  5. eframe::run_native()
```

### 数据流

```
~/.ikun_organizer.json
  → AppConfig::load()
    → AppContext.config
      → 各模块读取配置
      → 用户修改设置 → AppContext.save_config()
        → AppConfig::save() → ~/.ikun_organizer.json
```
