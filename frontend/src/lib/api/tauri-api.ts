import { invoke } from "@tauri-apps/api/core";

export interface ValidationResult {
  valid: boolean;
  message: string;
}

export interface BoxData {
  id: string;
  name: string;
  locked: boolean;
  position: [number, number];
  size: [number, number];
  item_count: number;
}

// --- liquid-verre-rs aligned types ---

export type GlassShape = "RoundedRect" | "Squircle";
export type GlassProfile = "Circular" | "Parabolic" | "Sinusoidal";
export type GlassCaptureBackend = "Auto" | "ForceDda" | "ForceWgc";

export interface GlassLiquidParams {
  corner_radius: number;
  lens_coverage: number;
  bevel_width: number;
  shape: GlassShape;
  super_n: number;
  profile: GlassProfile;
  ior: number;
  thickness: number;
  refr_strength: number;
  dispersion: number;
  absorption: number;
  clarity: number;
  blur_max: number;
  blur_min: number;
  progressive_curve: number;
  edge_always: number;
  tint_opacity: number;
  adaptive_tint: number;
  base_alpha: number;
  tint_color: [number, number, number, number];
  env_tint_mix: number;
  light_dir: [number, number, number];
  light_intensity: number;
  spec_intensity: number;
  spec_power: number;
  spec_spread: number;
  fresnel_f0: number;
  rim_width: number;
  caustic_gain: number;
  mouse_radius: number;
  mouse_intensity: number;
  mouse_spread: number;
  mouse_color: [number, number, number];
  proximity_glow: number;
  shadow_opacity: number;
  shadow_margin: number;
  lag_seconds: number;
}

export interface GlassTransparentParams {
  corner_radius: number;
  opacity: number;
  color: [number, number, number, number];
}

export interface GlassResizeFeedback {
  enabled: boolean;
  border_width: number;
  alpha: number;
  color: [number, number, number];
}

export interface GlassResizeConfig {
  enabled: boolean;
  edges: boolean;
  corners: boolean;
  grip_px: number;
  min_width: number;
  min_height: number;
  feedback: GlassResizeFeedback;
}

export interface GlassEngineConfig {
  capture_backend: GlassCaptureBackend;
  adaptive_quality: boolean;
  occlusion_culling: boolean;
  fullscreen_hide: boolean;
  fps_cap: number;
  monitor_limit: number;
}

export interface LiquidGlassConfig {
  enabled: boolean;
  liquid: GlassLiquidParams;
  transparent: GlassTransparentParams;
  resize: GlassResizeConfig;
  engine: GlassEngineConfig;
}

// --- end liquid-verre-rs types ---

export interface BoxStyleConfig {
  background_color: [number, number, number];
  background_alpha: number;
  border_radius: number;
  border_color: [number, number, number, number];
  border_width: number;
  title_font_size: number;
  title_font_family: string;
  title_font_color: [number, number, number, number];
  label_font_size: number;
  label_font_color: [number, number, number, number];
  icon_size: number;
  cell_spacing: number;
  title_bar_height: number;
  title_bar_color: [number, number, number, number];
  highlight_color: [number, number, number, number];
  hover_color: [number, number, number, number];
  shadow_enabled: boolean;
  shadow_color: [number, number, number, number];
  columns: number;
  lock_button_visible: boolean;
  close_button_visible: boolean;
  icon_only_mode: boolean;
  liquid_glass: LiquidGlassConfig;
}

export interface AiConfig {
  base_url: string;
  api_key: string;
  model: string;
  timeout_seconds: number;
  max_retries: number;
}

export interface AiClassifyResponse {
  results: Array<{ name: string; category: string }>;
  new_categories?: string[];
}

export interface ThemeColors {
  background: string | null;
  foreground: string | null;
  border: string | null;
  primary: string | null;
  primary_foreground: string | null;
  success: string | null;
  danger: string | null;
  muted_foreground: string | null;
}

export interface UiThemeConfig {
  light: ThemeColors;
  dark: ThemeColors;
}

export type SortMode = "name" | "type" | "modified_time" | "author" | "created_time" | "size" | "tag" | "title";
export type ViewMode = "extra_large" | "large" | "medium" | "small" | "list" | "detail" | "tiles" | "content";
export type GroupBy = "none" | "name" | "type" | "modified_time" | "author" | "created_time" | "size" | "tag" | "title";

export interface GeneralConfig {
  auto_start: boolean;
  language: string;
  single_instance: boolean;
  restore_layout_on_start: boolean;
  auto_classify_on_start: boolean;
}

export interface HotkeyConfig {
  toggle_panel: string;
  auto_classify: string;
}

