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
use std::fmt;
use std::path::PathBuf;

/// 配置文件名（存放在用户主目录）
const CONFIG_FILE_NAME: &str = ".ikun_organizer.json";

/// 原子写入文件：先写临时文件，再用 `rename` 原子替换目标文件。
///
/// 避免进程在写入中途被杀（强制关机/断电）产生半截目标文件。
/// Windows 的 `rename` 会原子替换已存在的目标文件；失败时清理临时文件。
fn atomic_write_file(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, content.as_bytes())
        .map_err(|e| anyhow::anyhow!("写入临时文件失败 {}: {}", tmp_path.display(), e))?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        // 替换失败时清理临时文件，避免残留
        let _ = std::fs::remove_file(&tmp_path);
        return Err(anyhow::anyhow!(
            "临时文件替换目标文件失败 {}: {}",
            path.display(),
            e
        ));
    }
    Ok(())
}

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
            if config.box_style.normalize() {
                config.last_modified = chrono::Utc::now();
                if let Err(error) = config.save() {
                    log::warn!("保存迁移后的分类框样式失败: {error}");
                }
            }
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
    ///
    /// 采用原子写入：先写临时文件再 `rename` 覆盖，避免进程在写入中途被杀
    /// （强制关机/断电）产生半截配置文件。配置文件一旦损坏，启动时会回退默认值，
    /// 进而把开机自启注册表条目同步删除（自启失效的隐藏根因之一）。
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::get_config_path()?;
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("序列化配置失败: {}", e))?;
        atomic_write_file(&path, &content)?;
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
        config.box_style.normalize();
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// 纯图标模式，开启后隐藏分类框名称
    pub icon_only_mode: bool,
    /// 液态玻璃效果参数
    pub liquid_glass: LiquidGlassConfig,
}

impl Default for BoxStyleConfig {
    fn default() -> Self {
        Self {
            background_color: [255, 255, 255],
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
            icon_only_mode: false,
            liquid_glass: LiquidGlassConfig::default(),
        }
    }
}

impl BoxStyleConfig {
    /// 统一迁移后的分类框圆角与液态玻璃兼容字段。
    pub(crate) fn normalize(&mut self) -> bool {
        let before = self.clone();

        // 旧版把半透明回退背景写成 [1, 1, 1]，视觉上接近纯黑。
        // Tauri 迁移计划已将默认值改为白色，需要在加载历史配置时同步迁移。
        if self.background_color == [1, 1, 1] {
            self.background_color = [255, 255, 255];
        }
        self.border_radius = self
            .border_radius
            .clamp(0.0, LIQUID_GLASS_CORNER_RADIUS_MAX);
        self.liquid_glass.normalize();
        // 「分类框风格」表单字段 → 库参数的统一映射（渲染出口只有 liquid-verre-rs）：
        // - 圆角：两种模式一致（面板不再暴露分模式圆角滑条，单一来源 border_radius）；
        // - 背景色/透明度：驱动透明玻璃底板。非液态模式的底板由库
        //   `GlassStyle::Transparent(TransparentParams)` 渲染，应用层 D2D 背景
        //   仅作为引擎不可用时的回退绘制（`d2d_render.rs` 第二趟）。
        //   2026-11-02 用户报「背景色/透明度不生效」：旧版只联动了圆角，
        //   颜色与 alpha 从未流入 transparent 参数组，这里补齐。
        self.liquid_glass.transparent.corner_radius = self.border_radius;
        self.liquid_glass.liquid.corner_radius = self.border_radius;
        let [red, green, blue] = self.background_color;
        self.liquid_glass.transparent.color = [
            f32::from(red) / 255.0,
            f32::from(green) / 255.0,
            f32::from(blue) / 255.0,
            1.0,
        ];
        self.liquid_glass.transparent.opacity = f32::from(self.background_alpha) / 255.0;
        *self != before
    }
}

// =============================================================================
// LiquidGlassConfig - 液态玻璃配置（liquid-verre-rs 引擎，参数镜像库 API）
// =============================================================================

