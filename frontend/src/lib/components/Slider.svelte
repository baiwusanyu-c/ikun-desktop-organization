<script lang="ts">
  let { value = 50, min = 0, max = 100, step = 1, label, suffix = "", digits, disabled = false, onchange }: {
    value?: number; min?: number; max?: number; step?: number; label?: string; suffix?: string;
    digits?: number; disabled?: boolean; onchange?: (value: number) => void;
  } = $props();
  // Snap to nearest step using integer arithmetic to eliminate floating-point drift
  let stepDecimals = $derived((String(step).split('.')[1] || '').length);
  let factor = $derived(Math.pow(10, stepDecimals));
  let snapped = $derived(
    Math.round(Math.round((value - min) * factor) / Math.round(step * factor)) * step + min
  );
  let progress = $derived(((snapped - min) / (max - min)) * 100);
  let displayValue = $derived(digits === undefined ? String(value) : value.toFixed(digits));
</script>

<label class="slider" class:disabled>
  {#if label}<span class="slider-header"><span>{label}</span><output>{displayValue}{suffix}</output></span>{/if}
  <input type="range" bind:value={value} {min} {max} {step} {disabled} oninput={() => onchange?.(value)} style="--progress: {progress}%" />
</label>

<style>
  .slider { display: flex; flex-direction: column; gap: 7px; width: 100%; color: var(--text-primary); }
  .slider.disabled { opacity: 0.42; }
  .slider-header { display: flex; justify-content: space-between; align-items: center; font-size: 13px; font-weight: 500; }
  output { min-width: 55px; color: var(--text-secondary); font: 12px ui-monospace, Consolas, monospace; text-align: right; }
  input { appearance: none; width: 100%; height: 18px; padding: 7px 0; border: 0; box-shadow: none; background: linear-gradient(to right, var(--primary-color) 0 var(--progress), rgba(0,0,0,.1) var(--progress) 100%) content-box; cursor: pointer; }
  input::-webkit-slider-thumb { appearance: none; width: 15px; height: 15px; margin-top: -5px; border: 2px solid #fff; border-radius: 50%; background: var(--primary-color); box-shadow: 0 1px 4px rgba(0,0,0,.22); }
</style>