# 使用 liquid-verre-rs 重构分类框渲染

## 目标

把分类框的渲染基座从自研 `liquid_glass.rs`（DDA 采集 + D2D Effect Graph + 自研 HLSL）迁移到 crates.io 的 `liquid-verre-rs = 0.4.2`（作者同为本项目作者，即本项目液态玻璃的库化版本），由库承担：液态玻璃效果、透明玻璃（非液态）底板、玻璃块拖拽移动、边缘/四角缩放及悬停加粗 border 视觉反馈；分类框 Win32 窗口退化为"内容层"，继续在玻璃之上绘制图标、栅格布局、title、锁定/关闭/更多按钮、多选橡皮筋、右键菜单、重命名编辑框等。

## 需求

### R1 风格设置面板接入库接口参数
- `box_style.liquid_glass`（旧 9 字段）替换为对齐库 API 的新配置结构（serde camelCase，JSON 持久化）：
  - `enabled`：true → `GlassStyle::Liquid`；false → `GlassStyle::Transparent`（替代原 D2D 回退底绘制，非液态分类框同样走库）。
  - `liquid`：`LiquidParams` 全量镜像（37 项：corner_radius/lens_coverage/bevel_width/shape/super_n/profile/ior/thickness/refr_strength/dispersion/absorption/clarity/blur_max/blur_min/progressive_curve/edge_always/tint_opacity/adaptive_tint/base_alpha/tint_color/env_tint_mix/light_dir/light_intensity/spec_intensity/spec_power/spec_spread/fresnel_f0/rim_width/caustic_gain/mouse_radius/mouse_intensity/mouse_spread/mouse_color/proximity_glow/shadow_opacity/shadow_margin/lag_seconds）。
  - `transparent`：`TransparentParams`（corner_radius/opacity/color）。
  - `resize`：`ResizeOptions`（enabled/edges/corners/grip_px/min_width/min_height）+ `ResizeFeedbackOptions`（enabled/border_width/alpha/color）。
  - `engine`：`WindowLevel` 固定 AlwaysOnBottom（不暴露）、capture backend（Auto/DDA/WGC）、adaptive_quality、occlusion_culling、fullscreen_hide、fps_cap（0=跟随刷新率）。
- 前端「风格设置」按组渲染滑块/开关/颜色/单选：几何、折射、毛玻璃、染色、光照、鼠标点光、阴影与滞后、缩放交互与反馈、透明模式、引擎（引擎级参数改动会触发引擎重建，UI 需提示）。
- `border_radius` 继续双向同步到 liquid.corner_radius 与 transparent.corner_radius（保持旧联动）。

### R2 玻璃底板由库渲染，内容层保持原有绘制
- 应用启动（platform_runtime::initialize）时 `Engine::spawn`（单渲染线程），`WindowLevel::AlwaysOnBottom` 保持分类框"低于其他应用窗口、高于桌面"的既有关键语义。
- `create_grid_window` 成功后为每个分类框 `engine.create_window(WindowConfig{rect(逻辑像素), style, resize, movable})`，`destroy_grid_window` 时 `close()`；box uuid ↔ glass id 映射存于 glass_engine 注册表。
- 分类框 ULW/DComp 内容层只画前景（图标/标签/title 文字/按钮/选框），背景改为全透明；库在其下方画玻璃块（含圆角、阴影、折射）。
- 库初始化失败（GPU 缺失等）→ 优雅降级：全部回退旧 D2D 圆角纯色回退背景 + 保留原生 HTCAPTION/边缘缩放 hit-test（引擎激活时该路径不走）。

