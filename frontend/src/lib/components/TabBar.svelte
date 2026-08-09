<script lang="ts">
  let { tabs = [], activeTab = "", onchange }: {
    tabs: Array<{ id: string; label: string }>; activeTab?: string; onchange?: (tabId: string) => void;
  } = $props();
</script>

<nav class="tabbar" aria-label="设置页面">
  {#each tabs as tab}
    <button type="button" class:active={activeTab === tab.id} onclick={() => onchange?.(tab.id)}>{tab.label}</button>
  {/each}
</nav>

<style>
  nav { display: flex; padding: 0 20px; border-bottom: 1px solid var(--border-tab); }
  button { position: relative; padding: 9px 16px; color: var(--text-muted); }
  button:hover { color: var(--text-primary); }
  button.active { color: var(--text-primary); font-weight: 600; }
  button.active::after { content: ""; position: absolute; left: 16px; right: 16px; bottom: -1px; height: 2px; border-radius: 2px; background: var(--primary-color); }
</style>
