<script lang="ts">
  import { Droplets, Palette, SquareDashed } from "lucide-svelte";
  import ColorPicker from "$lib/components/ColorPicker.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Toggle from "$lib/components/Toggle.svelte";
  import {
    updateBoxStyle, updateLiquidGlass, updateThemeColor,
    type AppConfig, type BoxStyleConfig, type LiquidGlassConfig,
    type GlassLiquidParams, type GlassTransparentParams,
    type GlassResizeConfig, type GlassResizeFeedback, type GlassEngineConfig,
  } from "$lib/api/tauri-api";
  import { applyThemeColorAcrossWindows } from "$lib/theme";

  let { config }: { config: AppConfig } = $props();
  let themeColor = $state("#14b8a6");
  let boxStyle = $state<BoxStyleConfig>({} as BoxStyleConfig);
  let ready = $state(false);
  let errorMessage = $state("");
  let loadedConfig: AppConfig | undefined;
  let updateQueue = Promise.resolve();

  const shapeOptions = [
    { value: "RoundedRect", label: "圆角矩形" },
    { value: "Squircle", label: "超椭圆" },
  ];
  const profileOptions = [
    { value: "Circular", label: "圆形" },
    { value: "Parabolic", label: "抛物线" },
    { value: "Sinusoidal", label: "正弦" },
  ];
  const backendOptions = [
    { value: "Auto", label: "自动" },
    { value: "ForceDda", label: "DDA" },
    { value: "ForceWgc", label: "WGC" },
  ];

  $effect(() => {
    if (config === loadedConfig) return;
    loadedConfig = config;
    themeColor = config.ui_theme.light.primary ?? "#14b8a6";
    boxStyle = $state.snapshot(config.box_style);
    ready = true;
  });

  // --- color conversion helpers (0~1 float arrays <-> hex) ---

  function rgbFloatToHex(rgb: readonly [number, number, number]): string {
    return "#" + rgb.map((c) =>
      Math.round(Math.min(1, Math.max(0, c)) * 255).toString(16).padStart(2, "0")
    ).join("");
  }

  function rgbaFloatToHex(rgba: readonly [number, number, number, number]): string {
    return rgbFloatToHex([rgba[0], rgba[1], rgba[2]]);
  }

  function hexToRgbFloat(hex: string): [number, number, number] {
    return [
      Number.parseInt(hex.slice(1, 3), 16) / 255,
      Number.parseInt(hex.slice(3, 5), 16) / 255,
      Number.parseInt(hex.slice(5, 7), 16) / 255,
    ];
  }

  // original 0~255 helpers (used for box background_color)
  function rgbToHex(rgb: [number, number, number]): string {
    return "#" + rgb.map((part) => Math.round(part).toString(16).padStart(2, "0")).join("");
  }

  function hexToRgb(hex: string): [number, number, number] {
    return [
      Number.parseInt(hex.slice(1, 3), 16),
      Number.parseInt(hex.slice(3, 5), 16),
      Number.parseInt(hex.slice(5, 7), 16),
    ];
  }

  // --- update queue ---

  function runUpdate(action: () => Promise<void>) {
    errorMessage = "";
    updateQueue = updateQueue.then(action).catch((error) => {
      errorMessage = "设置未能生效：" + String(error);
    });
  }

  // --- theme & box style (unchanged) ---

  function setThemeColor(color: string) {
    themeColor = color;
    applyThemeColorAcrossWindows(color);
    runUpdate(() => updateThemeColor(color));
  }

  function setBoxField<K extends keyof BoxStyleConfig>(key: K, value: BoxStyleConfig[K]) {
    boxStyle = { ...boxStyle, [key]: value };
    if (key === "border_radius") {
      // sync transparent corner_radius so both stay aligned
      const newTransparent = { ...boxStyle.liquid_glass.transparent, corner_radius: value as number };
      const newLg = { ...boxStyle.liquid_glass, transparent: newTransparent };
      boxStyle = { ...boxStyle, liquid_glass: newLg };
      runUpdate(async () => {
        await updateBoxStyle($state.snapshot(boxStyle));
        await updateLiquidGlass($state.snapshot(newLg));
      });
      return;
    }
    runUpdate(() => updateBoxStyle($state.snapshot(boxStyle)));
  }

  // --- liquid glass helpers ---

  function updateGlassFull(lg: LiquidGlassConfig) {
    boxStyle = { ...boxStyle, liquid_glass: lg };
    runUpdate(() => updateLiquidGlass($state.snapshot(lg)));
  }

  function setLiquid<K extends keyof GlassLiquidParams>(key: K, value: GlassLiquidParams[K]) {
    updateGlassFull({
      ...boxStyle.liquid_glass,
      liquid: { ...boxStyle.liquid_glass.liquid, [key]: value },
    });
  }

  function setTransparent<K extends keyof GlassTransparentParams>(key: K, value: GlassTransparentParams[K]) {
    updateGlassFull({
      ...boxStyle.liquid_glass,
      transparent: { ...boxStyle.liquid_glass.transparent, [key]: value },
    });
  }

  function setResize<K extends keyof GlassResizeConfig>(key: K, value: GlassResizeConfig[K]) {
    updateGlassFull({
      ...boxStyle.liquid_glass,
      resize: { ...boxStyle.liquid_glass.resize, [key]: value },
    });
  }

  function setFeedback<K extends keyof GlassResizeFeedback>(key: K, value: GlassResizeFeedback[K]) {
    updateGlassFull({
      ...boxStyle.liquid_glass,
      resize: {
        ...boxStyle.liquid_glass.resize,
        feedback: { ...boxStyle.liquid_glass.resize.feedback, [key]: value },
      },
    });
  }

  function setEngine<K extends keyof GlassEngineConfig>(key: K, value: GlassEngineConfig[K]) {
    updateGlassFull({
      ...boxStyle.liquid_glass,
      engine: { ...boxStyle.liquid_glass.engine, [key]: value },
    });
  }

  function setTintColorHex(hex: string) {
    const [, , , a] = boxStyle.liquid_glass.liquid.tint_color;
    const rgb = hexToRgbFloat(hex);
    setLiquid("tint_color", [rgb[0], rgb[1], rgb[2], a]);
  }

  function setTintAlpha(alpha: number) {
    const [r, g, b] = boxStyle.liquid_glass.liquid.tint_color;
    setLiquid("tint_color", [r, g, b, alpha]);
  }

  function setTransparentColorHex(hex: string) {
    const [, , , a] = boxStyle.liquid_glass.transparent.color;
    const rgb = hexToRgbFloat(hex);
    setTransparent("color", [rgb[0], rgb[1], rgb[2], a]);
  }

  function setTransparentAlpha(alpha: number) {
    const [r, g, b] = boxStyle.liquid_glass.transparent.color;
    setTransparent("color", [r, g, b, alpha]);
  }

  function setMouseColor(hex: string) {
    const rgb = hexToRgbFloat(hex);
    setLiquid("mouse_color", rgb);
  }

  function setFeedbackColor(hex: string) {
    const rgb = hexToRgbFloat(hex);
    setFeedback("color", rgb);
  }

  function setLightDir(idx: number, value: number) {
    const dir = [...boxStyle.liquid_glass.liquid.light_dir] as [number, number, number];
    dir[idx] = value;
    setLiquid("light_dir", dir);
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
        <div class="row"><span>纯图标模式（隐藏图标文本）</span><Toggle checked={boxStyle.icon_only_mode} onchange={(value) => setBoxField("icon_only_mode", value)} /></div>
      </div>
    </section>

    <section class="group">
      <header><Droplets size={17} /><div><h2>玻璃效果（liquid-verre-rs）</h2><p>开启后会增加 GPU 功耗</p></div></header>
      <div class="body fields">
        <div class="row"><span>液态玻璃效果</span><Toggle checked={boxStyle.liquid_glass.enabled} onchange={(value) => updateGlassFull({ ...boxStyle.liquid_glass, enabled: value })} /></div>
        <p class="hint">关闭时显示透明玻璃底板，开启时使用液态折射效果</p>

        <!-- 模式与几何 -->
        <details open>
          <summary class="group-label">模式与几何</summary>
          <div class="fields">
            <Slider label="圆角半径" value={boxStyle.liquid_glass.liquid.corner_radius} min={0} max={100} step={0.5} suffix="px" onchange={(v) => setLiquid("corner_radius", v)} />
            <RadioGroup label="形状" options={shapeOptions} value={boxStyle.liquid_glass.liquid.shape} onchange={(v) => setLiquid("shape", v as "RoundedRect" | "Squircle")} />
            <Slider label="超椭圆权重 (super_n)" value={boxStyle.liquid_glass.liquid.super_n} min={2} max={10} step={0.1} digits={1} onchange={(v) => setLiquid("super_n", v)} />
            <Slider label="斜面宽度 (bevel)" value={boxStyle.liquid_glass.liquid.bevel_width} min={1} max={100} suffix="px" onchange={(v) => setLiquid("bevel_width", v)} />
            <Slider label="透镜覆盖率 (lens)" value={boxStyle.liquid_glass.liquid.lens_coverage} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("lens_coverage", v)} />
            <RadioGroup label="轮廓曲线" options={profileOptions} value={boxStyle.liquid_glass.liquid.profile} onchange={(v) => setLiquid("profile", v as "Circular" | "Parabolic" | "Sinusoidal")} />
          </div>
        </details>

        <!-- 折射 -->
        <details open>
          <summary class="group-label">折射</summary>
          <div class="fields">
            <Slider label="折射率 (IOR)" value={boxStyle.liquid_glass.liquid.ior} min={1} max={2} step={0.01} digits={2} onchange={(v) => setLiquid("ior", v)} />
            <Slider label="厚度 (thickness)" value={boxStyle.liquid_glass.liquid.thickness} min={0} max={100} suffix="px" onchange={(v) => setLiquid("thickness", v)} />
            <Slider label="折射强度" value={boxStyle.liquid_glass.liquid.refr_strength} min={0} max={2} step={0.01} digits={2} onchange={(v) => setLiquid("refr_strength", v)} />
            <Slider label="色散 (dispersion)" value={boxStyle.liquid_glass.liquid.dispersion} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("dispersion", v)} />
            <Slider label="吸收 (absorption)" value={boxStyle.liquid_glass.liquid.absorption} min={0} max={2} step={0.01} digits={2} onchange={(v) => setLiquid("absorption", v)} />
          </div>
        </details>

        <!-- 模糊 -->
        <details>
          <summary class="group-label">模糊</summary>
          <div class="fields">
            <Slider label="清晰度 (clarity)" value={boxStyle.liquid_glass.liquid.clarity} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("clarity", v)} />
            <Slider label="模糊上限" value={boxStyle.liquid_glass.liquid.blur_max} min={0} max={96} onchange={(v) => setLiquid("blur_max", v)} />
            <Slider label="模糊下限" value={boxStyle.liquid_glass.liquid.blur_min} min={0} max={48} onchange={(v) => setLiquid("blur_min", v)} />
            <Slider label="渐进曲线" value={boxStyle.liquid_glass.liquid.progressive_curve} min={0} max={8} step={0.1} digits={1} onchange={(v) => setLiquid("progressive_curve", v)} />
            <Slider label="边缘永久模糊" value={boxStyle.liquid_glass.liquid.edge_always} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("edge_always", v)} />
          </div>
        </details>

        <!-- 染色 -->
        <details>
          <summary class="group-label">染色</summary>
          <div class="fields">
            <Slider label="染色不透明度" value={boxStyle.liquid_glass.liquid.tint_opacity} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("tint_opacity", v)} />
            <Slider label="自适应染色" value={boxStyle.liquid_glass.liquid.adaptive_tint} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("adaptive_tint", v)} />
            <Slider label="基础 alpha" value={boxStyle.liquid_glass.liquid.base_alpha} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("base_alpha", v)} />
            <Slider label="环境染色混合" value={boxStyle.liquid_glass.liquid.env_tint_mix} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("env_tint_mix", v)} />
            <div class="row"><span>染色颜色</span><div class="color-value"><code>{rgbaFloatToHex(boxStyle.liquid_glass.liquid.tint_color)}</code><ColorPicker value={rgbaFloatToHex(boxStyle.liquid_glass.liquid.tint_color)} onchange={setTintColorHex} /></div></div>
            <Slider label="染色 alpha" value={boxStyle.liquid_glass.liquid.tint_color[3]} min={0} max={1} step={0.01} digits={2} onchange={setTintAlpha} />
          </div>
        </details>

        <!-- 光照 -->
        <details>
          <summary class="group-label">光照</summary>
          <div class="fields">
            <Slider label="光源方向 X" value={boxStyle.liquid_glass.liquid.light_dir[0]} min={-1} max={1} step={0.01} digits={2} onchange={(v) => setLightDir(0, v)} />
            <Slider label="光源方向 Y" value={boxStyle.liquid_glass.liquid.light_dir[1]} min={-1} max={1} step={0.01} digits={2} onchange={(v) => setLightDir(1, v)} />
            <Slider label="光源方向 Z" value={boxStyle.liquid_glass.liquid.light_dir[2]} min={-1} max={1} step={0.01} digits={2} onchange={(v) => setLightDir(2, v)} />
            <Slider label="光照强度" value={boxStyle.liquid_glass.liquid.light_intensity} min={0} max={4} step={0.01} digits={2} onchange={(v) => setLiquid("light_intensity", v)} />
            <Slider label="反射强度" value={boxStyle.liquid_glass.liquid.spec_intensity} min={0} max={4} step={0.01} digits={2} onchange={(v) => setLiquid("spec_intensity", v)} />
            <Slider label="反射集中度 (spec_power)" value={boxStyle.liquid_glass.liquid.spec_power} min={1} max={512} onchange={(v) => setLiquid("spec_power", v)} />
            <Slider label="反射扩散" value={boxStyle.liquid_glass.liquid.spec_spread} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("spec_spread", v)} />
            <Slider label="Fresnel F0" value={boxStyle.liquid_glass.liquid.fresnel_f0} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("fresnel_f0", v)} />
            <Slider label="边缘光宽度 (rim)" value={boxStyle.liquid_glass.liquid.rim_width} min={0} max={16} step={0.1} digits={1} onchange={(v) => setLiquid("rim_width", v)} />
            <Slider label="焦散增益 (caustic)" value={boxStyle.liquid_glass.liquid.caustic_gain} min={0} max={4} step={0.01} digits={2} onchange={(v) => setLiquid("caustic_gain", v)} />
          </div>
        </details>

        <!-- 鼠标光效 -->
        <details>
          <summary class="group-label">鼠标光效</summary>
          <div class="fields">
            <Slider label="鼠标半径" value={boxStyle.liquid_glass.liquid.mouse_radius} min={20} max={1200} suffix="px" onchange={(v) => setLiquid("mouse_radius", v)} />
            <Slider label="鼠标强度" value={boxStyle.liquid_glass.liquid.mouse_intensity} min={0} max={4} step={0.01} digits={2} onchange={(v) => setLiquid("mouse_intensity", v)} />
            <Slider label="鼠标扩散" value={boxStyle.liquid_glass.liquid.mouse_spread} min={4} max={400} suffix="px" onchange={(v) => setLiquid("mouse_spread", v)} />
            <div class="row"><span>鼠标光颜色</span><div class="color-value"><code>{rgbFloatToHex(boxStyle.liquid_glass.liquid.mouse_color)}</code><ColorPicker value={rgbFloatToHex(boxStyle.liquid_glass.liquid.mouse_color)} onchange={setMouseColor} /></div></div>
            <Slider label="接近发光" value={boxStyle.liquid_glass.liquid.proximity_glow} min={0} max={2} step={0.01} digits={2} onchange={(v) => setLiquid("proximity_glow", v)} />
          </div>
        </details>

        <!-- 阴影与滞后 -->
        <details>
          <summary class="group-label">阴影与滞后</summary>
          <div class="fields">
            <Slider label="阴影不透明度" value={boxStyle.liquid_glass.liquid.shadow_opacity} min={0} max={1} step={0.01} digits={2} onchange={(v) => setLiquid("shadow_opacity", v)} />
            <Slider label="阴影边距" value={boxStyle.liquid_glass.liquid.shadow_margin} min={0} max={96} suffix="px" onchange={(v) => setLiquid("shadow_margin", v)} />
            <Slider label="滞后秒数 (lag)" value={boxStyle.liquid_glass.liquid.lag_seconds} min={0} max={0.1} step={0.001} digits={3} suffix="s" onchange={(v) => setLiquid("lag_seconds", v)} />
          </div>
        </details>

        <!-- 缩放交互 -->
        <details open>
          <summary class="group-label">缩放交互</summary>
          <div class="fields">
            <div class="row"><span>启用缩放</span><Toggle checked={boxStyle.liquid_glass.resize.enabled} onchange={(v) => setResize("enabled", v)} /></div>
            <div class="row"><span>边缘缩放</span><Toggle checked={boxStyle.liquid_glass.resize.edges} onchange={(v) => setResize("edges", v)} /></div>
            <div class="row"><span>角落缩放</span><Toggle checked={boxStyle.liquid_glass.resize.corners} onchange={(v) => setResize("corners", v)} /></div>
            <Slider label="抓取区域 (grip)" value={boxStyle.liquid_glass.resize.grip_px} min={2} max={128} suffix="px" onchange={(v) => setResize("grip_px", v)} />
            <Slider label="最小宽度" value={boxStyle.liquid_glass.resize.min_width} min={8} max={4096} suffix="px" onchange={(v) => setResize("min_width", v)} />
            <Slider label="最小高度" value={boxStyle.liquid_glass.resize.min_height} min={8} max={4096} suffix="px" onchange={(v) => setResize("min_height", v)} />
            <p class="sub-label">缩放反馈</p>
            <div class="row"><span>反馈启用</span><Toggle checked={boxStyle.liquid_glass.resize.feedback.enabled} onchange={(v) => setFeedback("enabled", v)} /></div>
            <Slider label="反馈边框宽度" value={boxStyle.liquid_glass.resize.feedback.border_width} min={0.5} max={32} step={0.5} digits={1} suffix="px" onchange={(v) => setFeedback("border_width", v)} />
            <Slider label="反馈 alpha" value={boxStyle.liquid_glass.resize.feedback.alpha} min={0} max={1} step={0.01} digits={2} onchange={(v) => setFeedback("alpha", v)} />
            <div class="row"><span>反馈颜色</span><div class="color-value"><code>{rgbFloatToHex(boxStyle.liquid_glass.resize.feedback.color)}</code><ColorPicker value={rgbFloatToHex(boxStyle.liquid_glass.resize.feedback.color)} onchange={setFeedbackColor} /></div></div>
          </div>
        </details>

        <!-- 透明玻璃 -->
        <details>
          <summary class="group-label">透明玻璃（关闭液态时生效）</summary>
          <div class="fields">
            <Slider label="圆角半径" value={boxStyle.liquid_glass.transparent.corner_radius} min={0} max={100} step={0.5} suffix="px" onchange={(v) => setTransparent("corner_radius", v)} />
            <Slider label="不透明度" value={boxStyle.liquid_glass.transparent.opacity} min={0} max={1} step={0.01} digits={2} onchange={(v) => setTransparent("opacity", v)} />
            <div class="row"><span>颜色</span><div class="color-value"><code>{rgbaFloatToHex(boxStyle.liquid_glass.transparent.color)}</code><ColorPicker value={rgbaFloatToHex(boxStyle.liquid_glass.transparent.color)} onchange={setTransparentColorHex} /></div></div>
            <Slider label="Alpha" value={boxStyle.liquid_glass.transparent.color[3]} min={0} max={1} step={0.01} digits={2} onchange={setTransparentAlpha} />
          </div>
        </details>

        <!-- 引擎 -->
        <details>
          <summary class="group-label">引擎</summary>
          <p class="hint">修改引擎配置将触发玻璃引擎自动重建</p>
          <div class="fields">
            <RadioGroup label="截帧后端" options={backendOptions} value={boxStyle.liquid_glass.engine.capture_backend} onchange={(v) => setEngine("capture_backend", v as "Auto" | "ForceDda" | "ForceWgc")} />
            <div class="row"><span>自适应质量</span><Toggle checked={boxStyle.liquid_glass.engine.adaptive_quality} onchange={(v) => setEngine("adaptive_quality", v)} /></div>
            <div class="row"><span>遮挡剔除</span><Toggle checked={boxStyle.liquid_glass.engine.occlusion_culling} onchange={(v) => setEngine("occlusion_culling", v)} /></div>
            <div class="row"><span>全屏时隐藏</span><Toggle checked={boxStyle.liquid_glass.engine.fullscreen_hide} onchange={(v) => setEngine("fullscreen_hide", v)} /></div>
            <Slider label="FPS 上限 (0=跟随刷新率)" value={boxStyle.liquid_glass.engine.fps_cap} min={0} max={240} onchange={(v) => setEngine("fps_cap", v)} />
            <Slider label="显示器限制 (0=全部)" value={boxStyle.liquid_glass.engine.monitor_limit} min={0} max={8} onchange={(v) => setEngine("monitor_limit", v)} />
          </div>
        </details>
      </div>
    </section>
  </div>
{/if}