/// 分类框控制面板圆角半径上限（`border_radius` 的旧联动量纲，保留兼容）。
pub const LIQUID_GLASS_CORNER_RADIUS_MAX: f32 = 30.0;

/// 玻璃形状模式（对应 `liquid_verre::Shape`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlassShape {
    /// 圆角矩形（精确欧氏 SDF）
    #[default]
    RoundedRect,
    /// 超椭圆 squircle（曲率连续）
    Squircle,
}

/// 斜面剖面类型（对应 `liquid_verre::Profile`，决定折射 LUT 的形状）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlassProfile {
    /// 圆形：边缘圆滑、放大感强（默认）
    #[default]
    Circular,
    /// 抛物线：位移线性，性能最好
    Parabolic,
    /// 正弦：过渡最柔和
    Sinusoidal,
}

/// 桌面采集后端偏好（对应 `liquid_verre::CaptureBackend`，引擎重建后生效）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlassCaptureBackend {
    /// 自动：HDR 输出走 WGC，SDR 输出 DDA 优先、失败回退 WGC
    #[default]
    Auto,
    /// 强制 DXGI Desktop Duplication
    ForceDda,
    /// 强制 Windows.Graphics.Capture
    ForceWgc,
}

/// 液态玻璃模式全部可调参数（镜像 `liquid_verre::LiquidParams` 共 37 项）。
///
/// 字段含义与夹紧区间与库 `sanitize()` 一致；长度量纲为逻辑像素。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlassLiquidParams {
    // ---- 几何 ----
    /// 圆角半径（0~400）
    pub corner_radius: f32,
    /// 斜面覆盖：0=仅边缘细带，1=覆盖整块（0~1）
    pub lens_coverage: f32,
    /// 边缘斜面带宽（1~400）
    pub bevel_width: f32,
    /// 形状模式
    pub shape: GlassShape,
    /// 超椭圆指数（Squircle 时生效，2~10）
    pub super_n: f32,
    /// 斜面剖面
    pub profile: GlassProfile,

    // ---- 折射 ----
    /// 玻璃折射率 IOR（1.0~2.0）
    pub ior: f32,
    /// 等效厚度（0~600）
    pub thickness: f32,
    /// 折射强度（0~2）
    pub refr_strength: f32,
    /// RGB 色散（0~1）
    pub dispersion: f32,
    /// Beer-Lambert 吸收强度（0~2）
    pub absorption: f32,

    // ---- 毛玻璃 ↔ 全透 ----
    /// 清晰度：0=毛玻璃，1=全透折射（0~1）
    pub clarity: f32,
    /// 最大模糊半径（0~96）
    pub blur_max: f32,
    /// 最小模糊半径（0~48，且不大于 blur_max）
    pub blur_min: f32,
    /// 渐进曲线指数（0~8）
    pub progressive_curve: f32,
    /// 边缘始终保留的清晰折射（0~1）
    pub edge_always: f32,

    // ---- 染色 ----
    /// 染色不透明度（0~1）
    pub tint_opacity: f32,
    /// 自适应染色（按环境亮度，0~1）
    pub adaptive_tint: f32,
    /// 基础 alpha（0~1）
    pub base_alpha: f32,
    /// 染色颜色 RGBA（每通道 0~1）
    pub tint_color: [f32; 4],
    /// 环境色混入比例（0~1）
    pub env_tint_mix: f32,

    // ---- 光照 ----
    /// 主光方向（指向光，入库前归一化）
    pub light_dir: [f32; 3],
    /// 主光强度（0~4）
    pub light_intensity: f32,
    /// 高光强度（0~4）
    pub spec_intensity: f32,
    /// 高光指数（1~512）
    pub spec_power: f32,
    /// 高光扩散（0~1）
    pub spec_spread: f32,
    /// Fresnel F0（0~1）
    pub fresnel_f0: f32,
    /// 描边宽度（0~16）
    pub rim_width: f32,
    /// 焦散增益（0~4）
    pub caustic_gain: f32,

    // ---- 鼠标点光 ----
    /// 鼠标半径（20~1200）
    pub mouse_radius: f32,
    /// 鼠标强度（0~4）
    pub mouse_intensity: f32,
    /// 鼠标高光扩散（4~400）
    pub mouse_spread: f32,
    /// 鼠标高光颜色 RGB（每通道 0~1）
    pub mouse_color: [f32; 3],
    /// 邻近余晖（0~2）
    pub proximity_glow: f32,

    // ---- 其他 ----
    /// 阴影不透明度（0~1）
    pub shadow_opacity: f32,
    /// 阴影边距（0~96）
    pub shadow_margin: f32,
    /// 拖拽滞后补偿秒数（0~0.1）
    pub lag_seconds: f32,
}

