<script lang="ts">
  import { Eye, EyeOff, Plus, Save, Trash2 } from "lucide-svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Toggle from "$lib/components/Toggle.svelte";
  import { updateAiConfig, type AiConfig, type AppConfig } from "$lib/api/tauri-api";

  interface HeaderEntry { id: number; key: string; value: string }
  let { config }: { config: AppConfig } = $props();
  let ai = $state<AiConfig>({} as AiConfig);
  let headers = $state<HeaderEntry[]>([]);
  let ready = $state(false);
  let showApiKey = $state(false);
  let saveState = $state<"idle" | "saving" | "saved" | "error">("idle");
  let message = $state("");
  let nextHeaderId = 0;
  let loadedConfig: AppConfig | undefined;

  $effect(() => {
    if (config === loadedConfig) return;
    loadedConfig = config;
    ai = $state.snapshot(config.ai);
    headers = Object.entries(ai.headers).map(([key, value]) => ({ id: nextHeaderId++, key, value }));
    ready = true;
  });

  function addHeader() { headers = [...headers, { id: nextHeaderId++, key: "", value: "" }]; }
  function removeHeader(id: number) { headers = headers.filter((header) => header.id !== id); }
  function updateHeader(id: number, field: "key" | "value", value: string) {
    headers = headers.map((header) => header.id === id ? { ...header, [field]: value } : header);
  }

  async function save() {
    saveState = "saving";
    message = "";
    const requestHeaders = Object.fromEntries(headers.filter((header) => header.key.trim()).map((header) => [header.key.trim(), header.value]));
    const next = { ...ai, headers: requestHeaders };
    try {
      await updateAiConfig($state.snapshot(next));
      ai = next;
      saveState = "saved";
      message = "AI 设置已保存";
      setTimeout(() => { if (saveState === "saved") saveState = "idle"; }, 1800);
    } catch (error) {
      saveState = "error";
      message = `保存失败：${String(error)}`;
    }
  }
</script>

