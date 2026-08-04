//! 统一应用配置

#![allow(dead_code)] // 配置类型，部分方法在当前构建路径未使用
//!
//! 所有设置集中在 `~/.ikun_organizer.json`，支持导入/导出。
//! 配置文件不存在时使用默认值并自动生成。
//!
//! # 配置段
//! - `general` — 通用行为（开机自启、语言、启动恢复）
//! - `hotkeys` — 全局热键
//! - `box_style` — 分类框默认样式
//! - `box_defaults` — 新建分类框默认值
//! - `ai` — AI 分类引擎
//! - `classification` — 默认分类规则（AI 失败时兜底）
//! - `layout` — 分类框布局持久化
//! - `debug` — 调试和日志

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 配置文件名（存放在用户主目录）
const CONFIG_FILE_NAME: &str = ".ikun_organizer.json";

// =============================================================================
// AppConfig — 顶层配置
// =============================================================================

/// 统一应用配置（对应 ~/.ikun_organizer.json）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 配置版本号
    pub version: String,
    /// 最后修改时间
    pub last_modified: chrono::DateTime<chrono::Utc>,
    /// 通用设置
    pub general: GeneralConfig,
    /// 全局热键
    pub hotkeys: HotkeyConfig,
    /// 分类框默认样式
    pub box_style: BoxStyleConfig,
    /// 新建分类框默认值
    pub box_defaults: BoxDefaultsConfig,
    /// AI 分类配置
    pub ai: AiConfig,
    /// 默认分类规则（AI 失败时兜底）
    pub classification: Vec<CategoryRule>,
    /// 分类框布局数据
    pub layout: LayoutConfig,
    /// 桌面覆盖窗口配置
    pub overlay: OverlayConfig,
    /// 调试配置
    pub debug: DebugConfig,
    /// UI 主题配置
    pub ui_theme: UiThemeConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: "2.0".to_string(),
            last_modified: chrono::Utc::now(),
            general: GeneralConfig::default(),
            hotkeys: HotkeyConfig::default(),
            box_style: BoxStyleConfig::default(),
            box_defaults: BoxDefaultsConfig::default(),
            ai: AiConfig::default(),
            classification: default_categories(),
            layout: LayoutConfig::default(),
            overlay: OverlayConfig::default(),
            debug: DebugConfig::default(),
            ui_theme: UiThemeConfig::default(),
        }
    }
}

impl AppConfig {
    /// 获取配置文件路径：`~/.ikun_organizer.json`
    pub fn get_config_path() -> anyhow::Result<PathBuf> {
        dirs::home_dir()
            .map(|p| p.join(CONFIG_FILE_NAME))
            .ok_or_else(|| anyhow::anyhow!("无法获取用户主目录"))
    }

    /// 加载配置（文件不存在则返回默认值并自动创建文件）
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::get_config_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("无法读取配置文件 {}: {}", path.display(), e))?;
            let mut config: AppConfig = serde_json::from_str(&content)
                .map_err(|e| anyhow::anyhow!("配置文件格式错误 {}: {}", path.display(), e))?;
            config.box_style.liquid_glass.normalize();
            log::info!("加载配置成功: {}", path.display());
            Ok(config)
        } else {
            log::info!("配置文件不存在，使用默认配置");
            let config = Self::default();
            // 首次运行自动生成默认配置文件
            if let Err(e) = config.save() {
                log::warn!("自动生成默认配置文件失败: {}", e);
            } else {
                log::info!("已生成默认配置文件: {}", path.display());
            }
            Ok(config)
        }
    }

    /// 保存配置到 `~/.ikun_organizer.json`
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::get_config_path()?;
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("序列化配置失败: {}", e))?;
        std::fs::write(&path, content)
            .map_err(|e| anyhow::anyhow!("写入配置文件失败 {}: {}", path.display(), e))?;
        log::info!("保存配置成功: {}", path.display());
        Ok(())
    }

    /// 导出配置到指定路径（复制当前配置文件）
    pub fn export_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("序列化配置失败: {}", e))?;
        std::fs::write(path, content).map_err(|e| anyhow::anyhow!("导出配置失败: {}", e))?;
        log::info!("导出配置到: {}", path.display());
        Ok(())
    }

    /// 从指定路径导入配置（覆盖当前配置）
    pub fn import_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let content =
            std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("读取配置失败: {}", e))?;
        let mut config: AppConfig = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("配置文件格式错误: {}", e))?;
        config.box_style.liquid_glass.normalize();
        log::info!("导入配置成功: {}", path.display());
        Ok(config)
    }
}

// =============================================================================
// GeneralConfig — 通用设置
// =============================================================================

/// 通用设置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// 是否开机自启动
    pub auto_start: bool,
    /// 界面语言（预留多语言）
    pub language: String,
    /// 是否只允许运行一个实例
    pub single_instance: bool,
    /// 启动时恢复上次分类框布局
    pub restore_layout_on_start: bool,
    /// 启动时自动执行一键分类
    pub auto_classify_on_start: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            language: "zh-CN".to_string(),
            single_instance: true,
            restore_layout_on_start: true,
            auto_classify_on_start: false,
        }
    }
}

// =============================================================================
// HotkeyConfig — 全局热键
// =============================================================================

/// 全局热键配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// 切换控制面板显示/隐藏
    pub toggle_panel: String,
    /// 执行一键智能分类
    pub auto_classify: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle_panel: "Ctrl+Alt+D".to_string(),
            auto_classify: "Ctrl+Alt+C".to_string(),
        }
    }
}

// =============================================================================
// BoxStyleConfig — 分类框默认样式
// =============================================================================