impl Default for GlassLiquidParams {
    fn default() -> Self {
        // 默认值对齐 liquid-verre-rs v0.4.2 的 Apple 风格液态玻璃稳态基线，
        // 仅 corner_radius 取 16.0 以兼容本项目旧面板的圆角联动。
        Self {
            corner_radius: 16.0,
            lens_coverage: 0.15,
            bevel_width: 26.0,
            shape: GlassShape::RoundedRect,
            super_n: 4.0,
            profile: GlassProfile::Circular,

            ior: 1.50,
            thickness: 26.0,
            refr_strength: 1.0,
            dispersion: 0.25,
            absorption: 0.18,

            clarity: 0.10,
            blur_max: 20.0,
            blur_min: 2.4,
            progressive_curve: 1.6,
            edge_always: 0.55,

            tint_opacity: 0.01,
            adaptive_tint: 0.35,
            base_alpha: 1.0,
            tint_color: [0.92, 0.95, 1.0, 1.0],
            env_tint_mix: 0.25,

            light_dir: [-0.45, -0.72, 0.53],
            light_intensity: 1.0,
            spec_intensity: 0.85,
            spec_power: 42.0,
            spec_spread: 0.20,
            fresnel_f0: 0.14,
            rim_width: 1.5,
            caustic_gain: 0.55,

            mouse_radius: 220.0,
            mouse_intensity: 1.35,
            mouse_spread: 160.0,
            mouse_color: [1.0, 1.0, 1.0],
            proximity_glow: 0.35,

            shadow_opacity: 0.10,
            shadow_margin: 24.0,
            lag_seconds: 0.012,
        }
    }
}

/// 透明玻璃模式全部参数（镜像 `liquid_verre::TransparentParams`）。
///
/// 液态玻璃关闭时使用：由引擎绘制圆角半透明底板，取代旧 D2D 回退背景。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlassTransparentParams {
    /// 圆角半径（0~400，逻辑像素）
    pub corner_radius: f32,
    /// 玻璃透明度 0~1：0=完全透明，1=完全不透明
    pub opacity: f32,
    /// 玻璃颜色 RGBA（每通道 0~1，alpha 与 opacity 相乘）
    pub color: [f32; 4],
}

impl Default for GlassTransparentParams {
    fn default() -> Self {
        // 兼容旧默认：background_color 白、background_alpha 38/255≈0.15、border_radius 16
        Self {
            corner_radius: 16.0,
            opacity: 0.15,
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// 缩放悬停加粗 border 视觉反馈（镜像 `liquid_verre::ResizeFeedbackOptions`）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlassResizeFeedback {
    /// 是否启用 border 反馈（关掉后仍可拖拽，只是无视觉提示）
    pub enabled: bool,
    /// border 细度（逻辑像素，0.5~32）
    pub border_width: f32,
    /// border 不透明度（0~1）
    pub alpha: f32,
    /// border 颜色 RGB（每通道 0~1）
    pub color: [f32; 3],
}

impl Default for GlassResizeFeedback {
    fn default() -> Self {
        Self {
            enabled: true,
            border_width: 5.0,
            alpha: 1.0,
            color: [1.0, 1.0, 1.0],
        }
    }
}

/// 边缘/对角拖拽缩放交互（镜像 `liquid_verre::ResizeOptions`）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlassResizeConfig {
    /// 总开关。关掉后玻璃块退化为只能拖位置
    pub enabled: bool,
    /// 允许四边拖拽缩放（左/右/上/下）
    pub edges: bool,
    /// 允许对角拖拽缩放（四个圆角）
    pub corners: bool,
    /// 把手带宽（逻辑像素，2~128，对角命中后为角部 grip x grip 方块）
    pub grip_px: f32,
    /// 缩放后最小宽（逻辑像素，8~4096）
    pub min_width: f32,
    /// 缩放后最小高（逻辑像素，8~4096）
    pub min_height: f32,
    /// 悬停时的加粗 border 视觉反馈
    pub feedback: GlassResizeFeedback,
}

impl Default for GlassResizeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            edges: true,
            corners: true,
            grip_px: 10.0,
            min_width: 24.0,
            min_height: 24.0,
            feedback: GlassResizeFeedback::default(),
        }
    }
}

