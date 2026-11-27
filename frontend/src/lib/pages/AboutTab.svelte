<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { Download, ExternalLink, RefreshCw } from "lucide-svelte";
  import {
    checkUpdate,
    getAppInfo,
    installUpdate,
    onUpdateProgress,
    openUrl,
    type UpdateCheck,
  } from "$lib/api/tauri-api";

  const AUTHOR_URL = "https://github.com/baiwusanyu-c";
  const PROJECT_URL = "https://github.com/baiwusanyu-c/ikun-desktop-organization";
  const RELEASES_URL = `${PROJECT_URL}/releases`;

  /**
   * 更新流程状态机。
   *
   * `installing` 是终态：Rust 侧随后触发优雅退出，进程结束，不会再有后续回调。
   */
  type UpdatePhase =
    | "idle"
    | "checking"
    | "upToDate"
    | "available"
    | "downloading"
    | "installing"
    | "error"
    | "notConfigured";

  let version = $state("0.1.0");
  let phase = $state<UpdatePhase>("idle");
  let message = $state("");
  let updateInfo = $state<UpdateCheck | null>(null);
  let percent = $state(0);
  let unlistenProgress: (() => void) | null = null;
  let disposed = false;

  const busy = $derived(
    phase === "checking" || phase === "downloading" || phase === "installing",
  );
  const downloadPage = $derived(updateInfo?.download_page ?? RELEASES_URL);

  onMount(() => {
    getAppInfo()
      .then((info) => { if (!disposed) version = info.version; })
      .catch(() => undefined);
  });

  onDestroy(() => {
    disposed = true;
    stopProgressListener();
  });

  async function openExternal(url: string) {
    try { await openUrl(url); }
    catch { window.open(url, "_blank", "noopener,noreferrer"); }
  }

  function stopProgressListener() {
    unlistenProgress?.();
    unlistenProgress = null;
  }

  async function ensureProgressListener() {
    try {
      const unlisten = await onUpdateProgress((progress) => {
        if (!disposed) percent = progress.percent;
      });
      if (disposed) {
        unlisten();
        return;
      }
      unlistenProgress = unlisten;
    } catch {
      // 进度展示不是安装的前置条件；订阅失败仍继续，但明确告知用户
      if (!disposed) message = "正在下载更新包（当前无法获取下载进度）…";
    }
  }

  async function handleCheckUpdate() {
    if (busy) return;
    phase = "checking";
    message = "";
    updateInfo = null;
    try {
      const result = await checkUpdate();
      if (disposed) return;
      updateInfo = result;
      if (result.available) {
        phase = "available";
        message = `发现新版本 v${result.latest_version ?? ""}（当前 v${result.current_version}）`;
      } else {
        phase = "upToDate";
        message = `已是最新版本 v${result.current_version}`;
      }
    } catch (error) {
      if (disposed) return;
      // Rust 侧已把网络 / 清单缺失 / 公钥未配置区分为可读中文，这里直接呈现
      const text = String(error);
      phase = text.includes("未配置签名公钥") ? "notConfigured" : "error";
      message = text;
    }
  }

  async function handleInstallUpdate() {
    if (busy) return;
    phase = "downloading";
    percent = 0;
    message = "正在下载更新包…";
    stopProgressListener();

    // 必须先完成监听注册再触发下载，否则慢 IPC 下会漏掉最早一批进度事件。
    await ensureProgressListener();
    if (disposed) return;

    try {
      const result = await installUpdate();
      stopProgressListener();
      if (disposed) return;
      phase = "installing";
      message = result || "更新包已校验，应用即将退出并完成安装…";
    } catch (error) {
      stopProgressListener();
      if (disposed) return;
      phase = "error";
      message = String(error);
    }
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
    <div class="row">
      <span class="label">版本号</span>
      <div class="value">
        <span>v{version}</span>
        <button type="button" class="check" onclick={handleCheckUpdate} disabled={busy}>
          <RefreshCw size={12} class={phase === "checking" ? "spinning" : ""} />
          {phase === "checking" ? "检查中" : "检查更新"}
        </button>
      </div>
    </div>
    {#if phase !== "idle"}
      <div class="update-status" class:error={phase === "error" || phase === "notConfigured"} role="status" aria-live="polite">
        <p class="message">{message}</p>
        {#if phase === "downloading"}
          <div
            class="progress"
            role="progressbar"
            aria-label="更新包下载进度"
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow={percent}
          ><span style={`width: ${percent}%`}></span></div>
        {/if}
        {#if phase === "available" && updateInfo?.notes}
          <pre class="notes">{updateInfo.notes}</pre>
        {/if}
        {#if phase === "available"}
          <p class="hint">点击「立即升级」后应用会退出并完成安装，随后自动重启；桌面图标与分类框设置都会保留。</p>
        {/if}
        {#if phase === "installing"}
          <p class="hint">安装期间不要断电；完成后会自动启动新版本，桌面图标与分类框设置都会保留。</p>
        {/if}
        <div class="actions">
          {#if phase === "available"}
            <button type="button" class="apply" onclick={handleInstallUpdate}>
              <Download size={12} />立即升级
            </button>
          {/if}
          {#if phase === "available" || phase === "error" || phase === "notConfigured"}
            <button type="button" class="link" onclick={() => openExternal(downloadPage)}>
              前往下载页 <ExternalLink size={12} />
            </button>
          {/if}
        </div>
      </div>
    {/if}
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
  .update-status { display: flex; flex-direction: column; gap: 7px; padding: 10px 15px; border-bottom: 1px solid var(--border-group); background: var(--primary-glow); }
  .update-status.error { background: rgba(214, 61, 61, .1); }
  .message { color: var(--text-secondary); font-size: 11px; line-height: 1.5; word-break: break-word; }
  .update-status.error .message { color: #d63d3d; }
  .progress { height: 4px; border-radius: 2px; background: var(--border-group); overflow: hidden; }
  .progress span { display: block; height: 100%; border-radius: 2px; background: var(--primary-color); transition: width 220ms ease-out; }
  .notes { max-height: 92px; overflow-y: auto; margin: 0; padding: 7px 9px; border-radius: 6px; background: var(--bg-group); color: var(--text-muted); font-size: 10px; line-height: 1.5; white-space: pre-wrap; word-break: break-word; }
  .hint { color: var(--text-muted); font-size: 10px; line-height: 1.5; }
  .actions { display: flex; align-items: center; justify-content: flex-end; gap: 11px; }
  .apply { display: flex; align-items: center; gap: 4px; padding: 5px 10px; border: 1px solid var(--primary-shadow); border-radius: 6px; color: #fff; background: var(--primary-color); font-size: 11px; font-weight: 600; }
  .apply:hover { color: #fff; background: var(--primary-hover); }
  :global(.spinning) { animation: spin 800ms linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