/// 分类框默认样式配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BoxStyleConfig {
    /// 背景颜色 RGB
    pub background_color: [u8; 3],
    /// 背景透明度 0-255
    pub background_alpha: u8,
    /// 圆角半径（像素）
    pub border_radius: f32,
    /// 边框颜色 RGBA
    pub border_color: [u8; 4],
    /// 边框宽度（像素）
    pub border_width: f32,
    /// 标题字体大小
    pub title_font_size: f32,
    /// 标题字体名称
    pub title_font_family: String,
    /// 标题文字颜色 RGBA
    pub title_font_color: [u8; 4],
    /// 标签字体大小
    pub label_font_size: f32,
    /// 标签文字颜色 RGBA
    pub label_font_color: [u8; 4],
    /// 图标尺寸（像素）
    pub icon_size: i32,
    /// 图标间距（像素）
    pub cell_spacing: i32,
    /// 标题栏高度（像素）
    pub title_bar_height: i32,
    /// 标题栏背景颜色 RGBA
    pub title_bar_color: [u8; 4],
    /// 选中项高亮颜色 RGBA
    pub highlight_color: [u8; 4],
    /// 悬停颜色 RGBA
    pub hover_color: [u8; 4],
    /// 是否显示窗口阴影
    pub shadow_enabled: bool,
    /// 阴影颜色 RGBA
    pub shadow_color: [u8; 4],
    /// 每行列数（0 = 自动计算）
    pub columns: i32,
    /// 是否显示锁定按钮
    pub lock_button_visible: bool,
    /// 是否显示关闭按钮
    pub close_button_visible: bool,
    /// 液态玻璃效果参数
    pub liquid_glass: LiquidGlassConfig,
}

impl Default for BoxStyleConfig {
    fn default() -> Self {
        Self {
            background_color: [1, 1, 1],
            background_alpha: 38,
            border_radius: 16.0,
            border_color: [100, 100, 100, 100],
            border_width: 1.0,
            title_font_size: 15.0,
            title_font_family: "Microsoft YaHei UI".to_string(),
            title_font_color: [255, 255, 255, 255],
            label_font_size: 13.0,
            label_font_color: [255, 255, 255, 230],
            icon_size: 64,
            cell_spacing: 54,
            title_bar_height: 40,
            title_bar_color: [30, 30, 30, 200],
            highlight_color: [60, 80, 120, 120],
            hover_color: [70, 90, 130, 70],
            shadow_enabled: true,
            shadow_color: [0, 0, 0, 200],
            columns: 0,
            lock_button_visible: true,
            close_button_visible: true,
            liquid_glass: LiquidGlassConfig::default(),
        }
    }
}

// =============================================================================
// LiquidGlassConfig - 液态玻璃渲染参数（对齐 liquid-glass-rs EffectStyle）
// =============================================================================

/// 液态玻璃圆角半径的控制面板与渲染上限。
pub const LIQUID_GLASS_CORNER_RADIUS_MAX: f32 = 30.0;

/// 液态玻璃渲染参数，对齐 liquid-glass-rs FrostedLiquid 示例的 EffectStyle。
///
/// - 滑块参数通过 LiquidGlassParam 枚举访问
/// - enabled 和 over_light 为 bool，通过独立方法访问
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LiquidGlassConfig {
    /// 液态玻璃效果开关
    pub enabled: bool,
    /// 位移强度（示例滑块 0~200），默认 100.0
    pub displacement_scale: f32,
    /// 模糊程度（示例滑块 0~1），默认 0.5
    pub blur_amount: f32,
    /// 饱和度（示例滑块 1~3），默认 1.40
    pub saturation: f32,
    /// 色差偏移（示例滑块 0~20），默认 2.0
    pub chromatic_offset: f32,
    /// 圆角半径（控制面板范围 0~30），默认 30.0
    pub corner_radius: f32,
    /// 弹性形变系数（示例滑块 0~1），默认 0.0
    pub elasticity: f32,
    /// 法线贴图模式：0=标准, 1=极坐标, 2=凸起, 3=程序化
    pub refraction_mode: u32,
    /// 亮区减弱折射（在明亮区域降低位移强度）
    pub over_light: bool,
}

impl Default for LiquidGlassConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            displacement_scale: 100.0,
            blur_amount: 0.5,
            saturation: 1.40,
            chromatic_offset: 2.0,
            corner_radius: LIQUID_GLASS_CORNER_RADIUS_MAX,
            elasticity: 0.0,
            refraction_mode: 0,
            over_light: false,
        }
    }
}

/// 液态玻璃参数标识（bool 字段 enabled/over_light 通过独立方法访问）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiquidGlassParam {
    DisplacementScale,
    BlurAmount,
    Saturation,
    ChromaticOffset,
    CornerRadius,
    Elasticity,
    RefractionMode,
}

impl LiquidGlassConfig {
    /// 将持久化或外部传入的配置收敛到当前支持范围。
    pub(crate) fn normalize(&mut self) {
        let corner_radius = self.corner_radius;
        self.set_param(LiquidGlassParam::CornerRadius, corner_radius);
    }

    /// 设置液态玻璃参数
    pub fn set_param(&mut self, param: LiquidGlassParam, value: f32) {
        match param {
            LiquidGlassParam::DisplacementScale => {
                self.displacement_scale = value.clamp(0.0, 200.0)
            }
            LiquidGlassParam::BlurAmount => self.blur_amount = value.clamp(0.0, 1.0),
            LiquidGlassParam::Saturation => self.saturation = value.clamp(1.0, 3.0),
            LiquidGlassParam::ChromaticOffset => self.chromatic_offset = value.clamp(0.0, 20.0),
            LiquidGlassParam::CornerRadius => {
                self.corner_radius = value.clamp(0.0, LIQUID_GLASS_CORNER_RADIUS_MAX)
            }
            LiquidGlassParam::Elasticity => self.elasticity = value.clamp(0.0, 1.0),
            LiquidGlassParam::RefractionMode => {
                self.refraction_mode = value.round().clamp(0.0, 3.0) as u32
            }
        }
    }