/// 引擎级配置（对应 `liquid_verre::EngineConfig` 子集，改动后触发引擎自动重建）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlassEngineConfig {
    /// 桌面采集后端偏好（引擎重建后生效）
    pub capture_backend: GlassCaptureBackend,
    /// 自适应质量控制器（GPU 负载高时自动降档）
    pub adaptive_quality: bool,
    /// 遮挡剔除：玻璃块被上层不透明窗口完整覆盖时暂停该块驱动
    pub occlusion_culling: bool,
    /// 独占全屏让路：检测到全屏应用时临时隐藏画布
    pub fullscreen_hide: bool,
    /// 帧率上限（0 = 跟随显示器刷新率，1~1000）
    pub fps_cap: u32,
    /// 画布限制在前 N 块显示器（0 = 全部）
    pub monitor_limit: u32,
}

impl Default for GlassEngineConfig {
    fn default() -> Self {
        Self {
            capture_backend: GlassCaptureBackend::default(),
            adaptive_quality: true,
            occlusion_culling: true,
            fullscreen_hide: true,
            fps_cap: 0,
            monitor_limit: 0,
        }
    }
}

/// 液态玻璃配置（JSON 键 `box_style.liquid_glass`）。
///
/// - `enabled=true`：引擎以液态玻璃模式渲染分类框底板（`liquid` 参数组）；
/// - `enabled=false`：引擎以透明玻璃模式渲染（`transparent` 参数组）；
/// - 两种模式都由 liquid-verre-rs 承担拖拽移动与边缘缩放交互；
/// - 引擎初始化失败时优雅降级回 D2D 回退背景与原生拖拽/缩放。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiquidGlassConfig {
    /// 液态玻璃效果开关（true=液态模式，false=透明模式）
    pub enabled: bool,
    /// 液态模式参数（37 项）
    pub liquid: GlassLiquidParams,
    /// 透明模式参数
    pub transparent: GlassTransparentParams,
    /// 缩放交互与视觉反馈
    pub resize: GlassResizeConfig,
    /// 引擎级参数（改动触发引擎重建）
    pub engine: GlassEngineConfig,
}

/// NaN / inf 安全夹紧（与 liquid-verre-rs `clamp_finite` 同语义）。
///
/// `f32::clamp` 对 NaN 原样透传（NaN 与任何边界比较都是 false），
/// 一旦流入 GPU 会让 SDF 运算整体失效且极难排查；非法输入一律收敛到边界。
fn clamp_finite_glass(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_nan() {
        lo
    } else if v == f32::INFINITY {
        hi
    } else if v == f32::NEG_INFINITY {
        lo
    } else {
        v.clamp(lo, hi)
    }
}