### R3 交互迁移：拖拽移动、边缘缩放与视觉反馈、锁定（固定）
- 库激活时分类框 `WM_NCHITTEST`：标题栏空白区与阴影留白/边缘把手带返回 `HTTRANSPARENT`（点穿到下层库画布，由库发起 Move/Resize 会话并绘制 hover 加粗 border 反馈与滞后补偿）；图标、按钮、主体格区域保持 `HTCLIENT`（点击、多选橡皮筋、OLE 拖出、右键菜单、重命名不变）。
- 分类框自有的 HTCAPTION 移动、HTLEFT/HTTOP… 缩放与 `hover_edge` 指示条在库激活路径下移除（由库接管）。
- 库 Move/Resize 会话期间：分类框按 `WindowSnapshot.moving/resizing` 标志在定时器 tick 中轮询 `rect()` 快照，`SetWindowPos` 让内容窗口跟随玻璃矩形（会话期间切高频档，复用 enter_drag_refresh 机制）；`ResizeEnded` 后 `recalculate_layout + reflow_items` 重排图标。
- 会话结束（MoveEnded/ResizeEnded 快照标志消失）→ 持久化布局（沿用现有保存链路）。
- 锁定（固定按钮）：同步 `set_movable(false)` + `set_resizable(false)`；解锁恢复；锁定时内容层保持全 HTCLIENT。
- `push_other_grids_below` / DPI 变化 / MonitorsChanged 后调用 `Engine::set_window_level(当前值)` 重压画布到 Z 序底部，保证内容窗口始终在玻璃之上。

### R4 库日志与事件并入项目日志系统
- 库走 `log` facade，项目 env_logger（滚动文件 ~/.ikun_organizer_log）已装全局 logger，自动捕获；`EngineConfig.log_level` 设 `Trace`（不收紧全局 facade 过滤，实际过滤仍以项目 env_logger 过滤器为准，避免影响其他模块日志级别）。
- `poll_engine_events()`（runtime tick 100ms）消费 `EngineEvent`：Ready/MonitorsChanged/CaptureBackendSelected/CaptureLost/CaptureRestored/ExclusiveFullscreen/LevelApplied/Fatal/Shutdown/EventsDropped 全部转中文 `log::info!/warn!/error!`。
- 删除自研 `liquid_glass.rs` 中的采集/渲染日志与 mem_probe 相关钩子，替换为引擎快照指标（quality_level/fps/monitor 数）。

## 不做/边界

- 不改动分类框业务逻辑（图标提取、分类、拖放目标、桌面隐藏等）。
- 不引入平台抽象（Windows-only 直接调用库 API）。
- `IKUN_DCOMP` 内容路径保留（内容层仍可用现有 DComp 合成画前景），但玻璃底板不再经过它。
- 旧配置字段（displacement_scale/blur_amount/saturation/chromatic_offset/elasticity/refraction_mode/over_light）不迁移（无对应新参数，加载时静默丢弃）。

## 验收标准

- [x] `cargo fmt` / `cargo clippy` / `cargo test`（512 通过）通过；`pnpm -C frontend run check`（0 错误）+ `build` 通过。（自动验证）
- [x] 冒烟验证：双屏 DDA 采集 + 画布显示 + 玻璃块=2；风格设置 61 控件接通 update_liquid_glass；视觉细节以用户实机验收为准。
- [x] enabled=false → GlassStyle::Transparent 映射有单测；实机表现待用户验收。
- [ ] （按决策记录 R3 修订）标题栏 = 原生移动 + WM_MOVING 同步库块；边缘缩放 = 库会话。缩放会话实测全链路通过：ResizeStarted → 跟随 tick + 参考线 → ResizeEnded → save_config。
- [x] 实测一次完整缩放会话（日志为证）；面板可调 grip/min/反馈参数。图标重排复用 WM_SIZE → reflow 原链路。
- [ ] 待用户实机验收（代码路径未改动，仅底板来源与命中带切换）。
- [x] set_box_locked → set_interactive(false) 接线 + 内容层全 HTCLIENT 保留；实机待验收。
- [x] 冒烟日志双通道验证通过（含 RUST_LOG=info 下 INFO 直出、引擎事件转中文）。
- [x] 回退分支编译/单测覆盖（is_active=false 全链路走旧 D2D 背景 + 原生 HT）；实机断 GPU 场景待验收。