export interface BoxDefaultsConfig {
  view_mode: ViewMode;
  sort_mode: SortMode;
  sort_ascending: boolean;
  group_by: GroupBy;
  default_width: number;
  default_height: number;
}

export interface CategoryRule {
  id: string;
  name: string;
  icon: string;
  exe_patterns: string[];
  path_patterns: string[];
  keywords: string[];
}

export interface BoxStyleOverrides {
  background_color: [number, number, number] | null;
  background_alpha: number | null;
  border_radius: number | null;
  border_color: [number, number, number, number] | null;
  border_width: number | null;
  title_font_size: number | null;
  label_font_size: number | null;
  icon_size: number | null;
  cell_spacing: number | null;
  title_bar_height: number | null;
  title_bar_color: [number, number, number, number] | null;
  highlight_color: [number, number, number, number] | null;
  hover_color: [number, number, number, number] | null;
  shadow_enabled: boolean | null;
  columns: number | null;
}

export interface BoxLayoutData {
  id: string;
  name: string;
  position: [number, number];
  size: [number, number];
  locked: boolean;
  sort_mode: SortMode;
  sort_ascending: boolean;
  view_mode: ViewMode;
  group_by: GroupBy;
  icon_names: string[];
  display_names: string[];
  grid_positions: Array<[number, number]>;
  style_overrides: BoxStyleOverrides | null;
}

export interface LayoutConfig { boxes: BoxLayoutData[] }
export interface OverlayConfig { icon_positions: Record<string, [number, number]>; icon_size: number; layout_direction: string }
export interface DebugConfig { log_level: string; log_to_file: boolean; log_file_path: string | null; log_max_size_mb: number; show_fps: boolean; render_wireframe: boolean }

export interface AppConfig {
  version: string;
  last_modified: string;
  general: GeneralConfig;
  hotkeys: HotkeyConfig;
  box_style: BoxStyleConfig;
  box_defaults: BoxDefaultsConfig;
  ai: AiConfig;
  classification: CategoryRule[];
  layout: LayoutConfig;
  overlay: OverlayConfig;
  debug: DebugConfig;
  ui_theme: UiThemeConfig;
}

export interface AppInfo {
  version: string;
  name: string;
  build_date: string;
}

export interface EditBoxPayload {
  config_id: string;
  current_name: string;
}

function parseConfig(value: AppConfig | string): AppConfig {
  return typeof value === "string" ? JSON.parse(value) as AppConfig : value;
}

export async function getConfig(): Promise<AppConfig> {
  return parseConfig(await invoke<AppConfig | string>("get_config"));
}

export async function saveConfig(config: AppConfig): Promise<void> {
  await invoke("save_config", { config });
}

export async function updateThemeColor(color: string): Promise<void> {
  await invoke("update_theme_color", { color });
}

export async function updateBoxStyle(boxStyle: BoxStyleConfig): Promise<void> {
  await invoke("update_box_style", { style: boxStyle });
}

export async function updateLiquidGlass(liquidGlass: LiquidGlassConfig): Promise<void> {
  await invoke("update_liquid_glass", { config: liquidGlass });
}

export async function updateAiConfig(aiConfig: AiConfig): Promise<void> {
  await invoke("update_ai_config", { config: aiConfig });
}

export async function getPendingEditBox(): Promise<EditBoxPayload | null> {
  return invoke<EditBoxPayload | null>("get_pending_edit_box");
}

export async function validateBoxName(name: string): Promise<ValidationResult> {
  return invoke<ValidationResult>("validate_box_name", { name });
}

export async function createBox(name: string, description = ""): Promise<BoxData> {
  return invoke<BoxData>("create_box", { name, description });
}

export async function editBox(id: string, name: string, description = ""): Promise<BoxData> {
  return invoke<BoxData>("edit_box", { id, name, description });
}

export async function showWindow(label: string): Promise<void> {
  await invoke("show_window", { label });
}

export async function hideWindow(label: string): Promise<void> {
  await invoke("hide_window", { label });
}

export async function openUrl(url: string): Promise<void> {
  await invoke("open_url", { url });
}

export async function checkUpdate(): Promise<string> {
  return invoke<string>("check_update");
}

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("get_app_info");
}


/**
 * Send a log message to the Rust backend.
 * The window label is automatically attached.
 *
 * @param level - Log level: "debug" | "info" | "warn" | "error"
 * @param message - Log message text
 */
export async function logToRust(level: "debug" | "info" | "warn" | "error", message: string): Promise<void> {
  try {
    await invoke("log_message", { level, message });
  } catch {
    // Silently ignore IPC failures to avoid infinite loops
  }
}