impl LiquidGlassConfig {
    /// 将持久化或外部传入的配置收敛到库支持的合法区间（镜像库 sanitize() 边界）。
    pub(crate) fn normalize(&mut self) {
        let l = &mut self.liquid;
        l.corner_radius = clamp_finite_glass(l.corner_radius, 0.0, 400.0);
        l.lens_coverage = clamp_finite_glass(l.lens_coverage, 0.0, 1.0);
        l.bevel_width = clamp_finite_glass(l.bevel_width, 1.0, 400.0);
        l.super_n = clamp_finite_glass(l.super_n, 2.0, 10.0);
        l.ior = clamp_finite_glass(l.ior, 1.0, 2.0);
        l.thickness = clamp_finite_glass(l.thickness, 0.0, 600.0);
        l.refr_strength = clamp_finite_glass(l.refr_strength, 0.0, 2.0);
        l.dispersion = clamp_finite_glass(l.dispersion, 0.0, 1.0);
        l.absorption = clamp_finite_glass(l.absorption, 0.0, 2.0);
        l.clarity = clamp_finite_glass(l.clarity, 0.0, 1.0);
        l.blur_max = clamp_finite_glass(l.blur_max, 0.0, 96.0);
        l.blur_min = clamp_finite_glass(l.blur_min, 0.0, 48.0)
            .min(clamp_finite_glass(l.blur_max, 0.0, 96.0));
        l.progressive_curve = clamp_finite_glass(l.progressive_curve, 0.0, 8.0);
        l.edge_always = clamp_finite_glass(l.edge_always, 0.0, 1.0);
        l.tint_opacity = clamp_finite_glass(l.tint_opacity, 0.0, 1.0);
        l.adaptive_tint = clamp_finite_glass(l.adaptive_tint, 0.0, 1.0);
        l.base_alpha = clamp_finite_glass(l.base_alpha, 0.0, 1.0);
        l.env_tint_mix = clamp_finite_glass(l.env_tint_mix, 0.0, 1.0);
        l.light_intensity = clamp_finite_glass(l.light_intensity, 0.0, 4.0);
        l.spec_intensity = clamp_finite_glass(l.spec_intensity, 0.0, 4.0);
        l.spec_power = clamp_finite_glass(l.spec_power, 1.0, 512.0);
        l.spec_spread = clamp_finite_glass(l.spec_spread, 0.0, 1.0);
        l.fresnel_f0 = clamp_finite_glass(l.fresnel_f0, 0.0, 1.0);
        l.rim_width = clamp_finite_glass(l.rim_width, 0.0, 16.0);
        l.caustic_gain = clamp_finite_glass(l.caustic_gain, 0.0, 4.0);
        l.mouse_radius = clamp_finite_glass(l.mouse_radius, 20.0, 1200.0);
        l.mouse_intensity = clamp_finite_glass(l.mouse_intensity, 0.0, 4.0);
        l.mouse_spread = clamp_finite_glass(l.mouse_spread, 4.0, 400.0);
        l.proximity_glow = clamp_finite_glass(l.proximity_glow, 0.0, 2.0);
        l.shadow_opacity = clamp_finite_glass(l.shadow_opacity, 0.0, 1.0);
        l.shadow_margin = clamp_finite_glass(l.shadow_margin, 0.0, 96.0);
        l.lag_seconds = clamp_finite_glass(l.lag_seconds, 0.0, 0.1);
        for c in l.tint_color.iter_mut() {
            *c = clamp_finite_glass(*c, 0.0, 1.0);
        }
        for c in l.mouse_color.iter_mut() {
            *c = clamp_finite_glass(*c, 0.0, 1.0);
        }
        let ld = l.light_dir;
        let len = (ld[0] * ld[0] + ld[1] * ld[1] + ld[2] * ld[2])
            .sqrt()
            .max(1e-4);
        l.light_dir = [ld[0] / len, ld[1] / len, ld[2] / len];

        let t = &mut self.transparent;
        t.corner_radius = clamp_finite_glass(t.corner_radius, 0.0, 400.0);
        t.opacity = clamp_finite_glass(t.opacity, 0.0, 1.0);
        for c in t.color.iter_mut() {
            *c = clamp_finite_glass(*c, 0.0, 1.0);
        }

        let r = &mut self.resize;
        r.grip_px = clamp_finite_glass(r.grip_px, 2.0, 128.0);
        r.min_width = clamp_finite_glass(r.min_width, 8.0, 4096.0);
        r.min_height = clamp_finite_glass(r.min_height, 8.0, 4096.0);
        r.feedback.border_width = clamp_finite_glass(r.feedback.border_width, 0.5, 32.0);
        r.feedback.alpha = clamp_finite_glass(r.feedback.alpha, 0.0, 1.0);
        for c in r.feedback.color.iter_mut() {
            *c = clamp_finite_glass(*c, 0.0, 1.0);
        }

        self.engine.fps_cap = self.engine.fps_cap.clamp(0, 1000);
        self.engine.monitor_limit = self.engine.monitor_limit.clamp(0, 16);
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
///
/// 所有字段均需用户自行填写，支持任何 OpenAI 兼容的 LLM API。
/// pi_key 始终为空字符串（禁止硬编码），需用户自行获取。
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    /// API 基础地址（必填，如 https://api.openai.com/v1）
    pub base_url: String,
    /// API 密钥（用户填写，从不硬编码）
    pub api_key: String,
    /// 模型名称（必填，如 gpt-4o-mini）
    pub model: String,
    /// API 请求超时秒数
    pub timeout_seconds: u32,
    /// 请求失败最大重试次数
    pub max_retries: u32,
}

/// 自定义 Debug 实现：脱敏 api_key，防止日志泄露
impl fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"***")
            .field("model", &self.model)
            .field("timeout_seconds", &self.timeout_seconds)
            .field("max_retries", &self.max_retries)
            .finish()
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            timeout_seconds: 120,
            max_retries: 0,
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
    /// 归属显示器身份（设备名 + 记录时屏矩形 + 屏内比例位置 + 待搬回标记）。
    ///
    /// 支撑「拔副屏后分类框等比重定位 / 重连自动搬回 / 启动恢复落屏」
    /// （2026-11-27）。None = 历史配置或身份未知，按绝对坐标 + 吸附兜底恢复。
    #[serde(default)]
    pub monitor: Option<BoxMonitorOrigin>,
}