## 技术笔记

- 依赖：`[target.'cfg(windows)'.dependencies] liquid-verre-rs = "0.4.2"`（已加，cargo check 通过；其 windows 0.58 与项目 windows 0.62 共存无冲突）。
- 坐标换算：库 Rect = 逻辑像素（=主屏物理像素，虚拟屏绝对坐标）。物理↔逻辑用主屏 scale = GetDpiForSystem()/96 换算；块所在屏 DPI 由库内部缩放。集中到纯函数 `glass_engine::phys_to_logical/logical_to_phys` + 单测。
- Z 序：画布每屏一个、进程内由库拥有；`SetWindowPos(canvas,...)` 被库禁止，内容窗口压底的既有逻辑（HWND_BOTTOM 强制）会把内容压到画布之下导致图标被玻璃盖住，故凡涉及"推底"路径（push_other_grids_below、WM_WINDOWPOSCHANGING 干预）后必须 `set_window_level` 重 assert 画布底部位置。
- 库为最终一致快照读（≤1 帧延迟），会话期间内容跟随用定时器轮询 `GlassWindow::rect()`，引擎事件仅用于日志与布局保存兜底。
- 与在途任务 `09-11-fix-glass-ghost-and-memory-climb` 关系：本重构整体替换其作用对象（自研玻璃渲染），该任务随重构完成可关闭。
- 风险：多屏混合 DPI 坐标语义需实测；overlay/对齐参考线窗口与画布的层叠需验证点击穿透不受影响。

## ⚠ 架构修订（2026-10-23 用户验收后推翻了「双层叠放」模型）

**错误理解（已废弃）**：把库玻璃当"底板/背景"，分类框保留自己的 Win32 窗口叠在画布之上，
靠「alpha=1 命中捕获层 + 透明把手带穿透」把边缘缩放让给库。实机验收证明这条路交互全坏
（拖拽/缩放/图标命中互相打架），本质是模型错了。

**正确目标（用户确认）**：分类框的**窗口本体**就是 liquid-verre-rs 的玻璃窗口——即库画布上的
GlassWindow 块；图标/栅格/title/固定等功能全部**移植到库窗口上**（内容层由库呈现、输入由库
统一接收后路由给宿主），不再为每个分类框创建独立 Win32 窗口、不再有"内容窗口叠玻璃"。
库不支持的能力由作者（同本项目作者）修改 liquid-verre-rs 补齐，差距清单见下节。

### 已实施部分的处置
- 保留：依赖接入、app_config 参数镜像与 normalize、glass_engine 适配层骨架（生命周期/注册表/
  坐标换算/事件转日志/Z 序）、前端风格设置 61 控件与 IPC、spec 同步。
- 待重写（阻塞于库 API）：d2d 内容层呈现改为推送库内容纹理（删「命中捕获层」hack）、
  grid_window/wnd_proc 的窗口与命中测试逻辑迁入事件 sink、ULW 提交路径退役。

### liquid-verre-rs 能力差距清单（请库作者实现/确认，按优先级）

**P0-1 宿主内容层（每块绘制）**
- `GlassWindow::update_content(buf: &[BGRA8 premul], w_phys, h_phys, stride, dirty: Option<Rect>)`
  （+ `clear_content()`）：库为每块持有 SRV，Pass B 画完该块玻璃后画 textured quad
  （dst=块矩形、UV 按 dst/纹理尺寸），随块 z_order 遮挡、随 ResizeEnded 由宿主重推尺寸。
  内容层固定在该块玻璃之上、border 反馈之下或之上由作者定（建议反馈最上）。
- 理由：分类框内容静态（状态变化才重绘），CPU 上传是最小可用路径；避免跨设备共享纹理的复杂度。

