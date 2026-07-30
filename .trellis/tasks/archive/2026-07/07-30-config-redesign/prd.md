# brainstorm: 配置文件重新设计

## 目标
全面审查当前 `~/.ikun_organizer.json` 配置文件的所有字段，评估其合理性，并基于发现的问题重新设计配置文件结构。

## 已知信息
* 配置文件路径: `~/.ikun_organizer.json`（JSON 格式）
* 配置文件版本: `2.0`
* 当前有 **11 个顶层字段**，涵盖通用设置、热键、样式、AI、分类规则、布局、覆盖层、调试、UI 主题
* 代码使用 `serde` 进行序列化/反序列化，所有 struct 均带 `#[serde(default)]`
* 存在实际用户配置文件，已有 2 个分类框的布局数据

## 当前配置文件字段详细清单

### 顶层结构 (AppConfig)
| 字段 | 类型 | 说明 |
|------|------|------|
| `version` | String | 配置版本号 (当前 "2.0") |
| `last_modified` | DateTime<Utc> | 最后修改时间 |
| `general` | GeneralConfig | 通用行为设置 |
| `hotkeys` | HotkeyConfig | 全局热键 |
| `box_style` | BoxStyleConfig | 分类框默认样式 |
| `box_defaults` | BoxDefaultsConfig | 新建分类框默认值 |
| `ai` | AiConfig | AI 分类引擎配置 |
| `classification` | Vec<CategoryRule> | 默认分类规则 |
| `layout` | LayoutConfig | 分类框布局持久化 |
| `overlay` | OverlayConfig | 桌面覆盖窗口配置 |
| `debug` | DebugConfig | 调试和日志 |
| `ui_theme` | UiThemeConfig | UI 主题颜色 |

### GeneralConfig (通用设置)
| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `auto_start` | bool | false | 开机自启 |
| `language` | String | "zh-CN" | 界面语言 |
| `single_instance` | bool | true | 单实例模式 |
| `restore_layout_on_start` | bool | true | 启动时恢复布局 |
| `auto_classify_on_start` | bool | false | 启动时自动分类 |

### HotkeyConfig (热键)
| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `toggle_panel` | String | "Ctrl+Alt+D" | 切换面板热键 |
| `auto_classify` | String | "Ctrl+Alt+C" | 自动分类热键 |

### BoxStyleConfig (分类框样式 - 20个字段)
| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `background_color` | [u8;3] | [1,1,1] | 背景色 RGB |
| `background_alpha` | u8 | 38 | 背景透明度 |
| `border_radius` | f32 | 16.0 | 圆角半径 |
| `border_color` | [u8;4] | [100,100,100,100] | 边框色 RGBA |
| `border_width` | f32 | 1.0 | 边框宽度 |
| `title_font_size` | f32 | 15.0 | 标题字号 |
| `title_font_family` | String | "Microsoft YaHei UI" | 标题字体 |
| `title_font_color` | [u8;4] | [255,255,255,255] | 标题颜色 |
| `label_font_size` | f32 | 13.0 | 标签字号 |
| `label_font_color` | [u8;4] | [255,255,255,230] | 标签颜色 |
| `icon_size` | i32 | 64 | 图标大小 |
| `cell_spacing` | i32 | 54 | 单元格间距 |
| `title_bar_height` | i32 | 40 | 标题栏高度 |
| `title_bar_color` | [u8;4] | [30,30,30,200] | 标题栏颜色 |
| `highlight_color` | [u8;4] | [60,80,120,120] | 高亮颜色 |
| `hover_color` | [u8;4] | [70,90,130,70] | 悬停颜色 |
| `shadow_enabled` | bool | true | 是否启用阴影 |
| `shadow_color` | [u8;4] | [0,0,0,200] | 阴影颜色 |
| `columns` | i32 | 0 | 列数 (0=自动) |
| `lock_button_visible` | bool | true | 显示锁定按钮 |
| `close_button_visible` | bool | true | 显示关闭按钮 |

### BoxDefaultsConfig (新建框默认值)
| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `view_mode` | ViewMode | Medium | 视图模式 |
| `sort_mode` | SortMode | Name | 排序模式 |
| `sort_ascending` | bool | true | 升序排列 |
| `group_by` | GroupBy | None | 分组方式 |
| `default_width` | i32 | 680 | 默认宽度 |
| `default_height` | i32 | 800 | 默认高度 |

### AiConfig (AI 配置)
| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `enabled` | bool | false | 是否启用 |
| `base_url` | String | openai url | API 地址 |
| `api_key` | String | "" | API 密钥 |
| `model` | String | "gpt-4o-mini" | 模型名称 |
| `max_tokens` | u32 | 2048 | 最大 token |
| `temperature` | f32 | 0.7 | 温度参数 |
| `system_prompt` | String | ... | 系统提示词 |
| `timeout_seconds` | u32 | 30 | 超时时间 |
| `max_retries` | u32 | 3 | 重试次数 |

