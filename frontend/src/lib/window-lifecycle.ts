import { getCurrentWindow } from "@tauri-apps/api/window";

export async function hideCurrentWindow(): Promise<void> {
  try {
    await getCurrentWindow().hide();
  } catch (error) {
    console.error("隐藏窗口失败:", error);
  }
}

export function installHideOnClose(beforeHide?: () => void | Promise<void>): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;

  getCurrentWindow().onCloseRequested(async (event) => {
    event.preventDefault();
    await beforeHide?.();
    await hideCurrentWindow();
  }).then((cleanup) => {
    if (disposed) cleanup();
    else unlisten = cleanup;
  }).catch((error) => {
    console.error("注册窗口关闭处理失败:", error);
  });

  return () => {
    disposed = true;
    unlisten?.();
  };
}
