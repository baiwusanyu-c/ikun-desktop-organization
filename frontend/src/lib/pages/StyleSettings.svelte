<script lang="ts">
  import { Droplets, Palette, SquareDashed } from "lucide-svelte";
  import ColorPicker from "$lib/components/ColorPicker.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Toggle from "$lib/components/Toggle.svelte";
  import { updateBoxStyle, updateLiquidGlass, updateThemeColor, type AppConfig, type BoxStyleConfig, type LiquidGlassConfig } from "$lib/api/tauri-api";

  let { config }: { config: AppConfig } = $props();
  let themeColor = $state("#14b8a6");
  let boxStyle = $state<BoxStyleConfig>({} as BoxStyleConfig);
  let ready = $state(false);
  let errorMessage = $state("");
  let loadedConfig: AppConfig | undefined;
  let updateQueue = Promise.resolve();

  const refractionOptions = [
    { value: "0", label: "标准" },
    { value: "1", label: "极坐标" },
    { value: "2", label: "凸起" },
    { value: "3", label: "程序化" },
  ];

  $effect(() => {
    if (config === loadedConfig) return;
    loadedConfig = config;
    themeColor = config.ui_theme.light.primary ?? "#14b8a6";
    boxStyle = $state.snapshot(config.box_style);
    ready = true;
  });

  function rgbToHex(rgb: [number, number, number]): string {
    return `#${rgb.map((part) => Math.round(part).toString(16).padStart(2, "0")).join("")}`;
  }

  function hexToRgb(hex: string): [number, number, number] {
    return [Number.parseInt(hex.slice(1, 3), 16), Number.parseInt(hex.slice(3, 5), 16), Number.parseInt(hex.slice(5, 7), 16)];
  }

  function runUpdate(action: () => Promise<void>) {
    errorMessage = "";
    updateQueue = updateQueue.then(action).catch((error) => {
      errorMessage = `设置未能生效：${String(error)}`;
    });
  }

  function setThemeColor(color: string) {
    themeColor = color;
    document.documentElement.style.setProperty("--primary-color", color);
    runUpdate(() => updateThemeColor(color));
  }

  function setBoxField<K extends keyof BoxStyleConfig>(key: K, value: BoxStyleConfig[K]) {
    boxStyle = { ...boxStyle, [key]: value };
    runUpdate(() => updateBoxStyle($state.snapshot(boxStyle)));
  }

  function setGlassField<K extends keyof LiquidGlassConfig>(key: K, value: LiquidGlassConfig[K]) {
    const liquidGlass = { ...boxStyle.liquid_glass, [key]: value };
    boxStyle = { ...boxStyle, liquid_glass: liquidGlass };
    runUpdate(() => updateLiquidGlass($state.snapshot(liquidGlass)));
  }
</script>

{#if ready}
  <div class="settings">
    {#if errorMessage}<div class="error" role="alert">{errorMessage}</div>{/if}

    <section class="group">
      <header><Palette size={17} /><div><h2>主题色</h2><p>影响编辑、创建、控制面板及分类框菜单选中效果</p></div></header>
      <div class="body row">
        <span>主题色</span>
        <div class="color-value"><code>{themeColor}</code><ColorPicker value={themeColor} onchange={setThemeColor} /></div>
      </div>
    </section>

    <section class="group">
      <header><SquareDashed size={17} /><h2>分类框风格</h2></header>
      <div class="body fields">
        <Slider label="圆角调节" value={boxStyle.border_radius} min={0} max={30} step={1} suffix="px" onchange={(value) => setBoxField("border_radius", value)} />
        <div class="row"><span>背景色</span><div class="color-value"><code>{rgbToHex(boxStyle.background_color)}</code><ColorPicker value={rgbToHex(boxStyle.background_color)} onchange={(value) => setBoxField("background_color", hexToRgb(value))} /></div></div>
        <Slider label="透明度" value={Math.round(boxStyle.background_alpha / 255 * 100)} min={0} max={100} suffix="%" onchange={(value) => setBoxField("background_alpha", Math.round(value * 2.55))} />
        <div class="row"><span>纯图标模式（隐藏分类名称）</span><Toggle checked={boxStyle.icon_only_mode} onchange={(value) => setBoxField("icon_only_mode", value)} /></div>
      </div>
    </section>

    <section class="group">
      <header><Droplets size={17} /><div><h2>液态玻璃效果</h2><p>开启后会增加 GPU 功耗</p></div></header>
      <div class="body fields">
        <div class="row"><span>开启液态玻璃效果</span><Toggle checked={boxStyle.liquid_glass.enabled} onchange={(value) => setGlassField("enabled", value)} /></div>
        <div class:disabled={!boxStyle.liquid_glass.enabled} class="glass-fields">
          <Slider label="位移强度" value={boxStyle.liquid_glass.displacement_scale} min={0} max={100} step={1} disabled={!boxStyle.liquid_glass.enabled} onchange={(value) => setGlassField("displacement_scale", value)} />
          <Slider label="模糊程度" value={boxStyle.liquid_glass.blur_amount} min={0} max={1} step={0.01} digits={2} disabled={!boxStyle.liquid_glass.enabled} onchange={(value) => setGlassField("blur_amount", value)} />
          <Slider label="饱和度" value={boxStyle.liquid_glass.saturation} min={1} max={3} step={0.1} digits={1} disabled={!boxStyle.liquid_glass.enabled} onchange={(value) => setGlassField("saturation", value)} />
          <Slider label="色差偏移" value={boxStyle.liquid_glass.chromatic_offset} min={0} max={20} step={1} disabled={!boxStyle.liquid_glass.enabled} onchange={(value) => setGlassField("chromatic_offset", value)} />
          <Slider label="弹性形变" value={boxStyle.liquid_glass.elasticity} min={0} max={0.2} step={0.001} digits={3} disabled={!boxStyle.liquid_glass.enabled} onchange={(value) => setGlassField("elasticity", value)} />
          <RadioGroup label="折射贴图" options={refractionOptions} value={String(boxStyle.liquid_glass.refraction_mode)} disabled={!boxStyle.liquid_glass.enabled} onchange={(value) => setGlassField("refraction_mode", Number(value))} />
          <div class="row"><span>亮区减弱折射</span><Toggle checked={boxStyle.liquid_glass.over_light} disabled={!boxStyle.liquid_glass.enabled} onchange={(value) => setGlassField("over_light", value)} /></div>
        </div>
      </div>
    </section>
  </div>
{/if}

<style>
  .settings { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 20px; }
  .group { overflow: visible; }
  header { min-height: 45px; display: flex; align-items: center; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--border-group); background: var(--bg-group-header); }
  header :global(svg) { color: var(--primary-color); flex: 0 0 auto; }
  h2 { font-size: 13px; line-height: 1.3; font-weight: 650; }
  p { margin-top: 2px; color: var(--text-muted); font-size: 11px; line-height: 1.35; }
  .body { padding: 13px 15px; }
  .fields, .glass-fields { display: flex; flex-direction: column; gap: 14px; }
  .glass-fields { padding-top: 14px; transition: opacity 150ms ease; }
  .glass-fields.disabled { opacity: .48; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 16px; font-size: 13px; font-weight: 500; }
  .color-value { display: flex; align-items: center; gap: 9px; }
  code { color: var(--text-secondary); font: 12px ui-monospace, Consolas, monospace; }
  .error { padding: 9px 11px; border: 1px solid rgba(239,68,68,.25); border-radius: 7px; background: rgba(239,68,68,.08); color: #b91c1c; font-size: 12px; }
</style>