<style>
  .settings { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 20px; min-height: 100%; background: #ffffff; }
  .group { overflow: visible; }
  header { min-height: 45px; display: flex; align-items: center; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--border-group); background: var(--bg-group-header); }
  header :global(svg) { color: var(--primary-color); flex: 0 0 auto; }
  h2 { font-size: 13px; line-height: 1.3; font-weight: 650; }
  p { margin-top: 2px; color: var(--text-muted); font-size: 11px; line-height: 1.35; }
  .body { padding: 13px 15px; }
  .fields { display: flex; flex-direction: column; gap: 14px; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 16px; font-size: 13px; font-weight: 500; }
  .color-value { display: flex; align-items: center; gap: 9px; }
  code { color: var(--text-secondary); font: 12px ui-monospace, Consolas, monospace; }
  .error { padding: 9px 11px; border: 1px solid rgba(239,68,68,.25); border-radius: 7px; background: rgba(239,68,68,.08); color: #b91c1c; font-size: 12px; }

  /* collapsible groups */
  details { border: 1px solid var(--border-group); border-radius: 8px; overflow: hidden; }
  details + details { margin-top: 10px; }
  summary.group-label {
    padding: 10px 14px;
    font-size: 13px;
    font-weight: 650;
    cursor: pointer;
    background: var(--bg-group-header);
    user-select: none;
    list-style: none;
  }
  summary.group-label::-webkit-details-marker { display: none; }
  summary.group-label::before {
    content: "▶";
    display: inline-block;
    margin-right: 8px;
    font-size: 10px;
    transition: transform 150ms ease;
  }
  details[open] > summary.group-label::before {
    transform: rotate(90deg);
  }
  details > .fields {
    padding: 12px 14px 14px;
  }

  .hint {
    color: var(--text-muted);
    font-size: 11px;
    margin: -8px 0 2px;
  }
  .sub-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0;
  }
</style>
