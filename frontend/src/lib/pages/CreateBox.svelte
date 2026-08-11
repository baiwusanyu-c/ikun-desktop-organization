<script lang="ts">
  import { onMount } from "svelte";
  import TitleBar from "$lib/components/TitleBar.svelte";
  import { createBox, validateBoxName } from "$lib/api/tauri-api";
  import { installThemeColorSync } from "$lib/theme";
  import { hideCurrentWindow, installHideOnClose } from "$lib/window-lifecycle";

  const MAX_NAME_LENGTH = 16;
  let name = $state("");
  let errorMessage = $state("");
  let isLoading = $state(false);
  let inputElement: HTMLInputElement;
  let characterCount = $derived(Array.from(name).length);

  function reset() { name = ""; errorMessage = ""; isLoading = false; }
  async function cancel() { reset(); await hideCurrentWindow(); }

  async function confirm() {
    if (isLoading) return;
    const trimmed = name.trim();
    if (!trimmed) { errorMessage = "名称不能为空"; return; }
    if (Array.from(trimmed).length > MAX_NAME_LENGTH) { errorMessage = `名称不能超过 ${MAX_NAME_LENGTH} 个字符`; return; }
    isLoading = true;
    errorMessage = "";
    try {
      const result = await validateBoxName(trimmed);
      if (!result.valid) { errorMessage = result.message; return; }
      await createBox(trimmed);
      reset();
      await hideCurrentWindow();
    } catch (error) { errorMessage = String(error); }
    finally { isLoading = false; }
  }

  function handleInput(event: Event) {
    const next = (event.currentTarget as HTMLInputElement).value;
    name = Array.from(next).slice(0, MAX_NAME_LENGTH).join("");
    errorMessage = "";
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === "Enter") { event.preventDefault(); void confirm(); }
    if (event.key === "Escape") { event.preventDefault(); void cancel(); }
  }

  onMount(() => {
    const removeCloseHandler = installHideOnClose(reset);
    const removeThemeSync = installThemeColorSync();
    requestAnimationFrame(() => inputElement?.focus());
    return () => {
      removeThemeSync();
      removeCloseHandler();
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />
<div class="window dialog-window">
  <TitleBar title="创建分类框" showMinimize={false} onClose={cancel} />
  <div class="form-body">
    <div class="label-row"><label for="create-name">分类名称</label><span class:limit={characterCount === MAX_NAME_LENGTH}>{characterCount}/{MAX_NAME_LENGTH}</span></div>
    <input bind:this={inputElement} id="create-name" value={name} oninput={handleInput} maxlength={MAX_NAME_LENGTH} class:error={!!errorMessage} placeholder="请输入分类名称..." disabled={isLoading} autocomplete="off" />
    <div class="message" class:error={!!errorMessage} role="status">{errorMessage || " "}</div>
    <footer><button type="button" class="secondary" onclick={cancel} disabled={isLoading}>取消</button><button type="button" class="primary-button" onclick={confirm} disabled={!name.trim() || isLoading}>{isLoading ? "创建中..." : "确认"}</button></footer>
  </div>
</div>