/// 分类框的归属显示器身份（持久化形态）。
///
/// 字段语义与 `box_core::monitor_remap::MonitorOrigin` 一致：
/// - `device`：Win32 显示器设备名，空 = 未知（仅按几何匹配）；
/// - `rect`：记录时归属屏完整矩形 `[x, y, w, h]`（虚拟桌面物理像素）；
/// - `rel`：框在屏内的比例位置 `[x, y]`（0.0..=1.0，等比映射的载荷）；
/// - `relocated`：归属屏已被拔掉、框暂搬在别的在岗屏，重连后应自动搬回。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct BoxMonitorOrigin {
    /// 显示器设备名（空 = 未知）
    pub device: String,
    /// 记录时归属屏完整矩形 [x, y, w, h]（虚拟桌面物理像素）
    pub rect: [i32; 4],
    /// 框在归属屏内的比例位置 [x, y]
    pub rel: [f32; 2],
    /// 归属屏不在岗、框被暂搬（等待自动搬回）
    pub relocated: bool,
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
            monitor: None,
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
#[derive(Default)]
pub struct UiThemeConfig {
    /// 亮色主题颜色
    pub light: ThemeColors,
    /// 暗色主题颜色
    pub dark: ThemeColors,
}

/// 主题颜色集合
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
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

