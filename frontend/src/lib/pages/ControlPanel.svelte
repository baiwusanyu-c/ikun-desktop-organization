<script lang="ts">
  import { onMount } from "svelte";
  import { Bot, Info, Palette } from "lucide-svelte";
  import TitleBar from "$lib/components/TitleBar.svelte";
  import AboutTab from "$lib/pages/AboutTab.svelte";
  import AiSettings from "$lib/pages/AiSettings.svelte";
  import StyleSettings from "$lib/pages/StyleSettings.svelte";
  import { getConfig, type AppConfig } from "$lib/api/tauri-api";
  import { installHideOnClose } from "$lib/window-lifecycle";
  import { applyThemeColor } from "$lib/utils/color";

  type TabId = "style" | "ai" | "about";
  let activeTab = $state<TabId>("style");
  let config = $state<AppConfig | null>(null);
  let loadError = $state("");

  const tabs = [
    { id: "style" as const, label: "风格设置", icon: Palette },
    { id: "ai" as const, label: "AI 设置", icon: Bot },
    { id: "about" as const, label: "关于", icon: Info },
  ];

  onMount(() => {
    const removeCloseHandler = installHideOnClose();
    getConfig().then((value) => {
      config = value;
      const color = value.ui_theme.light.primary ?? "#14b8a6";
      applyThemeColor(color);
    }).catch((error) => { loadError = `无法加载配置: ${String(error)}`; });
    return removeCloseHandler;
  });
</script>

<div class="glass-window panel-window">
  <TitleBar title="控制面板" showMinimize={true} />
  <nav class="tabs" aria-label="控制面板页面">
    {#each tabs as tab}
      <button type="button" class:active={activeTab === tab.id} onclick={() => activeTab = tab.id}>
        <tab.icon size={14} />{tab.label}
      </button>
    {/each}
  </nav>
  <div class="content-scroll">
    {#if loadError}
      <div class="load-state error" role="alert">{loadError}</div>
    {:else if !config}
      <div class="load-state">正在加载设置...</div>
    {:else if activeTab === "style"}
      <StyleSettings {config} />
    {:else if activeTab === "ai"}
      <AiSettings {config} />
    {:else}
      <AboutTab />
    {/if}
  </div>
</div>

<style>
  .panel-window { position: relative; }
  .tabs { flex: 0 0 auto; display: flex; padding: 0 18px; border-bottom: 1px solid var(--border-tab); background: rgba(255,255,255,.12); }
  .tabs button { position: relative; display: flex; align-items: center; gap: 6px; padding: 10px 14px; color: var(--text-muted); font-size: 12px; font-weight: 500; }
  .tabs button:hover { color: var(--text-primary); }
  .tabs button.active { color: var(--text-primary); font-weight: 650; }
  .tabs button.active::after { content: ""; position: absolute; left: 14px; right: 14px; bottom: -1px; height: 2px; border-radius: 2px; background: var(--primary-color); }
  .content-scroll { flex: 1 1 0; min-height: 0; overflow-x: hidden; overflow-y: scroll; overscroll-behavior: contain; background: var(--bg-group); }
  .load-state { display: grid; min-height: 180px; place-items: center; color: var(--text-secondary); }
  .load-state.error { color: #b91c1c; padding: 20px; text-align: center; }
</style>
