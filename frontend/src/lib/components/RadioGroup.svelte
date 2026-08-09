<script lang="ts">
  let { options = [], value = "", label, disabled = false, onchange }: {
    options: Array<{ value: string; label: string }>; value?: string; label?: string;
    disabled?: boolean; onchange?: (value: string) => void;
  } = $props();
</script>

<fieldset class="radio-group" {disabled}>
  {#if label}<legend>{label}</legend>{/if}
  <div class="options">
    {#each options as option}
      <label class:active={value === option.value}>
        <input type="radio" name={label ?? "option"} checked={value === option.value} onchange={() => onchange?.(option.value)} />
        <span>{option.label}</span>
      </label>
    {/each}
  </div>
</fieldset>

<style>
  fieldset { border: 0; min-width: 0; }
  legend { margin-bottom: 8px; font-size: 13px; font-weight: 500; }
  .options { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 6px; }
  label { padding: 7px 5px; border: 1px solid var(--border-input); border-radius: 6px; color: var(--text-secondary); text-align: center; cursor: pointer; }
  label:hover { background: rgba(0,0,0,.035); }
  label.active { border-color: var(--primary-color); color: var(--primary-hover); background: var(--primary-glow); font-weight: 600; }
  input { position: absolute; opacity: 0; }
</style>