// =============================================================================
// 单元测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- atomic_write_file ----

    #[test]
    fn test_atomic_write_file_creates_content() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        atomic_write_file(&target, r#"{"a":1}"#).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), r#"{"a":1}"#);
    }

    #[test]
    fn test_atomic_write_file_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        atomic_write_file(&target, "content").unwrap();
        // 成功后不允许残留 .json.tmp 临时文件
        let tmp = dir.path().join("config.json.tmp");
        assert!(!tmp.exists(), "写入成功后不应残留临时文件");
    }

    #[test]
    fn test_atomic_write_file_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        std::fs::write(&target, "old").unwrap();
        atomic_write_file(&target, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    }

    // ---- Default 值验证 ----

    #[test]
    fn test_box_style_normalize_migrates_legacy_near_black_background() {
        let mut style = BoxStyleConfig {
            background_color: [1, 1, 1],
            ..BoxStyleConfig::default()
        };

        assert!(style.normalize());
        assert_eq!(style.background_color, [255, 255, 255]);
        assert!(!style.normalize(), "迁移后的配置再次规范化不应产生变化");
    }

    #[test]
    fn test_box_style_normalize_preserves_custom_dark_background() {
        let mut style = BoxStyleConfig {
            background_color: [2, 2, 2],
            ..BoxStyleConfig::default()
        };

        style.normalize();
        assert_eq!(style.background_color, [2, 2, 2]);
    }

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
        assert!(ai.base_url.is_empty());
        assert!(ai.api_key.is_empty());
        assert!(ai.model.is_empty());
        assert_eq!(ai.timeout_seconds, 120);
        assert_eq!(ai.max_retries, 0);
    }

    #[test]
    fn test_ai_config_user_override() {
        let mut ai = AiConfig::default();
        ai.base_url = "https://my-api.example.com".to_string();
        ai.model = "my-custom-model".to_string();
        assert_eq!(ai.base_url, "https://my-api.example.com");
        assert_eq!(ai.model, "my-custom-model");
    }

    #[test]
    fn test_ai_config_debug_redacts_key() {
        let mut ai = AiConfig::default();
        ai.api_key = "super-secret-key-12345".to_string();
        let debug_str = format!("{:?}", ai);
        assert!(!debug_str.contains("super-secret-key-12345"));
        assert!(debug_str.contains("***"));
    }

    #[test]
    fn test_ai_config_backward_compat() {
        // 旧配置文件包含已删除字段时不应报错
        let json_str = r#"{"enabled":true,"base_url":"","api_key":"","model":"","timeout_seconds":120,"max_retries":2,"temperature":0.7,"max_tokens":2048,"system_prompt":"test","headers":{},"proxy_url":"","concurrency":5,"verify_ssl":true,"user_prompt_template":"","batch_size":10,"batch_delay_ms":500}"#;
        let ai: AiConfig = serde_json::from_str(json_str).expect("should parse with extra fields");
        assert_eq!(ai.timeout_seconds, 120);
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
            monitor: Some(BoxMonitorOrigin {
                device: "\\\\.\\DISPLAY2".to_string(),
                rect: [1920, 0, 2560, 1440],
                rel: [0.03125, 0.0694],
                relocated: true,
            }),
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
        assert_eq!(
            parsed.monitor, layout.monitor,
            "归属屏身份与 relocated 标记必须完整 roundtrip"
        );
    }

    #[test]
    fn test_box_monitor_origin_missing_fields_use_default() {
        let monitor: BoxMonitorOrigin = serde_json::from_str("{}").unwrap();
        assert_eq!(monitor, BoxMonitorOrigin::default());
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
            monitor: None,
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
        assert!(
            parsed.monitor.is_none(),
            "旧配置缺少 monitor 字段时必须默认 None"
        );
    }
}

// =============================================================================
// LiquidGlassConfig 测试（liquid-verre-rs 参数镜像）
// =============================================================================

#[cfg(test)]
mod liquid_glass_tests {
    use super::*;

    #[test]
    fn test_glass_config_default_values() {
        let config = LiquidGlassConfig::default();
        assert!(!config.enabled, "默认关闭液态玻璃，走透明模式");
        assert!((config.liquid.corner_radius - 16.0).abs() < f32::EPSILON);
        assert!((config.liquid.clarity - 0.10).abs() < f32::EPSILON);
        assert!((config.transparent.opacity - 0.15).abs() < f32::EPSILON);
        assert!((config.resize.grip_px - 10.0).abs() < f32::EPSILON);
        assert!(config.resize.enabled && config.resize.edges && config.resize.corners);
        assert!(config.resize.feedback.enabled);
        assert!(config.engine.adaptive_quality);
        assert!(config.engine.occlusion_culling);
        assert_eq!(config.engine.fps_cap, 0);
        assert_eq!(config.engine.capture_backend, GlassCaptureBackend::Auto);
    }