    /// 获取液态玻璃参数
    pub fn get_param(&self, param: LiquidGlassParam) -> f32 {
        match param {
            LiquidGlassParam::DisplacementScale => self.displacement_scale,
            LiquidGlassParam::BlurAmount => self.blur_amount,
            LiquidGlassParam::Saturation => self.saturation,
            LiquidGlassParam::ChromaticOffset => self.chromatic_offset,
            LiquidGlassParam::CornerRadius => self.corner_radius,
            LiquidGlassParam::Elasticity => self.elasticity,
            LiquidGlassParam::RefractionMode => self.refraction_mode as f32,
        }
    }
}

// =============================================================================
// BoxDefaultsConfig — 新建分类框默认值
// =============================================================================

/// 新建分类框的默认值配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BoxDefaultsConfig {
    /// 默认视图模式
    pub view_mode: ViewMode,
    /// 默认排序方式
    pub sort_mode: SortMode,
    /// 默认是否升序
    pub sort_ascending: bool,
    /// 默认分组方式
    pub group_by: GroupBy,
    /// 默认宽度（像素）
    pub default_width: i32,
    /// 默认高度（像素）
    pub default_height: i32,
}

impl Default for BoxDefaultsConfig {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::Medium,
            sort_mode: SortMode::Name,
            sort_ascending: true,
            group_by: GroupBy::None,
            default_width: 680,
            default_height: 800,
        }
    }
}

// =============================================================================
// AiConfig — AI 分类引擎
// =============================================================================

/// AI 分类引擎配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    /// 是否启用 AI 分类
    pub enabled: bool,
    /// API 基础地址
    pub base_url: String,
    /// API 密钥
    pub api_key: String,
    /// 模型名称
    pub model: String,
    /// 最大输出 token 数
    pub max_tokens: u32,
    /// 生成温度 0-1
    pub temperature: f32,
    /// 系统提示词
    pub system_prompt: String,
    /// API 请求超时秒数
    pub timeout_seconds: u32,
    /// 请求失败最大重试次数
    pub max_retries: u32,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            max_tokens: 2048,
            temperature: 0.7,
            system_prompt: "你是一个桌面图标分类助手，请根据应用名称和路径判断其分类。".to_string(),
            timeout_seconds: 30,
            max_retries: 3,
        }
    }
}

// =============================================================================
// CategoryRule — 分类规则（兜底）
// =============================================================================

/// 分类规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryRule {
    /// 分类唯一标识
    pub id: String,
    /// 分类显示名称
    pub name: String,
    /// 分类图标（emoji）
    pub icon: String,
    /// 可执行文件名匹配模式（支持通配符）
    pub exe_patterns: Vec<String>,
    /// 路径包含匹配模式
    pub path_patterns: Vec<String>,
    /// 关键词匹配
    pub keywords: Vec<String>,
}

/// 默认分类规则列表
pub fn default_categories() -> Vec<CategoryRule> {
    vec![
        CategoryRule {
            id: "games".into(),
            name: "游戏".into(),
            icon: "🎮".into(),
            exe_patterns: vec![
                "steam.exe".into(),
                "epicgameslauncher.exe".into(),
                "lol.exe".into(),
            ],
            path_patterns: vec!["\\Games\\".into(), "\\Steam\\".into()],
            keywords: vec!["game".into(), "play".into(), "游戏".into()],
        },
        CategoryRule {
            id: "dev".into(),
            name: "开发工具".into(),
            icon: "💻".into(),
            exe_patterns: vec!["code.exe".into(), "idea*.exe".into(), "rust*.exe".into()],
            path_patterns: vec!["\\Dev\\".into(), "\\IDE\\".into()],
            keywords: vec!["dev".into(), "code".into(), "ide".into()],
        },
        CategoryRule {
            id: "office".into(),
            name: "办公".into(),
            icon: "📄".into(),
            exe_patterns: vec![
                "word.exe".into(),
                "excel.exe".into(),
                "powerpoint.exe".into(),
            ],
            path_patterns: vec!["\\Office\\".into()],
            keywords: vec!["office".into(), "document".into(), "办公".into()],
        },
        CategoryRule {
            id: "media".into(),
            name: "媒体".into(),
            icon: "🎵".into(),
            exe_patterns: vec!["vlc.exe".into(), "spotify.exe".into()],
            path_patterns: vec!["\\Music\\".into(), "\\Video\\".into()],
            keywords: vec!["music".into(), "video".into(), "媒体".into()],
        },
        CategoryRule {
            id: "tools".into(),
            name: "工具".into(),
            icon: "🔧".into(),
            exe_patterns: vec!["notepad.exe".into(), "calc.exe".into()],
            path_patterns: vec!["\\Tools\\".into()],
            keywords: vec!["tool".into(), "utility".into(), "工具".into()],
        },
    ]
}

// =============================================================================
// LayoutConfig — 分类框布局持久化
// =============================================================================

/// 分类框布局数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LayoutConfig {
    /// 分类框列表
    pub boxes: Vec<BoxLayoutData>,
}

/// 单个分类框的布局数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BoxLayoutData {
    /// 分类框唯一 ID
    pub id: uuid::Uuid,
    /// 显示名称
    pub name: String,
    /// 窗口位置 [x, y]
    pub position: [f32; 2],
    /// 窗口尺寸 [宽, 高]
    pub size: [f32; 2],
    /// 是否锁定（锁定后不可拖拽/缩放）
    pub locked: bool,
    /// 排序方式
    pub sort_mode: SortMode,
    /// 是否升序
    pub sort_ascending: bool,
    /// 视图模式
    pub view_mode: ViewMode,
    /// 分组方式
    pub group_by: GroupBy,
    /// 包含的图标文件名列表（文件路径或 CLSID）
    pub icon_names: Vec<String>,
    /// 对应的显示名称列表（与 icon_names 一一对应）
    #[serde(default)]
    pub display_names: Vec<String>,
    /// 每个图标的网格位置 [col, row]，-1 表示自动布局（与 icon_names 一一对应）
    #[serde(default)]
    pub grid_positions: Vec<[i32; 2]>,
    /// 此框样式覆盖（None = 使用全局 box_style 默认值）
    pub style_overrides: Option<BoxStyleOverrides>,
}

