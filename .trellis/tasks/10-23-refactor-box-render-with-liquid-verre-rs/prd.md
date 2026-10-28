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

> **进度（2026-10-26，库 0.5.0 已发布）**：PR-A 已落地——输入路由 sink（Down/Up/Move/双击/右键/滚轮
> 合成标准消息转投代理窗口，标题拖拽走 WM_NCLBUTTONDOWN 模态循环复用既有 WM_MOVING 同步）、
> 命中几何快照（BlockGeom + 锁/更多按钮纯函数判定 + 单测）、内容推流（第一趟 content DIB →
> update_content，静态零成本）、代理窗口全透明 ULW 帧（可聚焦不可命中）、FocusHint→同线程 SetFocus、
> 画布级 IDropTarget 转发器（RegisterDragDrop 实测注册成功，回调行为待实机验证）、
> 引擎模式禁用 DComp 预建。514 测试全绿、warning 51<基线 54。
> **待实机验收**：图标点选/框选/双击/右键/hover/滚轮/标题拖拽+吸附参考线/grip 缩放+border 反馈/
> 锁定/拖入/拖出/键盘 ESC/tooltip/多框叠放。
> **遗留（PR-B）**：hover_edge 与捕获层死代码清理、content_bitmap 引擎模式免建、clear_content 时机、
> 拖入回调线程行为验证（若 RegisterDragDrop 跨线程注册有隐性问题，需库补 canvas hook）。

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

## 缺陷修复记录（2026-10-26 用户实机验收：双层残留在 sink 首版实现引发 5 症状）

**根因（全部在宿主侧，liquid-verre-rs 无缺陷）**：
1. 逐条转发库 120Hz 的 `WM_MOUSEMOVE` 到代理窗口 → posted 移动不像真实输入被系统
   合并 → 消息洪水压垮宿主队列 → 症状②按钮点不动、④hover 闪烁（状态抖动+渲染风暴）。
2. 合成 `WM_NCLBUTTONDOWN(HTCAPTION)` 借系统模态移动循环 → posted down/up 与模态
   启动存在竞态（up 先于模态建立即丢失）→ 症状③主线程永久挂死在模态循环、⑤无法拖拽。
3. 症状①双击无反应为 ②③ 队列堵塞的连带表现。

**修复（按库结论「接管式拖拽」配方）**：
- sink 只转发低频事件（Down/Up/双击/右键/滚轮），`Move` 仅记录 last_local（供
  cursor_override），不再进宿主队列；
- 标题区 Down（未锁定）→ `begin_box_drag`：代理窗口 SetCapture + 16ms 定时器轮询
  GetCursorPos → SetWindowPos → `WM_WINDOWPOSCHANGED` 反向 push 块矩形；Up 收尾
  `end_box_drag`：回读库权威块矩形对齐（库「中心不出屏」clamp 边界防撕裂）→
  参考线退出 + save_config；
- hover/tooltip 改为 100ms 低频轮询（`poll_glass_hover`：块内 SendMessage 合成
  WM_MOUSEMOVE 复用既有 handler，块外补发 WM_MOUSELEAVE）；
- 定时器三档：hover 100ms ↔ 移动/缩放会话 16ms（`switch_glass_timer`）。

**自动化验证（SendInput + 定向 PostMessage；桌面被用户窗口覆盖，截图受 WDA 屏蔽，
以日志/配置为证据）**：锁定按钮切换 ✓；标题双击 → edit-box 窗口创建 ✓；拖拽移动 +
出屏 clamp 对齐 + 配置保存 ✓；缩放会话状态机 begin→follow→end→save ✓；滚轮/hover
消息无崩溃 ✓；全程主线程存活无卡死 ✓；514 单测全绿、warning 51<基线 54 ✓。

**待用户实机视觉确认**：hover 高亮稳定性（无闪烁）、边缘缩放 border 反馈、拖拽参考线、tooltip。