    #[test]
    fn test_glass_config_serialization_roundtrip() {
        let mut config = LiquidGlassConfig::default();
        config.enabled = true;
        config.liquid.ior = 1.8;
        config.liquid.tint_color[0] = 0.5;
        config.resize.feedback.border_width = 8.0;
        config.engine.fps_cap = 120;
        let json = serde_json::to_string(&config).unwrap();
        let parsed: LiquidGlassConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn test_glass_config_backward_compatible_with_legacy_flat_fields() {
        // 旧版配置是平铺 9 字段；加载时必须忽略未知字段并用默认值填充新结构
        let json = r#"{
            "enabled": true,
            "displacement_scale": 50.0,
            "blur_amount": 0.3,
            "saturation": 1.5,
            "chromatic_offset": 5.0,
            "corner_radius": 16.0,
            "elasticity": 0.02,
            "refraction_mode": 0,
            "over_light": false
        }"#;
        let parsed: LiquidGlassConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.liquid, GlassLiquidParams::default());
    }

    #[test]
    fn test_normalize_clamps_out_of_range_params() {
        let mut config = LiquidGlassConfig::default();
        config.liquid.corner_radius = 999.0;
        config.liquid.ior = 0.5;
        config.liquid.blur_max = 10.0;
        config.liquid.blur_min = 48.0;
        config.resize.grip_px = f32::NAN;
        config.transparent.opacity = 5.0;
        config.engine.fps_cap = 5000;
        config.normalize();
        assert!((config.liquid.corner_radius - 400.0).abs() < f32::EPSILON);
        assert!((config.liquid.ior - 1.0).abs() < f32::EPSILON);
        assert!(
            config.liquid.blur_min <= config.liquid.blur_max,
            "blur_min 必须不超过 blur_max"
        );
        assert!(config.resize.grip_px.is_finite() && config.resize.grip_px >= 2.0);
        assert!((config.transparent.opacity - 1.0).abs() < f32::EPSILON);
        assert_eq!(config.engine.fps_cap, 1000);
    }

    #[test]
    fn test_normalize_renormalizes_light_direction() {
        let mut config = LiquidGlassConfig::default();
        config.liquid.light_dir = [-9.0, 0.0, 0.0];
        config.normalize();
        let len = (config.liquid.light_dir[0] * config.liquid.light_dir[0]
            + config.liquid.light_dir[1] * config.liquid.light_dir[1]
            + config.liquid.light_dir[2] * config.liquid.light_dir[2])
            .sqrt();
        assert!((len - 1.0).abs() < 1e-3, "主光方向必须归一化");
    }

    #[test]
    fn test_box_style_normalize_syncs_transparent_corner_radius() {
        // 分类框整体圆角继续联动透明模式圆角（旧行为），液态圆角保持独立
        let mut style = BoxStyleConfig::default();
        style.border_radius = 24.0;
        style.liquid_glass.liquid.corner_radius = 40.0;
        style.normalize();
        // 单一来源：两种模式圆角都跟随「圆角调节」，面板不再暴露分模式圆角滑条。
        assert!((style.liquid_glass.transparent.corner_radius - 24.0).abs() < f32::EPSILON);
        assert!((style.liquid_glass.liquid.corner_radius - 24.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_box_style_normalize_maps_background_to_transparent_params() {
        // 2026-11-02 回归钉：非液态模式的底板由库 TransparentParams 渲染，
        // 「分类框风格」的背景色/透明度必须流入 color/opacity，否则面板调节不生效。
        let mut style = BoxStyleConfig::default();
        style.background_color = [255, 0, 0];
        style.background_alpha = 184;
        style.normalize();
        let t = &style.liquid_glass.transparent;
        assert!((t.color[0] - 1.0).abs() < 1e-6);
        assert!(t.color[1].abs() < 1e-6);
        assert!(t.color[2].abs() < 1e-6);
        assert!((t.color[3] - 1.0).abs() < 1e-6);
        assert!((t.opacity - 184.0 / 255.0).abs() < 1e-6);
    }
}