impl Default for BoxLayoutData {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            name: "新分类框".to_string(),
            position: [100.0, 100.0],
            size: [680.0, 800.0],
            locked: false,
            sort_mode: SortMode::default(),
            sort_ascending: true,
            view_mode: ViewMode::default(),
            group_by: GroupBy::default(),
            icon_names: Vec::new(),
            display_names: Vec::new(),
            grid_positions: Vec::new(),
            style_overrides: None,
        }
    }
}

// =============================================================================
// BoxStyleOverrides — 每框样式覆盖
// =============================================================================

/// 分类框样式覆盖（字段均为可选，None 时使用全局 box_style 默认值）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BoxStyleOverrides {
    /// 背景颜色 RGB
    pub background_color: Option<[u8; 3]>,
    /// 背景透明度 0-255
    pub background_alpha: Option<u8>,
    /// 圆角半径
    pub border_radius: Option<f32>,
    /// 边框颜色 RGBA
    pub border_color: Option<[u8; 4]>,
    /// 边框宽度
    pub border_width: Option<f32>,
    /// 标题字体大小
    pub title_font_size: Option<f32>,
    /// 标签字体大小
    pub label_font_size: Option<f32>,
    /// 图标尺寸
    pub icon_size: Option<i32>,
    /// 图标间距
    pub cell_spacing: Option<i32>,
    /// 标题栏高度
    pub title_bar_height: Option<i32>,
    /// 标题栏背景颜色 RGBA
    pub title_bar_color: Option<[u8; 4]>,
    /// 高亮颜色 RGBA
    pub highlight_color: Option<[u8; 4]>,
    /// 悬停颜色 RGBA
    pub hover_color: Option<[u8; 4]>,
    /// 阴影开关
    pub shadow_enabled: Option<bool>,
    /// 每行列数
    pub columns: Option<i32>,
}

// =============================================================================
// OverlayConfig — 桌面覆盖窗口配置
// =============================================================================

/// 桌面覆盖窗口配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    /// 非管理图标位置映射：文件路径 → [x, y]（逻辑像素，96 DPI 基准）。
    pub icon_positions: HashMap<String, [f32; 2]>,
    /// 覆盖窗口图标尺寸。
    pub icon_size: i32,
    /// 排列方向：right-to-left / left-to-right。
    pub layout_direction: String,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            icon_positions: HashMap::new(),
            icon_size: 64,
            layout_direction: "left-to-right".to_string(),
        }
    }
}

// =============================================================================
// DebugConfig — 调试配置
// =============================================================================

/// 调试配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DebugConfig {
    /// 日志级别：trace / debug / info / warn / error
    pub log_level: String,
    /// 是否写入日志文件
    pub log_to_file: bool,
    /// 日志文件路径（None = 默认位置）
    pub log_file_path: Option<String>,
    /// 日志文件最大 MB
    pub log_max_size_mb: u32,
    /// 是否显示帧率
    pub show_fps: bool,
    /// 是否渲染线框模式
    pub render_wireframe: bool,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            log_to_file: false,
            log_file_path: None,
            log_max_size_mb: 10,
            show_fps: false,
            render_wireframe: false,
        }
    }
}

// =============================================================================
// 枚举类型
// =============================================================================

/// 排序模式
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    /// 按名称
    #[default]
    Name,
    /// 按类型
    Type,
    /// 按修改时间
    ModifiedTime,
    /// 按作者
    Author,
    /// 按创建时间
    CreatedTime,
    /// 按大小
    Size,
    /// 按标记
    Tag,
    /// 按标题
    Title,
}

/// 视图模式
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    /// 超大图标
    ExtraLarge,
    /// 大图标
    Large,
    /// 中等图标
    #[default]
    Medium,
    /// 小图标
    Small,
    /// 列表
    List,
    /// 详细信息
    Detail,
    /// 平铺
    Tiles,
    /// 内容
    Content,
}

/// 分组方式
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    /// 不分组
    #[default]
    None,
    /// 按名称
    Name,
    /// 按类型
    Type,
    /// 按修改时间
    ModifiedTime,
    /// 按作者
    Author,
    /// 按创建时间
    CreatedTime,
    /// 按大小
    Size,
    /// 按标记
    Tag,
    /// 按标题
    Title,
}

// =============================================================================
// UI 主题配置
// =============================================================================

/// UI 主题配置（支持 light/dark 切换）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiThemeConfig {
    /// 亮色主题颜色
    pub light: ThemeColors,
    /// 暗色主题颜色
    pub dark: ThemeColors,
}

/// 主题颜色集合
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeColors {
    /// 背景色 (#RRGGBB 或 #RRGGBBAA)
    pub background: Option<String>,
    /// 前景色
    pub foreground: Option<String>,
    /// 边框色
    pub border: Option<String>,
    /// 主色
    pub primary: Option<String>,
    /// 主色前景
    pub primary_foreground: Option<String>,
    /// 成功色
    pub success: Option<String>,
    /// 危险色
    pub danger: Option<String>,
    /// 弱化前景色
    pub muted_foreground: Option<String>,
}

impl Default for UiThemeConfig {
    fn default() -> Self {
        Self {
            light: ThemeColors::default(),
            dark: ThemeColors::default(),
        }
    }
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            background: None,
            foreground: None,
            border: None,
            primary: None,
            primary_foreground: None,
            success: None,
            danger: None,
            muted_foreground: None,
        }
    }
}

