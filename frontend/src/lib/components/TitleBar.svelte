<script lang="ts">
  import { Minus, Square, X } from "lucide-svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { hideCurrentWindow } from "$lib/window-lifecycle";

  let {
    title = "",
    showMinimize = true,
    showMaximize = false,
    showClose = true,
    onClose,
  }: {
    title?: string;
    showMinimize?: boolean;
    showMaximize?: boolean;
    showClose?: boolean;
    onClose?: () => void | Promise<void>;
  } = $props();

  let isMaximized = $state(false);

  async function handleMinimize() {
    await getCurrentWindow().minimize();
  }

  async function handleMaximize() {
    const window = getCurrentWindow();
    isMaximized = await window.isMaximized();
    if (isMaximized) await window.unmaximize();
    else await window.maximize();
    isMaximized = !isMaximized;
  }

  async function handleClose() {
    if (onClose) await onClose();
    else await hideCurrentWindow();
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <div class="titlebar-title" data-tauri-drag-region>{title}</div>
  <div class="titlebar-buttons">
    {#if showMinimize}
      <button type="button" class="titlebar-btn" onclick={handleMinimize} title="最小化" aria-label="最小化">
        <Minus size={15} />
      </button>
    {/if}
    {#if showMaximize}
      <button type="button" class="titlebar-btn" onclick={handleMaximize} title="最大化" aria-label="最大化">
        <Square size={isMaximized ? 12 : 14} />
      </button>
    {/if}
    {#if showClose}
      <button type="button" class="titlebar-btn close" onclick={handleClose} title="关闭" aria-label="关闭">
        <X size={16} />
      </button>
    {/if}
  </div>
</header>

<style>
  .titlebar {
    height: var(--titlebar-height);
    flex: 0 0 var(--titlebar-height);
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-left: 16px;
    border-bottom: 1px solid var(--border-tab);
    background: rgba(255, 255, 255, 0.22);
    user-select: none;
  }
  .titlebar-title { flex: 1; color: var(--text-primary); font-size: 13px; font-weight: 600; }
  .titlebar-buttons { height: 100%; display: flex; }
  .titlebar-btn {
    width: 42px;
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--text-secondary);
    transition: background 150ms ease, color 150ms ease;
  }
  .titlebar-btn:hover { background: rgba(0, 0, 0, 0.06); color: var(--text-primary); }
  .titlebar-btn.close { border-radius: 0 var(--border-radius-window) 0 0; }
  .titlebar-btn.close:hover { background: #e81123; color: #fff; }
</style>