## 缺陷修复记录（2026-10-27 用户实机验收：升 0.5.1 后「整个分类框完全不可用」5 症状）

**根因 1：锁定（固定）= 输入全灭**（症状 1/2/4/5 的直接原因，宿主侧接线错误）
- `attach_box` / `set_box_locked` 把锁定实现成 `GlassWindow::set_interactive(!locked)`。
  库对 `interactive=false` 的语义是「块不参与点击穿透判定」（库 `runtime/mod.rs` 的
  `inside` 计算、`core/draw.rs` 注释），于是画布恒带 `WS_EX_TRANSPARENT`。
- 引擎架构下分类框窗口是**全透明代理窗**（`d2d_render` 只提交全透明帧，画面由库合成），
  命中链「代理窗透明像素 → 库画布 → sink → 代理窗」在第二环就断：整块区域收不到任何
  鼠标消息 → 按钮点不动、标题双击无反应、hover 不动、拖不动，**连解锁都做不到**。
  用户两个框当时都是 `locked=true`（`~/.ikun_organizer.json`），故表现为整体不可用。
- 修复：新增单一口径 `block_config_for(style_cfg, logical, locked)` —— 锁定只关
  `resize.enabled`（库的缩放把手与会话），`interactive` **恒为 true**；标题拖拽是否
  放行仍由 sink 的 `BlockGeom::locked` + 内容层 `WM_LBUTTONDOWN` 判定（两处既有逻辑）。
  回归钉：单测 `locked_block_keeps_input_routing_and_disables_resize_handles`。

**根因 2：debug 构建下库渲染线程约 4.4 分钟必死**（症状 3「一拖就卡死」的时间线来源）
- 库 `core/quality.rs:225` 的 `good_streak: u8` 在质量档 0（正常机器永不升档，故永不复位）
  下每个统计窗口自增，256 拍后 `+= 1` 触发 dev 的 overflow-checks panic。
- panic 会跳过库的 `set_running(false)`，`Engine::is_running()` 永远停在 `true`
  → 宿主 `revive_if_dead` 永不触发 → 日志持续 `玻璃引擎[运行中] fps=0`，画布不再摘除
  穿透位、sink 停摆，分类框玻璃与全部块输入死亡。
- 日志实锤（本日 4 次，均在引擎启动后约 266 s）：09:35:55(0.5.0)、16:16:32、16:42:53、
  16:58:47(0.5.1)。release 不开溢出检查（静默回绕），所以打包版没暴露。
- 宿主侧止血：`Cargo.toml` 加 `[profile.dev.package.liquid-verre-rs] overflow-checks=false`
  让依赖在 dev 下与 release 语义一致（库发 0.5.2 后可删）。
- 补 panic 感知复活：`main.rs` panic hook 命中 `liquid-verre` 源码位置时
  `glass_engine::mark_engine_suspect()`（hook 内绝不调用引擎 API，防自死锁）；
  `revive_if_dead` 增加「嫌疑标志 + 快照 epoch 连续 2 拍停滞」双条件通道，
  10 s 限流，不再单靠会说谎的 `is_running()`。
- **库侧根治（一行，待 0.5.2）**：`good_streak`/`bad_streak` 改 `saturating_add(1)`。

**顺带修掉的工程问题**
1. 工作树编译不过：`poll_glass_hover` 引用 `GridWindowData::hover_out_ticks` 但结构体无该
   字段（上一轮改动只落一半）→ 补字段与三处初始化；hover「离开块」需连续 2 拍才补发
   `WM_MOUSELEAVE`（单拍越界抖动正是 hover 闪烁源）。
2. 接管式拖拽看门狗：`on_grid_refresh_tick` 中若左键已物理抬起却未收到 UP，强制
   `end_box_drag` 收尾（否则窗口以 16ms 追着手跑 = 用户看到的「卡死」）；
   `begin_box_drag` 在库缩放会话期间直接让位，避免两套状态机抢同一窗口。