// =============================================================================
// 单元测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Default 值验证 ----

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.version, "2.0");
        assert_eq!(config.general.language, "zh-CN");
        assert!(!config.general.auto_start);
        assert!(config.general.single_instance);
        assert!(config.general.restore_layout_on_start);
        assert!(!config.general.auto_classify_on_start);
    }

    #[test]
    fn test_box_style_default() {
        let style = BoxStyleConfig::default();
        assert_eq!(style.background_alpha, 38);
        assert_eq!(style.border_radius, 16.0);
        assert_eq!(style.icon_size, 64);
        assert_eq!(style.columns, 0);
        assert!(style.lock_button_visible);
        assert!(style.close_button_visible);
        assert!(style.shadow_enabled);
    }

    #[test]
    fn test_box_defaults_default() {
        let defaults = BoxDefaultsConfig::default();
        assert_eq!(defaults.view_mode, ViewMode::Medium);
        assert_eq!(defaults.sort_mode, SortMode::Name);
        assert!(defaults.sort_ascending);
        assert_eq!(defaults.group_by, GroupBy::None);
        assert_eq!(defaults.default_width, 680);
        assert_eq!(defaults.default_height, 800);
    }

    #[test]
    fn test_hotkey_default() {
        let hotkeys = HotkeyConfig::default();
        assert_eq!(hotkeys.toggle_panel, "Ctrl+Alt+D");
        assert_eq!(hotkeys.auto_classify, "Ctrl+Alt+C");
    }

    #[test]
    fn test_ai_default() {
        let ai = AiConfig::default();
        assert!(!ai.enabled);
        assert_eq!(ai.model, "gpt-4o-mini");
        assert_eq!(ai.timeout_seconds, 30);
        assert_eq!(ai.max_retries, 3);
    }

    #[test]
    fn test_debug_default() {
        let debug = DebugConfig::default();
        assert_eq!(debug.log_level, "info");
        assert!(!debug.log_to_file);
        assert!(!debug.show_fps);
    }

    #[test]
    fn test_default_categories_not_empty() {
        let categories = default_categories();
        assert!(categories.len() >= 3, "至少应有 3 个默认分类");
        let ids: Vec<&str> = categories.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"games"));
        assert!(ids.contains(&"dev"));
        assert!(ids.contains(&"office"));
    }

    // ---- 枚举序列化格式 ----

    #[test]
    fn test_sort_mode_serde_snake_case() {
        let mode = SortMode::ModifiedTime;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"modified_time\"");

        let parsed: SortMode = serde_json::from_str("\"created_time\"").unwrap();
        assert_eq!(parsed, SortMode::CreatedTime);
    }

    #[test]
    fn test_view_mode_serde_snake_case() {
        let mode = ViewMode::ExtraLarge;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"extra_large\"");

        let parsed: ViewMode = serde_json::from_str("\"detail\"").unwrap();
        assert_eq!(parsed, ViewMode::Detail);
    }

    #[test]
    fn test_group_by_serde_snake_case() {
        let group = GroupBy::None;
        let json = serde_json::to_string(&group).unwrap();
        assert_eq!(json, "\"none\"");
    }

    // ---- 序列化/反序列化往返 ----

    #[test]
    fn test_app_config_roundtrip() {
        let config = AppConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, config.version);
        assert_eq!(parsed.general.language, config.general.language);
        assert_eq!(parsed.box_style.icon_size, config.box_style.icon_size);
        assert_eq!(parsed.ai.model, config.ai.model);
        assert_eq!(parsed.classification.len(), config.classification.len());
        assert_eq!(parsed.overlay.icon_size, config.overlay.icon_size);
        assert_eq!(
            parsed.overlay.layout_direction,
            config.overlay.layout_direction
        );
    }

    #[test]
    fn test_overlay_config_default() {
        let overlay = OverlayConfig::default();
        assert!(overlay.icon_positions.is_empty());
        assert_eq!(overlay.icon_size, 64);
        assert_eq!(overlay.layout_direction, "left-to-right");
    }

    #[test]
    fn test_overlay_config_roundtrip_preserves_icon_positions() {
        let mut config = AppConfig::default();
        config.overlay.icon_positions.insert(
            r"C:\Users\chenzhihui\Desktop\app.lnk".to_string(),
            [120.0, 240.0],
        );
        config.overlay.icon_size = 64;
        config.overlay.layout_direction = "right-to-left".to_string();

        let json = serde_json::to_string(&config).unwrap();
        let parsed: AppConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.overlay.icon_size, 64);
        assert_eq!(parsed.overlay.layout_direction, "right-to-left");
        assert_eq!(
            parsed
                .overlay
                .icon_positions
                .get(r"C:\Users\chenzhihui\Desktop\app.lnk"),
            Some(&[120.0, 240.0])
        );
    }

    #[test]
    fn test_partial_json_uses_overlay_default() {
        let json = r#"{"version": "3.0", "layout": {"boxes": []}}"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();

        assert!(config.overlay.icon_positions.is_empty());
        assert_eq!(config.overlay.icon_size, 64);
        assert_eq!(config.overlay.layout_direction, "left-to-right");
    }

    #[test]
    fn test_box_layout_data_roundtrip() {
        let layout = BoxLayoutData {
            id: uuid::Uuid::new_v4(),
            name: "测试框".to_string(),
            position: [100.0, 200.0],
            size: [680.0, 800.0],
            locked: true,
            sort_mode: SortMode::Type,
            sort_ascending: false,
            view_mode: ViewMode::Large,
            group_by: GroupBy::Name,
            icon_names: vec!["a.exe".to_string(), "b.lnk".to_string()],
            display_names: vec!["程序A".to_string(), "程序B".to_string()],
            grid_positions: Vec::new(),
            style_overrides: None,
        };

        let json = serde_json::to_string(&layout).unwrap();
        let parsed: BoxLayoutData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "测试框");
        assert_eq!(parsed.position, [100.0, 200.0]);
        assert_eq!(parsed.size, [680.0, 800.0]);
        assert!(parsed.locked);
        assert_eq!(parsed.sort_mode, SortMode::Type);
        assert!(!parsed.sort_ascending);
        assert_eq!(parsed.view_mode, ViewMode::Large);
        assert_eq!(parsed.group_by, GroupBy::Name);
        assert_eq!(parsed.icon_names.len(), 2);
    }

    // ---- 部分 JSON 容错（serde default）----

    #[test]
    fn test_partial_json_falls_back_to_default() {
        let json = r#"{"version": "3.0"}"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.version, "3.0");
        assert_eq!(config.general.language, "zh-CN");
        assert_eq!(config.box_style.icon_size, 64);
    }

    #[test]
    fn test_partial_general_falls_back() {
        let json = r#"{"auto_start": true}"#;
        let general: GeneralConfig = serde_json::from_str(json).unwrap();
        assert!(general.auto_start);
        assert_eq!(general.language, "zh-CN");
        assert!(general.single_instance);
    }

    #[test]
    fn test_empty_json_uses_all_defaults() {
        let json = "{}";
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.version, "2.0");
        assert_eq!(config.general.language, "zh-CN");
        assert_eq!(config.box_style.border_radius, 16.0);
    }

    // ---- 配置文件路径 ----

    #[test]
    fn test_config_path_in_home_dir() {
        let path = AppConfig::get_config_path().unwrap();
        let home = dirs::home_dir().unwrap();
        assert_eq!(path, home.join(".ikun_organizer.json"));
    }

    // ---- 导出/导入 ----

    #[test]
    fn test_export_import_roundtrip() {
        let config = AppConfig::default();
        let dir = std::env::temp_dir().join("ikun_test_export");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_export.json");

        config.export_to(&path).unwrap();
        let imported = AppConfig::import_from(&path).unwrap();

        assert_eq!(imported.version, config.version);
        assert_eq!(imported.general.language, config.general.language);
        assert_eq!(imported.classification.len(), config.classification.len());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- BoxStyleOverrides ----

    #[test]
    fn test_style_overrides_partial_deserialization() {
        let json = r#"{"icon_size": 64, "border_radius": 20.0}"#;
        let overrides: BoxStyleOverrides = serde_json::from_str(json).unwrap();
        assert_eq!(overrides.icon_size, Some(64));
        assert_eq!(overrides.border_radius, Some(20.0));
        assert!(overrides.background_alpha.is_none());
    }

    // ---- UUID 精确删除验证 ----

    #[test]
    fn test_box_layout_retain_by_uuid_removes_exact_one() {
        let id_a = uuid::Uuid::new_v4();
        let id_b = uuid::Uuid::new_v4();
        let id_c = uuid::Uuid::new_v4();

        let mut layout = LayoutConfig {
            boxes: vec![
                BoxLayoutData {
                    id: id_a,
                    name: "A".into(),
                    ..Default::default()
                },
                BoxLayoutData {
                    id: id_b,
                    name: "B".into(),
                    ..Default::default()
                },
                BoxLayoutData {
                    id: id_c,
                    name: "C".into(),
                    ..Default::default()
                },
            ],
        };

        // 按 B 的 UUID 删除，应该只移除 B
        layout.boxes.retain(|b| b.id != id_b);
        assert_eq!(layout.boxes.len(), 2);
        assert_eq!(layout.boxes[0].name, "A");
        assert_eq!(layout.boxes[0].id, id_a);
        assert_eq!(layout.boxes[1].name, "C");
        assert_eq!(layout.boxes[1].id, id_c);
    }

    #[test]
    fn test_box_layout_retain_by_uuid_no_match_keeps_all() {
        let id_a = uuid::Uuid::new_v4();
        let id_b = uuid::Uuid::new_v4();
        let unknown_id = uuid::Uuid::new_v4();

        let mut layout = LayoutConfig {
            boxes: vec![
                BoxLayoutData {
                    id: id_a,
                    name: "A".into(),
                    ..Default::default()
                },
                BoxLayoutData {
                    id: id_b,
                    name: "B".into(),
                    ..Default::default()
                },
            ],
        };

        // 不存在的 UUID 不应删除任何条目
        layout.boxes.retain(|b| b.id != unknown_id);
        assert_eq!(layout.boxes.len(), 2);
    }

    #[test]
    fn test_box_layout_retain_by_uuid_duplicate_name_different_id() {
        // 验证：即使两个 box 同名，UUID 删除也只删一个
        let id_a = uuid::Uuid::new_v4();
        let id_b = uuid::Uuid::new_v4();

        let mut layout = LayoutConfig {
            boxes: vec![
                BoxLayoutData {
                    id: id_a,
                    name: "同名".into(),
                    ..Default::default()
                },
                BoxLayoutData {
                    id: id_b,
                    name: "同名".into(),
                    ..Default::default()
                },
            ],
        };

        // 按 id_b 删除，应该只删第二个
        layout.boxes.retain(|b| b.id != id_b);
        assert_eq!(layout.boxes.len(), 1);
        assert_eq!(layout.boxes[0].id, id_a);
        assert_eq!(layout.boxes[0].name, "同名");
    }

    #[test]
    fn test_box_layout_data_display_names_serialization() {
        let layout = BoxLayoutData {
            id: uuid::Uuid::new_v4(),
            name: "测试框".to_string(),
            position: [100.0, 200.0],
            size: [680.0, 800.0],
            locked: false,
            sort_mode: SortMode::Name,
            sort_ascending: true,
            view_mode: ViewMode::Medium,
            group_by: GroupBy::None,
            icon_names: vec![
                "C:\\Users\\Desktop\\test.lnk".to_string(),
                "::{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}".to_string(),
            ],
            display_names: vec!["测试".to_string(), "控制面板".to_string()],
            grid_positions: Vec::new(),
            style_overrides: None,
        };

        let json = serde_json::to_string(&layout).unwrap();
        let parsed: BoxLayoutData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.display_names.len(), 2);
        assert_eq!(parsed.display_names[0], "测试");
        assert_eq!(parsed.display_names[1], "控制面板");
    }

    #[test]
    fn test_box_layout_data_display_names_backward_compat() {
        // 旧配置文件没有 display_names 字段，应该使用默认空数组
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "旧框",
            "position": [100.0, 200.0],
            "size": [680.0, 800.0],
            "locked": false,
            "sort_mode": "name",
            "sort_ascending": true,
            "view_mode": "medium",
            "group_by": "none",
            "icon_names": ["test.lnk"],
            "style_overrides": null
        }"#;

        let parsed: BoxLayoutData = serde_json::from_str(json).unwrap();
        assert!(
            parsed.display_names.is_empty(),
            "旧配置应默认 display_names 为空"
        );
        assert_eq!(parsed.icon_names.len(), 1);
    }
}