### CategoryRule (分类规则)
| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | String | 分类 ID |
| `name` | String | 显示名称 |
| `icon` | String | 图标 (emoji) |
| `exe_patterns` | Vec<String> | 可执行文件匹配模式 |
| `path_patterns` | Vec<String> | 路径匹配模式 |
| `keywords` | Vec<String> | 关键词匹配 |

### LayoutConfig (布局数据)
| 字段 | 类型 | 说明 |
|------|------|------|
| `boxes` | Vec<BoxLayoutData> | 所有分类框布局 |

### BoxLayoutData (单个框布局)
| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | Uuid | 唯一标识 |
| `name` | String | 框名称 |
| `position` | [f32;2] | 位置坐标 |
| `size` | [f32;2] | 尺寸 |
| `locked` | bool | 是否锁定 |
| `sort_mode` | SortMode | 排序模式 |
| `sort_ascending` | bool | 升序 |
| `view_mode` | ViewMode | 视图模式 |
| `group_by` | GroupBy | 分组 |
| `icon_names` | Vec<String> | 图标文件路径列表 |
| `display_names` | Vec<String> | 显示名称列表 |
| `grid_positions` | Vec<[i32;2]> | 网格位置 |
| `style_overrides` | Option<BoxStyleOverrides> | 样式覆盖 |

### BoxStyleOverrides (样式覆盖 - 13个可选字段)
与 BoxStyleConfig 部分字段相同，均为 Option<T> 类型

### OverlayConfig (覆盖层)
| 字段 | 类型 | 说明 |
|------|------|------|
| `icon_positions` | HashMap<String,[f32;2]> | 图标位置映射 |
| `icon_size` | i32 | 图标大小 (默认 64) |
| `layout_direction` | String | 布局方向 (默认 "left-to-right") |

### DebugConfig (调试)
| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `log_level` | String | "info" | 日志级别 |
| `log_to_file` | bool | false | 写入文件 |
| `log_file_path` | Option<String> | null | 日志文件路径 |
| `log_max_size_mb` | u32 | 10 | 最大文件大小 |
| `show_fps` | bool | false | 显示帧率 |
| `render_wireframe` | bool | false | 线框渲染 |

### UiThemeConfig (UI 主题)
| 字段 | 类型 | 说明 |
|------|------|------|
| `light` | ThemeColors | 亮色主题 |
| `dark` | ThemeColors | 暗色主题 |

### ThemeColors (主题颜色 - 8个字段)
| 字段 | 类型 | 说明 |
|------|------|------|
| `background` | Option<String> | 背景色 |
| `foreground` | Option<String> | 前景色 |
| `border` | Option<String> | 边框色 |
| `primary` | Option<String> | 主色 |
| `primary_foreground` | Option<String> | 主色前景 |
| `success` | Option<String> | 成功色 |
| `danger` | Option<String> | 危险色 |
| `muted_foreground` | Option<String> | 柔和前景色 |

### 枚举类型
- **SortMode**: Name, Type, ModifiedTime, Author, CreatedTime, Size, Tag, Title
- **ViewMode**: ExtraLarge, Large, Medium, Small, List, Detail, Tiles, Content
- **GroupBy**: None, Name, Type, ModifiedTime, Author, CreatedTime, Size, Tag, Title

## 发现的问题

### 1. 职责混乱
- `box_style` (样式) 和 `box_defaults` (行为默认值) 分开存储，但概念上紧密相关
- `layout` 中每个框存储了 sort_mode/view_mode/group_by，与 `box_defaults` 重复
- `overlay.icon_size` 与 `box_style.icon_size` 重复

### 2. 数据膨胀
- 布局数据 (`icon_names`, `display_names`, `grid_positions`) 混在配置文件中，随使用增长
- `ui_theme` 所有字段均为 null（未使用），占用无意义空间

### 3. 安全性
- `ai.api_key` 明文存储在 JSON 中，存在安全风险

### 4. 颜色表示不一致
- `box_style` 使用 `[u8;3]` / `[u8;4]` 数组
- `ui_theme` 使用 `Option<String>`（CSS 格式？）
- 两种颜色格式共存，增加复杂度

### 5. 配置 vs 状态 未分离
- 用户偏好设置（general, hotkeys, box_style）与运行时状态（layout, overlay）混在一起
- 配置变更频率差异大，但存储在同一文件

## 临时假设
* 用户可能需要配置迁移（旧配置自动升级）
* 可能需要将配置拆分为多个文件

## 待解决问题
* 仅保留阻断性偏好问题，保持简短

## 需求（演进中）
* 从已知信息开始

## 验收标准（演进中）
- [ ] <可测试的标准>

## 完成定义（团队质量门槛）
* 添加/更新测试（单元/集成）
* Lint / typecheck / CI 通过
* 行为变更时更新文档/笔记
* 考虑发布/回滚（如果有风险）

## 明确排除范围
* <本任务不做什么>

## 技术笔记
* 配置文件: `~/.ikun_organizer.json`
* 源码: `src/config/app_config.rs` (~700行)
* 序列化: serde + serde_json
* 所有 struct 使用 `#[serde(default)]` 保证向后兼容
