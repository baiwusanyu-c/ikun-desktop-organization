<script lang="ts">
  import { Eye, EyeOff, Save } from "lucide-svelte";
  import Toggle from "$lib/components/Toggle.svelte";
  import { updateAiConfig, type AiConfig, type AppConfig } from "$lib/api/tauri-api";

  let { config }: { config: AppConfig } = $props();
  let ai = $state<AiConfig>({} as AiConfig);
  let ready = $state(false);
  let showApiKey = $state(false);
  let saveState = $state<"idle" | "saving" | "saved" | "error">("idle");
  let message = $state("");
  let loadedConfig: AppConfig | undefined;

  $effect(() => {
    if (config === loadedConfig) return;
    loadedConfig = config;
    ai = $state.snapshot(config.ai);
    ready = true;
  });

  async function save() {
    saveState = "saving";
    message = "";
    try {
      await updateAiConfig($state.snapshot(ai));
      saveState = "saved";
      message = "AI 设置已保存";
      setTimeout(() => { if (saveState === "saved") saveState = "idle"; }, 1800);
    } catch (error) {
      saveState = "error";
      message = "保存失败：" + String(error);
    }
  }
</script>

{#if ready}
  <div class="ai-settings">
    <section class="group info">
      <header><h2>API 配置说明</h2></header>
      <div class="body">
        <p class="hint">请填写任意兼容 OpenAI 格式的 LLM API 配置。支持以下常见服务商：</p>
        <ul class="providers">
          <li><a href="https://platform.openai.com/api-keys" target="_blank" rel="noopener">OpenAI</a> — base_url: <code>https://api.openai.com/v1</code></li>
          <li><a href="https://platform.deepseek.com/" target="_blank" rel="noopener">DeepSeek</a> — base_url: <code>https://api.deepseek.com</code></li>
          <li><a href="https://open.bigmodel.cn/" target="_blank" rel="noopener">智谱 AI (GLM)</a> — base_url: <code>https://open.bigmodel.cn/api/paas/v4</code></li>
          <li><a href="https://dashscope.console.aliyun.com/" target="_blank" rel="noopener">阿里云百炼 (Qwen)</a> — base_url: <code>https://dashscope.aliyuncs.com/compatible-mode/v1</code></li>
        </ul>
      </div>
    </section>

    <section class="group">
      <header><h2>基本设置</h2></header>
      <div class="body fields">
        <label for="base-url">API 地址 <span class="required">*</span></label>
        <input id="base-url" type="url" bind:value={ai.base_url} placeholder="https://api.openai.com/v1" required />
        <label for="api-key">API Key <span class="required">*</span></label>
        <div class="password">
          <input id="api-key" type={showApiKey ? "text" : "password"} bind:value={ai.api_key} autocomplete="off" placeholder="sk-..." required />
          <button type="button" onclick={() => showApiKey = !showApiKey} title={showApiKey ? "隐藏 API Key" : "显示 API Key"} aria-label={showApiKey ? "隐藏 API Key" : "显示 API Key"}>
            {#if showApiKey}<EyeOff size={15} />{:else}<Eye size={15} />{/if}
          </button>
        </div>
        <label for="model">模型名称 <span class="required">*</span></label>
        <input id="model" bind:value={ai.model} placeholder="gpt-4o-mini" required />
        <div class="two-column">
          <div>
            <label for="timeout">请求超时（秒）</label>
            <input id="timeout" type="number" bind:value={ai.timeout_seconds} min="5" max="120" />
          </div>
          <div>
            <label for="retries">最大重试次数</label>
            <input id="retries" type="number" bind:value={ai.max_retries} min="0" max="5" />
          </div>
        </div>
      </div>
    </section>

    <footer class="save-footer">
      {#if message}<span class:error={saveState === "error"} role="status">{message}</span>{/if}
      <button type="button" class="save" onclick={save} disabled={saveState === "saving"}>
        <Save size={15} />{saveState === "saving" ? "保存中..." : "保存"}
      </button>
    </footer>
  </div>
{/if}

<style>
  .ai-settings { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 20px; min-height: 100%; background: #ffffff; }
  .group { overflow: hidden; }
  .group.info { border: 1px solid var(--primary-color); border-radius: 8px; }
  header { min-height: 43px; display: flex; align-items: center; padding: 10px 14px; border-bottom: 1px solid var(--border-group); background: var(--bg-group-header); }
  h2 { font-size: 13px; font-weight: 650; }
  .body { padding: 13px 15px; }
  .fields { display: flex; flex-direction: column; gap: 9px; }
  label, .row > span { color: var(--text-secondary); font-size: 12px; font-weight: 600; }
  .required { color: #dc2626; }
  input { width: 100%; padding: 8px 10px; }
  .row { display: flex; align-items: center; justify-content: space-between; min-height: 28px; }
  .two-column { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .two-column > div { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .password { position: relative; }
  .password input { padding-right: 38px; }
  .password button { position: absolute; right: 4px; top: 4px; width: 30px; height: 28px; display: grid; place-items: center; color: var(--text-muted); border-radius: 5px; }
  .password button:hover { background: rgba(0,0,0,.06); }
  .hint { color: var(--text-muted); font-size: 11px; line-height: 1.6; margin: 0 0 8px 0; }
  .providers { margin: 0; padding-left: 18px; list-style: none; }
  .providers li { font-size: 11px; line-height: 1.8; color: var(--text-secondary); }
  .providers li::before { content: "•"; margin-right: 6px; color: var(--primary-color); }
  .providers a { color: var(--primary-color); text-decoration: none; font-weight: 600; }
  .providers a:hover { text-decoration: underline; }
  .providers code { font-size: 10px; background: rgba(0,0,0,.06); padding: 1px 5px; border-radius: 3px; color: var(--text-muted); }
  .save-footer { display: flex; align-items: center; justify-content: flex-end; min-height: 38px; gap: 12px; }
  .save-footer span { color: #15803d; font-size: 12px; }
  .save-footer span.error { color: #b91c1c; }
  .save { display: flex; align-items: center; gap: 7px; height: 34px; padding: 0 18px; border-radius: 8px; color: #fff; background: var(--primary-color); font-weight: 600; }
  .save:hover:not(:disabled) { background: var(--primary-hover); }
  .save:disabled { opacity: .55; cursor: wait; }
</style>
