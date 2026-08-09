import type { AiConfig, AppConfig, BoxStyleConfig } from "$lib/api/tauri-api";

let config = $state<AppConfig | null>(null);

export function getConfigState(): AppConfig | null {
  return config;
}

export function setConfigState(next: AppConfig): void {
  config = next;
}

export function setBoxStyleState(boxStyle: BoxStyleConfig): void {
  if (config) config.box_style = boxStyle;
}

export function setAiConfigState(ai: AiConfig): void {
  if (config) config.ai = ai;
}

export type { AiConfig, AppConfig, BoxStyleConfig };
