<script lang="ts">
  let { checked = false, label, disabled = false, onchange }: {
    checked?: boolean; label?: string; disabled?: boolean; onchange?: (checked: boolean) => void;
  } = $props();
</script>

<label class="toggle-row" class:disabled>
  <input class="native" type="checkbox" {checked} {disabled} onchange={(event) => onchange?.(event.currentTarget.checked)} />
  <span class="toggle" aria-hidden="true"><span class="thumb"></span></span>
  {#if label}<span class="label">{label}</span>{/if}
</label>

<style>
  .toggle-row { display: inline-flex; align-items: center; gap: 9px; cursor: pointer; }
  .toggle-row.disabled { opacity: .42; cursor: not-allowed; }
  .native { position: absolute; width: 1px; height: 1px; opacity: 0; }
  .toggle { position: relative; width: 38px; height: 22px; flex: 0 0 38px; border-radius: 11px; background: rgba(0,0,0,.14); transition: background 180ms ease; }
  .thumb { position: absolute; left: 2px; top: 2px; width: 18px; height: 18px; border-radius: 50%; background: #fff; box-shadow: 0 1px 3px rgba(0,0,0,.25); transition: transform 180ms ease; }
  .native:checked + .toggle { background: var(--primary-color); }
  .native:checked + .toggle .thumb { transform: translateX(16px); }
  .native:focus-visible + .toggle { outline: 2px solid var(--primary-color); outline-offset: 2px; }
  .label { font-size: 13px; color: var(--text-primary); }
</style>
