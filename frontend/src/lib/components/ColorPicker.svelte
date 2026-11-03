<script lang="ts">
  import { tick } from "svelte";

  let { value = "#14b8a6", onchange }: { value?: string; onchange?: (color: string) => void } = $props();
  const presets = ["#14b8a6", "#0ea5e9", "#2563eb", "#8b5cf6", "#ec4899", "#ef4444", "#f59e0b", "#22c55e"];
  let isOpen = $state(false);
  // popover: fixed + moved to document.body, see bodyPortal() below
  let panelTop = $state(0);
  let panelLeft = $state(0);
  let swatchEl: HTMLButtonElement | undefined = $state();
  let hexInput = $state("");
  $effect(() => { hexInput = value; });

  // The popover must escape its ancestor chain to use viewport coordinates:
  // two ancestors silently become the containing block for a fixed descendant
  // -- .window-shadow { filter: drop-shadow } (measured +8px) and
  // .fields { contain: layout style } (measured +271px). A mis-placed popover
  // inside the scroller distorts the scrollable range, which is what showed up
  // as 'clicking the swatch does nothing' and 'blank panel bottom / the
  // scrollbar stops responding'. Re-parenting to <body> removes both.
  function bodyPortal(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        // Detach here: Svelte's teardown walks the original fragment, which no
        // longer contains this node. Re-inserting it would resurrect it (leaked
        // popover that can never be closed again).
        node.remove();
      },
    };
  }

  async function toggleOpen() {
    isOpen = !isOpen;
    if (isOpen && swatchEl) {
      await tick();
      const rect = swatchEl.getBoundingClientRect();
      panelTop = rect.bottom + 8;
      panelLeft = Math.max(8, Math.min(window.innerWidth - 198, rect.right - 190));
    }
  }

  function commitHex() {
    const normalized = hexInput.trim().startsWith("#") ? hexInput.trim() : `#${hexInput.trim()}`;
    if (/^#[0-9a-f]{6}$/i.test(normalized)) onchange?.(normalized.toLowerCase());
    else hexInput = value;
  }
</script>

<svelte:window onclick={() => isOpen = false} />
<div class="color-picker">
  <button type="button" class="swatch" bind:this={swatchEl} style:background-color={value} onclick={(event) => { event.stopPropagation(); void toggleOpen(); }} title={`选择颜色 ${value}`} aria-label={`选择颜色 ${value}`}></button>
  {#if isOpen}
    <div class="panel" use:bodyPortal style:top={`${panelTop}px`} style:left={`${panelLeft}px`} role="dialog" aria-label="颜色选择器" tabindex="-1" onclick={(event) => event.stopPropagation()} onkeydown={(event) => { if (event.key === "Escape") isOpen = false; }}>
      <div class="presets">
        {#each presets as color}
          <button type="button" class="preset" class:active={value.toLowerCase() === color} style:background-color={color} onclick={() => onchange?.(color)} title={color} aria-label={color}></button>
        {/each}
      </div>
      <div class="hex-row">
        <span>HEX</span>
        <input aria-label="十六进制颜色" bind:value={hexInput} maxlength="7" spellcheck={false} onblur={commitHex} onkeydown={(event) => { if (event.key === "Enter") commitHex(); }} />
        <input class="native" aria-label="系统颜色选择器" type="color" value={value} oninput={(event) => onchange?.(event.currentTarget.value)} />
      </div>
    </div>
  {/if}
</div>

<style>
  .color-picker { position: relative; }
  .swatch { width: 34px; height: 28px; border: 2px solid rgba(255,255,255,.9); border-radius: 6px; box-shadow: 0 0 0 1px var(--border-input), 0 1px 3px rgba(0,0,0,.15); }
  .panel { position: fixed; z-index: 20; width: 190px; padding: 10px; border: 1px solid var(--border-input); border-radius: 8px; background: rgba(255,255,255,.97); box-shadow: 0 8px 28px rgba(0,0,0,.18); }
  .presets { display: grid; grid-template-columns: repeat(4, 1fr); gap: 7px; margin-bottom: 10px; }
  .preset { width: 34px; height: 25px; justify-self: center; border: 2px solid transparent; border-radius: 5px; }
  .preset.active { border-color: #1d1d1f; }
  .hex-row { display: grid; grid-template-columns: auto 1fr 28px; align-items: center; gap: 6px; color: var(--text-secondary); font-size: 10px; font-weight: 700; }
  .hex-row input { min-width: 0; height: 28px; padding: 4px 6px; font: 12px ui-monospace, Consolas, monospace; }
  .hex-row .native { width: 28px; padding: 1px; }
</style>
