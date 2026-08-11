import { emit, listen } from "@tauri-apps/api/event";
import { getConfig } from "$lib/api/tauri-api";
import { applyThemeColor } from "$lib/utils/color";

const DEFAULT_THEME_COLOR = "#14b8a6";
const THEME_COLOR_CHANGED_EVENT = "theme-color-changed";

async function refreshThemeColor(): Promise<void> {
  try {
    const config = await getConfig();
    applyThemeColor(config.ui_theme.light.primary ?? DEFAULT_THEME_COLOR);
  } catch (error) {
    console.error("同步主题色失败:", error);
  }
}

export function applyThemeColorAcrossWindows(color: string): void {
  applyThemeColor(color);
  void emit(THEME_COLOR_CHANGED_EVENT, color).catch((error) => {
    console.error("广播主题色失败:", error);
  });
}

export function installThemeColorSync(): () => void {
  let disposed = false;
  const cleanups: Array<() => void> = [];

  function register(listener: Promise<() => void>): void {
    listener.then((cleanup) => {
      if (disposed) cleanup();
      else cleanups.push(cleanup);
    }).catch((error) => {
      console.error("注册主题同步事件失败:", error);
    });
  }

  register(listen<string>(THEME_COLOR_CHANGED_EVENT, ({ payload }) => {
    applyThemeColor(payload);
  }));
  register(listen("window-opened", () => {
    void refreshThemeColor();
  }));
  void refreshThemeColor();

  return () => {
    disposed = true;
    cleanups.forEach((cleanup) => cleanup());
  };
}