// =============================================================================
// LiquidGlassConfig ??
// =============================================================================

#[cfg(test)]
mod liquid_glass_tests {
    use super::*;

    #[test]
    fn test_liquid_glass_config_default_values() {
        let config = LiquidGlassConfig::default();
        assert!(config.enabled);
        assert!((config.displacement_scale - 100.0).abs() < f32::EPSILON);
        assert!((config.blur_amount - 0.5).abs() < f32::EPSILON);
        assert!((config.saturation - 1.40).abs() < f32::EPSILON);
        assert!((config.chromatic_offset - 2.0).abs() < f32::EPSILON);
        assert!((config.corner_radius - LIQUID_GLASS_CORNER_RADIUS_MAX).abs() < f32::EPSILON);
        assert!((config.elasticity - 0.0).abs() < f32::EPSILON);
        assert_eq!(config.refraction_mode, 0);
        assert!(!config.over_light);
    }

    #[test]
    fn test_liquid_glass_config_set_and_get_param() {
        let mut config = LiquidGlassConfig::default();

        config.set_param(LiquidGlassParam::DisplacementScale, 150.0);
        assert!(
            (config.get_param(LiquidGlassParam::DisplacementScale) - 150.0).abs() < f32::EPSILON
        );

        config.set_param(LiquidGlassParam::BlurAmount, 0.75);
        assert!((config.get_param(LiquidGlassParam::BlurAmount) - 0.75).abs() < f32::EPSILON);

        config.set_param(LiquidGlassParam::Saturation, 2.5);
        assert!((config.get_param(LiquidGlassParam::Saturation) - 2.5).abs() < f32::EPSILON);

        config.set_param(LiquidGlassParam::ChromaticOffset, 5.0);
        assert!((config.get_param(LiquidGlassParam::ChromaticOffset) - 5.0).abs() < f32::EPSILON);

        config.set_param(LiquidGlassParam::CornerRadius, 24.0);
        assert!((config.get_param(LiquidGlassParam::CornerRadius) - 24.0).abs() < f32::EPSILON);

        config.set_param(LiquidGlassParam::Elasticity, 0.5);
        assert!((config.get_param(LiquidGlassParam::Elasticity) - 0.5).abs() < f32::EPSILON);

        config.set_param(LiquidGlassParam::RefractionMode, 2.0);
        assert!((config.get_param(LiquidGlassParam::RefractionMode) - 2.0).abs() < f32::EPSILON);
        assert_eq!(config.refraction_mode, 2);
    }