**P0-2 输入路由（命中判定 + 事件转发）**
- `GlassWindow::set_input_sink(sink)`，回调**同步**在库渲染线程（canvas wndproc 上下文）执行，
  宿主保证微秒级返回（数据用共享快照，无锁/短锁）：
  - `on_pointer_down(id, ev) -> Handled|Drag|Passthrough`：canvas 收到 LBUTTONDOWN 且 hit_test
    命中本块时**先问 sink**（ev 含块内物理坐标）；返回 Drag 才进入库的移动/会话状态机。
  - `on_pointer_move(id, pt)` / `on_pointer_up(...)` / `on_double_click` / `on_button_up(右键)` /
    `on_wheel(delta)`：转发给宿主（hover 高亮、tooltip、多选、滚动、双击打开、右键菜单）。
  - sink 返回 Handled 的按压，库不得开始拖拽；库的 resize 把手带优先级高于 sink（边缘=缩放）。
- WM_SETCURSOR：hover 判定也走 sink（宿主可在回调里决定图标手型/默认箭头）。
- 理由：没有这两项，库画布收到点击就只会自己拖窗，图标点击/框选/双击全部失效；反之宿主
  全吃掉又失去库拖拽。路由是"库窗口即分类框"的核心缺口。

**P1-3 拖拽接管信号**
- 宿主在 sink 里对图标按压起 OLE DoDragDrop（长流程）时：`GlassWindow::suspend_input(true/false)`
  或直接约定「on_pointer_down 返回 Handled 即本轮不拖」+ `release_capture()`；DoDragDrop 的
  anchor HWND 由宿主用隐藏壳窗口解决，不需库改。

**P1-4 焦点窗口**
- `GlassWindow::set_focus_hwnd(hwnd)`：块被点击（sink 放行后）库对 focus_hwnd SetFocus，
  使键盘交互（ESC/Delete/F2）可达；canvas 自身保持 WS_EX_NOACTIVATE 不抢焦点。

**P1-5 Drop 目标路由**
- 文件拖入分类框：宿主可对 `canvas_handle()` 注册 IDropTarget（无需库改，坐标宿主自己按块
  矩形换算），但要求库保证 canvas 在块区域非 click-through 且消息被 pump（现状已满足）。
  若后续 canvas 样式调整需保持该兼容。

**P2-6 拖拽位置过滤（对齐吸附）**
- `Engine::set_move_filter(id, Fn(Rect)->Rect)`：库会话内每帧应用宿主修正（消除"松手才吸回"）；
  没有也可先由宿主 ResizeEnded 后处理。

**已具备、无需改**：移动/缩放会话+视觉反馈、min 尺寸、z_order/bring_to_front、set_visible、
set_movable/set_interactive/set_resizable（固定/解锁）、WindowLevel::AlwaysOnBottom、
monitors/occlusion/fullscreen、log facade 与事件。

### 项目侧移植计划（等 P0-1/P0-2 就绪后执行）
1. grid_window：退役每框 ULW 窗口 → "GlassBlockView"（注册表 + 内容重绘推流 + 事件消费）；
   仅保留一个进程级隐藏壳窗口（键盘焦点 / OLE 锚点 / 定时器消息）。
2. wnd_proc：图标/标题/按钮/框选/菜单逻辑迁到 input sink 适配层（坐标=块内物理像素）；
   HT 边缘/hover_edge/捕获层全部删除。
3. d2d_render：绘制逻辑保留（离屏 DIB 渲染），提交端从 UpdateLayeredWindow 改为
   `update_content(...)` 推流；状态变化才推 dirty。
4. glass_engine：内容纹理尺寸管理、事件路由分发、焦点/锁定联动。
5. 对齐参考线：会话中按库 rect 轮询画线；吸附待 P2-6。

## 实施决策记录（编码期确定，⚠ 其中交互/叠放相关条目已被上方「架构修订」取代）

