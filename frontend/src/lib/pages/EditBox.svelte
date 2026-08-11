<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import TitleBar from "$lib/components/TitleBar.svelte";
  import { editBox, getPendingEditBox, validateBoxName, type EditBoxPayload } from "$lib/api/tauri-api";
  import { installThemeColorSync } from "$lib/theme";
  import { hideCurrentWindow, installHideOnClose } from "$lib/window-lifecycle";

  const MAX_NAME_LENGTH = 16;
  let configId = $state("");
  let originalName = $state("");
  let name = $state("");
  let errorMessage = $state("");
  let isLoading = $state(false);
  let inputElement: HTMLInputElement;
  let characterCount = $derived(Array.from(name).length);

  function clearTransientState() { errorMessage = ""; isLoading = false; }
  function applyPayload(payload: EditBoxPayload) {
    configId = payload.config_id;
    originalName = payload.current_name;
    name = payload.current_name;
    errorMessage = "";
    requestAnimationFrame(() => { inputElement?.focus(); inputElement?.select(); });
  }
  async function cancel() { clearTransientState(); await hideCurrentWindow(); }

  async function confirm() {
    if (isLoading) return;
    const trimmed = name.trim();
    if (!configId) { errorMessage = "未收到分类框信息，请重新打开编辑窗口"; return; }
    if (!trimmed) { errorMessage = "名称不能为空"; return; }
    if (Array.from(trimmed).length > MAX_NAME_LENGTH) { errorMessage = `名称不能超过 ${MAX_NAME_LENGTH} 个字符`; return; }
    isLoading = true;
    errorMessage = "";
    try {
      if (trimmed !== originalName) {
        const result = await validateBoxName(trimmed);
        if (!result.valid) { errorMessage = result.message; return; }
      }
      await editBox(configId, trimmed);
      originalName = trimmed;
      await hideCurrentWindow();
    } catch (error) { errorMessage = String(error); }
    finally { isLoading = false; }
  }

  function handleInput(event: Event) {
    name = Array.from((event.currentTarget as HTMLInputElement).value).slice(0, MAX_NAME_LENGTH).join("");
    errorMessage = "";
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === "Enter") { event.preventDefault(); void confirm(); }
    if (event.key === "Escape") { event.preventDefault(); void cancel(); }
  }

  onMount(() => {
    const removeCloseHandler = installHideOnClose(clearTransientState);
    const removeThemeSync = installThemeColorSync();
    let removeEventListener: (() => void) | undefined;
    let disposed = false;
    listen<EditBoxPayload>("edit-box-opened", ({ payload }) => applyPayload(payload))
      .then((cleanup) => {
        if (disposed) cleanup();
        else {
          removeEventListener = cleanup;
          getPendingEditBox().then((payload) => { if (!disposed && payload) applyPayload(payload); }).catch(() => undefined);
        }
      });
    return () => {
      disposed = true;
      removeEventListener?.();
      removeThemeSync();
      removeCloseHandler();
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />
<div class="window dialog-window">
  <TitleBar title="编辑分类框" showMinimize={false} onClose={cancel} />
  <div class="form-body">
    <div class="label-row"><label for="edit-name">分类名称</label><span class:limit={characterCount === MAX_NAME_LENGTH}>{characterCount}/{MAX_NAME_LENGTH}</span></div>
    <input bind:this={inputElement} id="edit-name" value={name} oninput={handleInput} maxlength={MAX_NAME_LENGTH} class:error={!!errorMessage} placeholder="请输入分类名称..." disabled={isLoading || !configId} autocomplete="off" />
    <div class="message" class:error={!!errorMessage} role="status">{errorMessage || (!configId ? "等待分类框数据..." : " ")}</div>
    <footer><button type="button" class="secondary" onclick={cancel} disabled={isLoading}>取消</button><button type="button" class="primary-button" onclick={confirm} disabled={!configId || !name.trim() || isLoading}>{isLoading ? "保存中..." : "确认"}</button></footer>
  </div>
</div>
