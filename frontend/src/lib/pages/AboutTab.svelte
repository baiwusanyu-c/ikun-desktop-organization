<script lang="ts">
  import { onMount } from "svelte";
  import { ExternalLink, RefreshCw } from "lucide-svelte";
  import { checkUpdate, getAppInfo, openUrl } from "$lib/api/tauri-api";

  const AUTHOR_URL = "https://github.com/baiwusanyu-c";
  const PROJECT_URL = "https://github.com/baiwusanyu-c/ikun-desktop-organization";
  let version = $state("0.1.0");
  let isChecking = $state(false);
  let updateResult = $state("");

  onMount(() => { getAppInfo().then((info) => { version = info.version; }).catch(() => undefined); });

  async function openExternal(url: string) {
    try { await openUrl(url); }
    catch { window.open(url, "_blank", "noopener,noreferrer"); }
  }

  async function handleCheckUpdate() {
    if (isChecking) return;
    isChecking = true;
    updateResult = "";
    try { updateResult = await checkUpdate(); }
    catch (error) { updateResult = String(error); }
    finally { isChecking = false; }
  }
</script>

<div class="about">
  <section class="identity">
    <div class="logo"><img src="/logo.ico" alt="logo" /></div>
    <h1>ikun-organizer</h1>
    <p>AI 驱动的桌面图标管理工具</p>
    <span>智能分类 · 拖拽操作 · 标签提取</span>
  </section>
  <section class="info">
    <div class="row"><span class="label">版本号</span><div class="value"><span>v{version}</span><button type="button" class="check" onclick={handleCheckUpdate} disabled={isChecking}><RefreshCw size={12} class={isChecking ? "spinning" : ""} />{isChecking ? "检查中" : "检查更新"}</button></div></div>
    {#if updateResult}<div class="result" role="status">{updateResult}</div>{/if}
    <div class="row"><span class="label">作者</span><button type="button" class="link" onclick={() => openExternal(AUTHOR_URL)}>baiwusanyu-c <ExternalLink size={12} /></button></div>
    <div class="row"><span class="label">项目地址</span><button type="button" class="link" onclick={() => openExternal(PROJECT_URL)}>GitHub <ExternalLink size={12} /></button></div>
    <div class="row"><span class="label">许可证</span><span class="value">MIT</span></div>
  </section>
</div>

<style>
  .about { padding: 26px 28px 30px; }
  .identity { display: flex; flex-direction: column; align-items: center; padding: 8px 0 26px; text-align: center; }
  .logo { width: 66px; height: 66px; display: grid; place-items: center; margin-bottom: 13px; border-radius: 16px; overflow: hidden; }
  .logo img { width: 100%; height: 100%; object-fit: contain; }
  h1 { font-size: 20px; font-weight: 700; }
  .identity p { margin-top: 7px; color: var(--text-secondary); font-size: 13px; }
  .identity span { margin-top: 4px; color: var(--text-muted); font-size: 12px; }
  .info { overflow: hidden; border: 1px solid var(--border-group); border-radius: 8px; background: var(--bg-group); }
  .row { min-height: 47px; display: flex; align-items: center; justify-content: space-between; padding: 9px 15px; border-bottom: 1px solid var(--border-group); }
  .row:last-child { border-bottom: 0; }
  .label { color: var(--text-muted); font-size: 12px; }
  .value { display: flex; align-items: center; gap: 9px; color: var(--text-primary); font-size: 12px; font-weight: 600; }
  .check { display: flex; align-items: center; gap: 4px; padding: 4px 8px; border: 1px solid var(--primary-shadow); border-radius: 6px; color: var(--primary-hover); background: var(--primary-glow); font-size: 10px; font-weight: 600; }
  .check:hover:not(:disabled) { color: #fff; background: var(--primary-color); }
  .check:disabled { opacity: .65; cursor: wait; }
  .link { display: flex; align-items: center; gap: 4px; color: var(--primary-hover); font-size: 12px; font-weight: 600; }
  .link:hover { text-decoration: underline; }
  .result { padding: 8px 15px; border-bottom: 1px solid var(--border-group); color: var(--text-secondary); background: var(--primary-glow); font-size: 11px; text-align: right; }
  :global(.spinning) { animation: spin 800ms linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