1. **交互分工（R3 最终语义）**：
   - 边缘/四角缩放 + 悬停加粗 border 反馈 = 库缩放会话（`ResizeOptions`/`ResizeFeedbackOptions`）。内容层用「命中捕获层」实现让位：alpha=1 近隐形填充覆盖除把手带外的整块矩形，把手带（`grip_px × 屏DPI比例`，与库 `hit_resize` 同源判据）保持 alpha=0 → ULW/DComp 对全透明像素自动穿透鼠标到下层库画布。
   - 标题栏拖拽移动 = 保留原生 HTCAPTION 模态移动（保住双击改标题、参考线、保存链路），`WM_MOVING` 把内容矩形同步进库块（命令队列去重，玻璃 ≤1 帧贴上）；块的 `movable=false`，库移动会话不启用。
   - 库缩放会话期间：`WM_USER_GLASS_RESIZE`（引擎事件回调渲染线程 PostMessage）→ 16ms 跟随定时器 → tick 读 `WindowSnapshot.rect()` 贴图 + `resizing` 回落收尾（参考线关闭、`save_config` 持久化、`reflow` 经 WM_SIZE 自动完成）。
   - 固定（锁定）：内容层全 HTCLIENT（既有逻辑）+ `GlassWindow::set_interactive(false)` 双保险；解锁恢复。
2. **Z 序不变量**：`WindowLevel::AlwaysOnBottom`（画布低于全部普通窗口、高于桌面，与旧 `HWND_BOTTOM` 语义一致）；`WM_USER_PUSH_GRIDS_BELOW` 后 `raise_box` + `reassert_canvas_bottom()`（画布压回最底，保证内容窗口恒在玻璃之上）。库遮挡剔除对 WS_EX_LAYERED 一律否决，内容层不会误触发剔除。
3. **内容层几何**：外投影留白恒 0（投影/圆角/折射全在库画布），`render_padding` 读取路径保留；旧 `liquid_glass.rs` 自研渲染器整体删除；引擎不可用 = 唯一回退路径（D2D 圆角半透明背景 + 原生边缘缩放/标题拖拽，即旧「关玻璃」形态）。
4. **非液态玻璃分类框**：`enabled=false` 时库以 `GlassStyle::Transparent`（圆角/透明度/颜色，联动 border_radius）渲染底板，同样具备库的圆角与后续把手能力。
5. **日志**：库走 `log` facade 自动进 env_logger 滚动文件；`EngineConfig.log_level=Trace`（库构造会 `log::set_max_level` 全局门闩，取 Trace 避免收紧项目其他模块日志级别，实际过滤权留在项目 env_logger）；`EngineEvent` 全量映射为中文日志（runtime tick 消费）；mem_probe `[内存分解]` 的玻璃段改报引擎指标（fps/质量档/屏数/块数），`FRAME_TIMINGS` 由 thread_local 改全局 Mutex（输出方与记账方不同线程）。
6. **坐标换算**：块 `x/y` 传虚拟屏物理像素（与 GetWindowRect 同系），`w/h` 传逻辑像素 = 物理 ÷ (该屏DPI/主屏DPI)；回读用 `WindowSnapshot.scale` 乘回。单测钉死往返。
7. **已知行为变化**：库会把块中心钳制在所在屏内（最多半出屏），分类框无法整体拖出屏外（防丢失，替代旧「可拖没」行为）。

## 实施顺序

1. `app_config.rs` 新配置结构 + normalize + 兼容加载。
2. `glass_engine.rs`：引擎生命周期、注册表、坐标换算（含单测）、事件→日志。
3. `grid_window.rs` / `wnd_proc.rs`：attach/detach、hit-test 改造、会话跟随、锁联动、DPI 变化。
4. `d2d_render.rs` 内容层去背景 + `liquid_glass.rs` 删除 + mem_probe/context/config_cmd 调用点迁移。
5. `platform_runtime.rs` 引擎启动/停止接线。
6. 前端 `tauri-api.ts` + `StyleSettings.svelte` 全参数面板。
7. fmt/clippy/test + frontend check/build + 规格文档同步。