{#if ready}
  <div class="ai-settings">
    <section class="group">
      <header><h2>基本设置</h2></header>
      <div class="body fields">
        <div class="row"><span>启用 AI</span><Toggle checked={ai.enabled} onchange={(value) => ai.enabled = value} /></div>
        <label for="base-url">API 地址</label><input id="base-url" type="url" bind:value={ai.base_url} placeholder="https://api.openai.com/v1" />
        <label for="api-key">API Key</label>
        <div class="password"><input id="api-key" type={showApiKey ? "text" : "password"} bind:value={ai.api_key} autocomplete="off" placeholder="sk-..." /><button type="button" onclick={() => showApiKey = !showApiKey} title={showApiKey ? "隐藏 API Key" : "显示 API Key"} aria-label={showApiKey ? "隐藏 API Key" : "显示 API Key"}>{#if showApiKey}<EyeOff size={15} />{:else}<Eye size={15} />{/if}</button></div>
        <div class="two-column"><div><label for="model">模型</label><input id="model" bind:value={ai.model} placeholder="gpt-4o-mini" /></div><div><label for="max-tokens">最大 Token 数</label><input id="max-tokens" type="number" bind:value={ai.max_tokens} min="1" max="8192" /></div></div>
        <Slider label="温度" value={ai.temperature} min={0} max={2} step={0.1} digits={1} onchange={(value) => ai.temperature = value} />
        <div class="two-column"><div><label for="timeout">请求超时（秒）</label><input id="timeout" type="number" bind:value={ai.timeout_seconds} min="5" max="120" /></div><div><label for="retries">最大重试次数</label><input id="retries" type="number" bind:value={ai.max_retries} min="0" max="5" /></div></div>
      </div>
    </section>

    <section class="group">
      <header class="header-actions"><h2>高级配置</h2><button type="button" class="add" onclick={addHeader}><Plus size={14} />添加请求头</button></header>
      <div class="body fields">
        {#if headers.length === 0}<p class="empty">未设置额外请求头</p>{/if}
        {#each headers as header (header.id)}
          <div class="header-row"><input aria-label="请求头名称" value={header.key} oninput={(event) => updateHeader(header.id, "key", event.currentTarget.value)} placeholder="Header Key" /><input aria-label="请求头值" value={header.value} oninput={(event) => updateHeader(header.id, "value", event.currentTarget.value)} placeholder="Header Value" /><button type="button" onclick={() => removeHeader(header.id)} title="删除请求头" aria-label="删除请求头"><Trash2 size={15} /></button></div>
        {/each}
        <label for="proxy">代理地址</label><input id="proxy" type="url" bind:value={ai.proxy_url} placeholder="http://127.0.0.1:7890（可选）" />
        <div class="two-column"><div><label for="concurrency">并发数</label><input id="concurrency" type="number" bind:value={ai.concurrency} min="1" max="10" /></div><div class="toggle-cell"><span>验证 SSL 证书</span><Toggle checked={ai.verify_ssl} onchange={(value) => ai.verify_ssl = value} /></div></div>
      </div>
    </section>

    <section class="group"><header><h2>提示词</h2></header><div class="body fields"><label for="system-prompt">系统提示词</label><textarea id="system-prompt" bind:value={ai.system_prompt} rows="4"></textarea><label for="user-prompt">用户提示词模板</label><textarea id="user-prompt" bind:value={ai.user_prompt_template} rows="5"></textarea><p class="hint">支持 {"{filename}"}、{"{filepath}"}、{"{category_name}"} 占位符</p></div></section>

    <section class="group"><header><h2>批处理</h2></header><div class="body fields"><label for="batch-size">批处理大小</label><input id="batch-size" class="short" type="number" bind:value={ai.batch_size} min="1" max="50" /><Slider label="批处理延迟" value={ai.batch_delay_ms} min={0} max={10000} step={100} suffix="ms" onchange={(value) => ai.batch_delay_ms = value} /></div></section>

    <footer class="save-footer">
      {#if message}<span class:error={saveState === "error"} role="status">{message}</span>{/if}
      <button type="button" class="save" onclick={save} disabled={saveState === "saving"}><Save size={15} />{saveState === "saving" ? "保存中..." : "保存"}</button>
    </footer>
  </div>
{/if}

<style>
  .ai-settings { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 20px; }
  .group { overflow: hidden; }
  header { min-height: 43px; display: flex; align-items: center; padding: 10px 14px; border-bottom: 1px solid var(--border-group); background: var(--bg-group-header); }
  h2 { font-size: 13px; font-weight: 650; }
  .header-actions { justify-content: space-between; }
  .body { padding: 13px 15px; }
  .fields { display: flex; flex-direction: column; gap: 9px; }
  label, .row > span, .toggle-cell > span { color: var(--text-secondary); font-size: 12px; font-weight: 600; }
  input, textarea { width: 100%; padding: 8px 10px; }
  textarea { resize: vertical; line-height: 1.5; min-height: 72px; }
  .row { display: flex; align-items: center; justify-content: space-between; min-height: 28px; }
  .two-column { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .two-column > div { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .password { position: relative; }
  .password input { padding-right: 38px; }
  .password button { position: absolute; right: 4px; top: 4px; width: 30px; height: 28px; display: grid; place-items: center; color: var(--text-muted); border-radius: 5px; }
  .password button:hover { background: rgba(0,0,0,.06); }
  .add { display: flex; align-items: center; gap: 5px; padding: 5px 8px; border-radius: 6px; color: var(--primary-hover); background: var(--primary-glow); font-size: 11px; font-weight: 600; }
  .header-row { display: grid; grid-template-columns: minmax(0, 2fr) minmax(0, 3fr) 30px; gap: 7px; }
  .header-row button { display: grid; place-items: center; color: #dc2626; border-radius: 6px; }
  .header-row button:hover { background: rgba(239,68,68,.1); }
  .empty, .hint { color: var(--text-muted); font-size: 11px; }
  .toggle-cell { justify-content: flex-end; }
  .short { width: 150px; }
  .save-footer { display: flex; align-items: center; justify-content: flex-end; min-height: 38px; gap: 12px; }
  .save-footer span { color: #15803d; font-size: 12px; }
  .save-footer span.error { color: #b91c1c; }
  .save { display: flex; align-items: center; gap: 7px; height: 34px; padding: 0 18px; border-radius: 8px; color: #fff; background: var(--primary-color); font-weight: 600; }
  .save:hover:not(:disabled) { background: var(--primary-hover); }
  .save:disabled { opacity: .55; cursor: wait; }
</style>