    #[test]
    fn test_liquid_glass_config_bool_accessors() {
        let mut config = LiquidGlassConfig::default();

        // enabled defaults to true
        assert!(config.enabled);
        config.enabled = false;
        assert!(!config.enabled);

        // over_light defaults to false
        assert!(!config.over_light);
        config.over_light = true;
        assert!(config.over_light);
    }

    #[test]
    fn test_liquid_glass_param_ranges_match_example_controls() {
        let mut config = LiquidGlassConfig::default();
        config.set_param(LiquidGlassParam::BlurAmount, 9.0);
        assert_eq!(config.blur_amount, 1.0);
        config.set_param(LiquidGlassParam::Saturation, -1.0);
        assert_eq!(config.saturation, 1.0);
        config.set_param(LiquidGlassParam::ChromaticOffset, 99.0);
        assert_eq!(config.chromatic_offset, 20.0);
        config.set_param(LiquidGlassParam::CornerRadius, 99.0);
        assert_eq!(config.corner_radius, LIQUID_GLASS_CORNER_RADIUS_MAX);
        config.set_param(LiquidGlassParam::RefractionMode, 99.0);
        assert_eq!(config.refraction_mode, 3);
    }

    #[test]
    fn test_liquid_glass_normalizes_legacy_corner_radius() {
        let mut config = LiquidGlassConfig {
            corner_radius: 100.0,
            ..LiquidGlassConfig::default()
        };

        config.normalize();

        assert_eq!(config.corner_radius, LIQUID_GLASS_CORNER_RADIUS_MAX);
    }

