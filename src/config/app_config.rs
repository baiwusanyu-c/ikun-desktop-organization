//! 统一应用配置
//!
//! 所有设置集中在一个 JSON 文件中，支持导入/导出。
//! 存储位置：
//! - Windows: `%APPDATA%/ikun-desktop-organization/config.json`
//! - macOS/Linux: `~/.config/ikun-desktop-organization/config.json`

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 统一应用配置（对应 config.json）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: String,
    pub last_modified: chrono::DateTime<chrono::Utc>,
    /// 通用设置
    pub general: GeneralConfig,
    /// 分类框默认样式
    pub box_style: BoxStyleConfig,
    /// AI 分类配置
    pub ai: AiConfig,
    /// 热键配置
    pub hotkeys: HotkeyConfig,
    /// 系统托盘配置
    pub tray: TrayConfig,
    /// 布局数据（分类框位置/尺寸/锁定）
    pub layout: LayoutConfig,
    /// 分类规则
    pub classification: ClassificationConfig,
    /// 调试配置
    pub debug: DebugConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: "2.0".to_string(),
            last_modified: chrono::Utc::now(),
            general: GeneralConfig::default(),
            box_style: BoxStyleConfig::default(),
            ai: AiConfig::default(),
            hotkeys: HotkeyConfig::default(),
            tray: TrayConfig::default(),
            layout: LayoutConfig::default(),
            classification: ClassificationConfig::default(),
            debug: DebugConfig::default(),
        }
    }
}

impl AppConfig {
    /// 加载配置（文件不存在则返回默认值）
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

    /// 保存配置
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

    /// 导出配置到指定路径
    pub fn export_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        log::info!("导出配置到: {:?}", path);
        Ok(())
    }

    /// 从指定路径导入配置
    pub fn import_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = serde_json::from_str(&content)?;
        log::info!("导入配置成功: {:?}", path);
        Ok(config)
    }

    /// 获取配置文件路径
    fn get_config_path() -> anyhow::Result<PathBuf> {
        dirs::config_dir()
            .map(|p| p.join("ikun-desktop-organization/config.json"))
            .ok_or_else(|| anyhow::anyhow!("无法获取配置目录"))
    }
}

// ===== 通用设置 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub auto_start: bool,
    pub auto_classify_on_start: bool,
    pub language: String,
    pub single_instance: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            auto_classify_on_start: true,
            language: "zh-CN".to_string(),
            single_instance: true,
        }
    }
}

// ===== 分类框样式 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxStyleConfig {
    pub background_color: [u8; 3],
    pub background_alpha: u8,
    pub border_radius: f32,
    pub border_color: [u8; 4],
    pub border_width: f32,
    pub title_font_size: f32,
    pub title_font_family: String,
    pub label_font_size: f32,
    pub icon_size: i32,
    pub cell_spacing: i32,
    pub title_bar_height: i32,
    pub highlight_color: [u8; 4],
    pub hover_color: [u8; 4],
    pub shadow_enabled: bool,
    pub shadow_color: [u8; 4],
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
            label_font_size: 13.0,
            icon_size: 48,
            cell_spacing: 16,
            title_bar_height: 40,
            highlight_color: [60, 80, 120, 120],
            hover_color: [70, 90, 130, 70],
            shadow_enabled: true,
            shadow_color: [0, 0, 0, 200],
        }
    }
}

// ===== AI 配置 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub system_prompt: String,
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
        }
    }
}

// ===== 热键配置 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub toggle_panel: String,
    pub toggle_icons: String,
    pub auto_classify: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle_panel: "Ctrl+Alt+D".to_string(),
            toggle_icons: "Ctrl+Alt+H".to_string(),
            auto_classify: "Ctrl+Alt+C".to_string(),
        }
    }
}

// ===== 系统托盘配置 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayConfig {
    pub enabled: bool,
    pub show_on_start: bool,
    pub double_click_action: String,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_on_start: false,
            double_click_action: "show_panel".to_string(),
        }
    }
}

// ===== 布局配置 =====

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LayoutConfig {
    pub boxes: Vec<BoxLayoutData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxLayoutData {
    pub id: uuid::Uuid,
    pub name: String,
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub locked: bool,
    pub sort_mode: SortMode,
    pub sort_ascending: bool,
    pub view_mode: ViewMode,
    pub group_by: Option<GroupBy>,
    pub icon_names: Vec<String>,
}

// ===== 分类规则配置 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationConfig {
    pub source: String,
    pub file_path: Option<String>,
    pub categories: Vec<CategoryRule>,
}

impl Default for ClassificationConfig {
    fn default() -> Self {
        Self {
            source: "embedded".to_string(),
            file_path: None,
            categories: vec![
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
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryRule {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub exe_patterns: Vec<String>,
    pub path_patterns: Vec<String>,
    pub keywords: Vec<String>,
}

// ===== 调试配置 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugConfig {
    pub log_level: String,
    pub log_to_file: bool,
    pub log_file_path: Option<String>,
    pub log_max_size_mb: u32,
    pub show_fps: bool,
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

// ===== 枚举类型 =====

/// 排序模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    Name,
    Type,
    ModifiedTime,
    Author,
    CreatedTime,
    Size,
    Tag,
    Title,
}

impl Default for SortMode {
    fn default() -> Self {
        Self::Name
    }
}

/// 视图模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    ExtraLarge,
    Large,
    Medium,
    Small,
    List,
    Detail,
    Tiles,
    Content,
}

impl Default for ViewMode {
    fn default() -> Self {
        Self::Medium
    }
}

/// 分组方式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    None,
    Name,
    Type,
    ModifiedTime,
    Author,
    CreatedTime,
    Size,
    Tag,
    Title,
}

impl Default for GroupBy {
    fn default() -> Self {
        Self::None
    }
}