3. sink 的 `Move` 日志降为 debug（INFO 级下 120Hz 刷日志）。
4. `wake_canvas_idle` / `revive_if_dead` 的注释口径更新到 0.5.1（注销窗口类的绕法已随
   库 #2 修复移除；wake 按库侧建议降频保留为双保险）。

**验证结论**
- `cargo test` 515 全绿（含新增 2 条回归钉）、`cargo fmt --check` 干净、`cargo build`
  54 warnings（与基线持平）。
- 宿主消息链路（PostMessage 复现 sink 下发序列）：锁定按钮切换 ✓、标题双击 → 创建
  edit-box 窗口 ✓、三点菜单 → 菜单窗口创建 ✓、图标点击 `[CLICK] hit=0` ✓。
- **端到端真实鼠标未能在 Codex 执行环境验证**：该环境被 Job Object 限制
  （`SetCursorPos`/`SendInput` 返回 err=5、`OpenDesktop` 拒绝访问、DDA
  `DuplicateOutput` 0x80070005 → 库回退 WGC 且渲染循环 fps=0、块 `is_alive()=false`），
  「画布摘除 `WS_EX_TRANSPARENT` → sink 收到事件」这一环需用户在真实桌面验收。
  另：分类框窗口带 `WDA_EXCLUDEFROMCAPTURE`，截图看不到框体，核对画面请设
  `IKUN_ALLOW_CAPTURE=1` 后重启。

**用户验收清单（0.5.1 + 本轮修复）**
1. 未锁定态：标题空白处按下即可拖走整框，拖拽中出现对齐参考线，松手后布局落盘。
2. 锁定态（点锁定按钮变锁）：框不可拖不可缩放，但图标点击/选中/右键菜单/锁定按钮本身
   仍然可点，且能再点一次解锁（旧行为：锁上即全死）。
3. 标题双击 → 弹出「编辑分类框」窗口；三点菜单按钮 → 弹出菜单；图标 hover 高亮稳定不闪。
4. 挂机 10 分钟（跨过旧 panic 时间点 266 s）后日志无 `[PANIC] ... quality.rs`，
   `mem_probe` 的 `玻璃引擎[...] fps` 不归零，分类框仍可交互。
5. 若仍出现「运行几分钟后全灭」：抓 `~/.ikun_organizer_log` 里 `PANIC` 与
   `玻璃引擎[运行中] fps=0` 两行，即为库侧渲染线程再次死亡，需推 0.5.2 的 saturating 修复。

## 缺陷修复记录（2026-10-28 用户复验：锁定已正常，其余 4 症状的另两条根因）

用户复验反馈「只有固定（锁定）好了，其他照旧」。真实鼠标复现 + 日志逐条定位到两条新根因，
都与上一轮的两条无关：

**根因 3：库画布窗口类没有 `CS_DBLCLKS` → 双击标题永远不触发**（症状 1）
- 库 `platform/window.rs` 注册画布类只给了 `style: CS_CLASSDC`。没有 `CS_DBLCLKS`，
  Windows 不会产生 `WM_LBUTTONDBLCLK`，库的 `PointerPhase::DoubleClick` 分支
  （`window.rs:206`）永远不执行。
- 日志实锤（09:50:20 / 09:50:22）：用户在标题上双击，sink 只收到**两组独立 Down/Up**，
  每组还各自拉起一次接管式拖拽（begin→end + `收集布局数据` + save_config），编辑框从不弹。
- 修复（宿主侧合成，不依赖库发版）：sink 自己按系统节律判定双击 ——
  `RouteState` 记录上次 Down 的单调 ms 与块内坐标，第二次 Down 落在
  `GetDoubleClickTime()` + `SM_C?DOUBLECLK/2`（按 DPI 折算到物理像素）容差内即判双击，
  走与真相位同一个投递口 `post_double_click`（标题→`WM_NCLBUTTONDBLCLK(HTCAPTION)`，
  主体→`WM_LBUTTONDBLCLK`）。纯函数 `is_double_click` + 单测钉死。