    #[test]
    fn test_liquid_glass_config_serialization() {
        let config = LiquidGlassConfig::default();
        let json = serde_json::to_string(&config).expect("Failed to serialize");

        assert!(json.contains("enabled"));
        assert!(json.contains("displacement_scale"));
        assert!(json.contains("blur_amount"));
        assert!(json.contains("saturation"));
        assert!(json.contains("chromatic_offset"));
        assert!(json.contains("corner_radius"));
        assert!(json.contains("elasticity"));
        assert!(json.contains("refraction_mode"));
        assert!(json.contains("over_light"));

        let deserialized: LiquidGlassConfig =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.enabled, config.enabled);
        assert!((deserialized.displacement_scale - config.displacement_scale).abs() < f32::EPSILON);
        assert!((deserialized.blur_amount - config.blur_amount).abs() < f32::EPSILON);
        assert!((deserialized.saturation - config.saturation).abs() < f32::EPSILON);
        assert_eq!(deserialized.refraction_mode, config.refraction_mode);
        assert_eq!(deserialized.over_light, config.over_light);
    }

    #[test]
    fn test_liquid_glass_config_backward_compatibility() {
        // Old config files with legacy fields should deserialize with new defaults
        let old_json = r#"{
            "blur_radius": 2.8,
            "refraction_strength": 7.0,
            "glass_alpha": 0.88,
            "edge_effect_width": 26.0,
            "chromatic_offset": 1.2,
            "contrast": 1.04,
            "brightness": 0.015,
            "shadow_offset_y": 7.0,
            "shadow_softness": 9.0,
            "shadow_opacity": 0.28,
            "top_light_position": 0.55,
            "top_light_strength": 0.16,
            "lower_shadow_position": 0.35,
            "lower_shadow_strength": 0.10,
            "color_gain": 1.025
        }"#;

        let config: LiquidGlassConfig =
            serde_json::from_str(old_json).expect("Failed to deserialize old config");

        // All new fields should have default values since old JSON doesn't contain them
        assert!(config.enabled);
        assert!((config.displacement_scale - 100.0).abs() < f32::EPSILON);
        assert!((config.blur_amount - 0.5).abs() < f32::EPSILON);
        assert!((config.saturation - 1.40).abs() < f32::EPSILON);
        assert_eq!(config.refraction_mode, 0);
        assert!(!config.over_light);
    }

    #[test]
    fn test_liquid_glass_param_enum_coverage() {
        let mut config = LiquidGlassConfig::default();

        let params = [
            (LiquidGlassParam::DisplacementScale, 120.0),
            (LiquidGlassParam::BlurAmount, 0.75),
            (LiquidGlassParam::Saturation, 2.0),
            (LiquidGlassParam::ChromaticOffset, 3.0),
            (
                LiquidGlassParam::CornerRadius,
                LIQUID_GLASS_CORNER_RADIUS_MAX,
            ),
            (LiquidGlassParam::Elasticity, 0.3),
            (LiquidGlassParam::RefractionMode, 1.0),
        ];

        for (param, value) in params.iter() {
            config.set_param(*param, *value);
            assert!(
                (config.get_param(*param) - *value).abs() < f32::EPSILON,
                "Failed for param {:?}, expected {}, got {}",
                param,
                value,
                config.get_param(*param)
            );
        }
    }

    // ===== 架构自检新增测试：LiquidGlassConfig 参数钳位边界 =====

    #[test]
    fn test_set_param_clamps_negative_values() {
        let mut config = LiquidGlassConfig::default();
        let negative_params = [
            (LiquidGlassParam::DisplacementScale, -10.0, 0.0),
            (LiquidGlassParam::BlurAmount, -0.5, 0.0),
            (LiquidGlassParam::ChromaticOffset, -3.0, 0.0),
            (LiquidGlassParam::CornerRadius, -5.0, 0.0),
            (LiquidGlassParam::Elasticity, -1.0, 0.0),
        ];
        for (param, input, expected) in negative_params.iter() {
            config.set_param(*param, *input);
            assert!(
                (config.get_param(*param) - *expected).abs() < f32::EPSILON,
                "Negative clamp failed for {:?}: got {}",
                param,
                config.get_param(*param)
            );
        }
    }

    #[test]
    fn test_set_param_clamps_over_max_values() {
        let mut config = LiquidGlassConfig::default();
        let over_max_params = [
            (LiquidGlassParam::DisplacementScale, 999.0, 200.0),
            (LiquidGlassParam::BlurAmount, 5.0, 1.0),
            (LiquidGlassParam::Saturation, 10.0, 3.0),
            (LiquidGlassParam::ChromaticOffset, 100.0, 20.0),
            (
                LiquidGlassParam::CornerRadius,
                999.0,
                LIQUID_GLASS_CORNER_RADIUS_MAX,
            ),
            (LiquidGlassParam::Elasticity, 5.0, 1.0),
        ];
        for (param, input, expected) in over_max_params.iter() {
            config.set_param(*param, *input);
            assert!(
                (config.get_param(*param) - *expected).abs() < f32::EPSILON,
                "Over-max clamp failed for {:?}: got {}",
                param,
                config.get_param(*param)
            );
        }
    }

    #[test]
    fn test_set_param_refraction_mode_rounds_and_clamps() {
        let mut config = LiquidGlassConfig::default();
        config.set_param(LiquidGlassParam::RefractionMode, 1.7);
        assert_eq!(config.refraction_mode, 2);
        config.set_param(LiquidGlassParam::RefractionMode, -1.0);
        assert_eq!(config.refraction_mode, 0);
        config.set_param(LiquidGlassParam::RefractionMode, 10.0);
        assert_eq!(config.refraction_mode, 3);
    }

    #[test]
    fn test_saturation_lower_bound_is_one() {
        let mut config = LiquidGlassConfig::default();
        config.set_param(LiquidGlassParam::Saturation, 0.5);
        assert!((config.saturation - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_normalize_clamps_corner_radius_over_max() {
        let mut config = LiquidGlassConfig {
            corner_radius: 999.0,
            ..LiquidGlassConfig::default()
        };
        config.normalize();
        assert!((config.corner_radius - LIQUID_GLASS_CORNER_RADIUS_MAX).abs() < f32::EPSILON);
    }
}