- 两个必要约束：**按钮热区不参与合成双击**（锁定/更多上连点两次是「切换两次」，
  不能被解释成「双击标题=开编辑框」）；一次双击只认一次（三连击不会变两次双击）。
- 待库侧根治（并入 0.5.2 清单）：画布类补 `CS_DBLCLKS`。补上后本合成逻辑自然退化为
  「与真相位同口径」，不会双触发（真 DoubleClick 走同一 helper）。

**根因 4：库 FocusHint 抢回焦点 → 刚弹出的模态菜单立刻自杀**（症状 2 的三点按钮）
- 三点按钮在 `WM_LBUTTONDOWN` 里同步调用 `show_menu_window`，菜单靠
  `WM_ACTIVATE(WA_INACTIVE)` 关闭；而块被按压消费后库会发 FocusHint，
  宿主在 `WM_USER_GLASS_FOCUS` 里对代理窗 `SetFocus` —— 菜单刚建好就被抢走焦点，
  下一拍 `WA_INACTIVE` 直接关掉，用户侧就是「点了没反应」。
- 修复：`WM_USER_GLASS_FOCUS` 在「本框菜单打开中 / 图标就地重命名编辑中」时忽略抢焦点。
- 验证（不碰光标的 PostMessage 复刻 sink 序列，两个框都过）：
  `Down → DesktopOrgMenu 创建(0x320ef4) → 再投 WM_USER_GLASS_FOCUS → 菜单仍存活 → ESC 正常关闭`。

**顺带（同一轮日志暴露的浪费）**
- 接管式拖拽的参考线 overlay 改为**懒创建**：`begin_box_drag` 不再立刻拉起
  5040x2080 全屏 overlay，首次产生真实位移时才 `on_enter_size_move`；
  没有位移就既不建 overlay 也不 `save_config`（旧实现标题每点一次写一次配置）。
- 新增 `debug_log_blocks()`（debug 级、1 s 一条）打印每块库侧真值
  `rect/interactive/visible/occluded/hovered/moving/resizing/resize_enabled/movable`，
  以后「某个框点不动、另一个正常」一眼可判。
- 实测：光标进块 → 画布摘除 `WS_EX_TRANSPARENT` 延迟 **23–25 ms**（锁定框同样在线）。

**2026-10-28 真实输入复验结果**
- 锁定按钮 ✓（用户手点：`锁定状态切换: 已解锁 → 开发工具`）
- 图标点击/选中/框间拖拽 ✓（`[CLICK] hit=0/2`、`handle_box_to_box_move`）
- 标题双击 → 编辑框 ✓（`WM_NCLBUTTONDBLCLK 触发编辑分类框对话框` + `按需创建持久 WebView 窗口: edit-box`）
- 三点菜单 ✓（FocusHint 后存活，ESC 正常关闭）
- 待用户目视确认：拖拽跟手 + 对齐参考线出现/消失时机、图标与按钮 hover 是否还闪
  （屏幕录制不可用：分类框窗口带 `WDA_EXCLUDEFROMCAPTURE`）。

## 实施顺序

1. `app_config.rs` 新配置结构 + normalize + 兼容加载。
2. `glass_engine.rs`：引擎生命周期、注册表、坐标换算（含单测）、事件→日志。
3. `grid_window.rs` / `wnd_proc.rs`：attach/detach、hit-test 改造、会话跟随、锁联动、DPI 变化。
4. `d2d_render.rs` 内容层去背景 + `liquid_glass.rs` 删除 + mem_probe/context/config_cmd 调用点迁移。
5. `platform_runtime.rs` 引擎启动/停止接线。
6. 前端 `tauri-api.ts` + `StyleSettings.svelte` 全参数面板。
7. fmt/clippy/test + frontend check/build + 规格文档同步。
