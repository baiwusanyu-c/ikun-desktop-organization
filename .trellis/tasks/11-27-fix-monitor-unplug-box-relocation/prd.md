# 修复：拔掉副屏后分类框丢失（等比重定位 + 重连自动搬回）

## 目标

多屏场景下拔掉副屏（或显示器电源关闭）后，位于副屏的分类框滞留在已消失的
虚拟桌面坐标区域：不可见、不可操控，且玻璃块与代理窗口错位。要求：

1. 副屏消失时，框按「屏内比例」等比映射到最近的在岗显示器对应位置（允许互相重叠）；
2. 副屏重新接回时，框**自动搬回**原屏的原相对位置；
3. 持久化引入「显示器身份 + 屏内相对坐标」，修复“拔屏后重启看不见框”的遗留项
   （11-26 任务 PRD 技术笔记预留）。

## 已定案的设计决策（与用户确认）

- 不按屏幕拆多份分类框数据；保持全局 `layout.boxes` 单列表，每框记录归属屏身份。
- 尺寸口径（实现定案，2026-11-27 自检轮对齐）：**位置按屏内比例映射，
  尺寸保持逻辑口径**——目标屏物理尺寸 = 当前物理 x（目标屏 DPI / 当前窗口 DPI），
  与手工跨屏拖拽时 PerMonitorV2 `WM_DPICHANGED` 的换算完全同构；仅当换算后
  仍装不下目标屏时按长宽比整体等比缩小。因此 `MonitorOrigin` 只记录
  **位置比例 + 屏身份**，尺寸由窗口现状 / `BoxLayoutData.size`（本就是 96
  逻辑口径）现算，不落第二份数据。保列数宽度吸附走 `relayout_for_dpi`，
  防止「图标变乱」复发。
- 目标屏：离“消失屏”最近的在岗屏（两屏场景即主屏）。
- 副屏重连：**自动搬回**（用户明确要求）。搬回时机 = 拔回事件到达且防抖后的
  对账通过；若用户在滞留期间又手动拖过框（会话收尾重新收养归属屏），则取消搬回，
  尊重用户最新意图。

## 根因（已核实）

- 全项目无 `WM_DISPLAYCHANGE` / `WM_DEVICECHANGE` 处理；玻璃引擎
  `MonitorsChanged`（glass_engine.rs L1500）只失效缓存 + 压画布层序，不搬框。
  注意 `monitor_utils` 的矩形缓存是 **thread_local**，引擎线程失效 ≠ 窗口线程失效。
- 既有自愈（建窗吸附 / WM_WINDOWPOSCHANGED 吸附 / collect_grid_layout 落盘吸附）
  全部依赖「窗口位置消息」触发，拔屏瞬间不会触发。
- `BoxLayoutData`（app_config.rs L931）只存 position/size，无显示器身份，
  跨 DPI 屏恢复基准不准。

## 需求

### R1 纯逻辑下沉 `box_core/monitor_remap.rs`（0 unsafe、可单测）

- `MonitorOrigin { device: String, rect: [i32;4], rel: [f32;4] }`
  （rect = 记录时该屏完整矩形，虚拟桌面物理像素；rel = 屏内比例位置 0..=1）。
- `to_relative(box, monitor_bounds) -> rel`：夹到 [0,1]，全程 saturating/f64。
- `from_relative(rel, monitor_bounds, min_w, min_h) -> RectI`：
  等比展开 + 最小尺寸钳制（min = 玻璃逻辑下限 200x140 按目标屏 DPI 折算）+
  整体装入目标屏（复用 `fit_into_bounds` 语义：装得下则保尺寸平移，
  装不下等比缩）。
- `find_live_match(origin, live: &[(device, bounds)]) -> Option<usize>`：
  优先 device 相等且（rect 全等 或 分辨率全等）；device 缺失/变更时退回 IoU>=0.75。
- `pick_nearest_live(origin_rect_or_box_rect, live) -> usize`：复用
  `monitor_geom::monitor_index_for_center`。
- `plan_for_box(box, origin, pending_home, live, bounds, min) -> Decision`：
  - `ReturnHome(target_idx, rect)`：pending_home 在岗（含“滞留期间用户没再拖过”前提，
    由会话收尾清 pending 保证）；
  - `Settled(new_origin)`：框完整落在某在岗屏（刷新归属记账，清 pending 由
    收尾路径负责，这里仅幂等）；
  - `Relocate(target_idx, rect)`：滞留且有 origin —— rel 直接用 origin.rel
    （避免反复往返漂移），目标 = 离 origin.rect 中心最近的在岗屏；pending_home := origin；
  - `FallbackSnap(rect)`：滞留但无 origin 可用 —— 退回 snap_into_single_monitor。

### R2 运行时触发（全部在分类框自己的窗口线程执行）

- `wnd_proc.rs`：
  - `WM_DISPLAYCHANGE` → `display_reconcile::schedule_reconcile(hwnd)`（防抖 500ms）；
  - `WM_TIMER`（新增 `TIMER_ID_MONITOR_RECONCILE`）→ 跑一轮对账；首轮后若
    枚举仍与预期不符或发生过搬动，追加 1500ms 二次确认拍（热插拔稳定期，
    `EnumDisplayMonitors` 可能仍返回旧屏集）；
  - 新增 `WM_USER_DISPLAY_RECONCILE`（glass_engine 线程 PostMessage 触发，复用
    `post_to_all_grid_windows` 模式），同走 schedule 防抖。
- 每个窗口只搬自己 → 无跨线程 SetWindowPos（历史死锁红线）。

### R3 每框归属记账（grid_window.rs）

- `GridWindowData` 新增：
  - `monitor_origin: Option<MonitorOrigin>`（归属屏身份 + 屏内比例）；
  - `pending_home: Option<MonitorOrigin>`（拔屏暂搬标记，重连自动搬回依据）；
  - `reconcile_pass: u8`（防抖拍计数）。
- 收养点（用户驱动几何稳定 → 重记 origin、清 pending_home）：
  - `create_grid_window` 最终落位后（无 hint 时收养当前屏）；
  - `end_glass_resize_session` / 玻璃移动会话收尾（最终 snap 之后）；
  - `WM_EXITSIZEMOVE`（回退模式拖拽/缩放、DPI 跨屏拖拽结束）。
- 程序化搬动（relocate/return）不得走收养路径。

### R4 显示器枚举带设备名（monitor_utils.rs + ffi_types.rs）

- `MONITORINFOEXW`（szDevice [u16;32]）+ `get_monitors_named() ->
  Vec<(String, (i32,i32,i32,i32))>`：每次真实枚举（对账是低频路径，不走 TTL 缓存）。
- 设备名语义：`\\\\.\\DISPLAY2`。Windows 同名复用不保证 100% 稳定，
  匹配同时校验分辨率/IoU，误配后果仅是“没搬回”，可接受（文档注释说明）。

### R5 持久化（app_config.rs + collect + restore）

- `BoxLayoutData` 新增 `#[serde(default)] monitor: Option<BoxMonitorOrigin>`：
  `BoxMonitorOrigin { device, rect, rel, relocated }`（旧配置自动兼容）。
- `collect_grid_layout` 从窗口 data 写出 origin + relocated(=pending_home.is_some())；
- `platform_runtime::restore_layout` 启动恢复：
  - monitor 在岗 → `from_relative` 直接落回该屏（relocated=true 时天然实现
    “重启后自动搬回”）；
  - monitor 离岗/缺失 → 离岗：`plan` 等比映射到最近在岗屏，GridCreateParams 传
    hints（origin 保持原屏、pending_home=原屏），重启后副屏再插上仍能搬回；
    缺失（历史配置）→ 维持现状（吸附兜底）。
  - 物理坐标 → 96 基准逻辑坐标换算按目标屏 DPI 折算（与 collect_grid_layout 同口径）。

### R6 搬动执行（新文件 `platform/windows/display_reconcile.rs`）

- `relocate_box(hwnd, target_content_rect, ...) -> bool`：
  - `invalidate_monitor_rects_cache()`（窗口线程！thread_local）；
  - 按目标屏 DPI：`relayout_for_dpi` 吸附宽度保列数 → `update_dcomp_scale` →
    SetWindowPos（外框）→ 玻璃激活时 `push_box_rect(_with_ratio)` +
    `update_block_geom` 对齐「块==命中层==内容」→ `content_valid=false`
    强制全量重绘 → `render_layered_window`；
  - 本轮有实际搬动 → `drag_handler::save_config()` 落盘一次。
- 日志中文；只在真实事件时打 info，防抖拍空转打 debug。

## 验收标准

- [x] 拔掉副屏 ≤1s：副屏上的分类框等比映射到最近在岗屏对应位置（允许重叠），
      可见、可拖拽、可缩放、可双击；玻璃块与内容窗口不错位。（2026-11-27 实机两轮通过）
- [x] 重新接上副屏：框自动搬回副屏原相对位置（按屏内比例）。（实机通过）
- [ ] 滞留期间用户手动把框拖到别的在岗屏（会话收尾）→ 归属屏改判、取消自动搬回。
- [ ] 拔屏状态下重启 → 框等比恢复到在岗屏；重启期间副屏接回 → 下一轮对账搬回；
      拔屏时退出、重启时副屏已接回 → 启动恢复直接落回原屏（搬回语义持久化）。
- [ ] 单屏机 / 历史配置（无 monitor 字段）行为不变。
- [ ] 新纯函数单测：往返恒等、最小尺寸、装不下等比缩、IoU 匹配、四分支决策。
- [ ] `cargo fmt --check` / clippy 严格组合 / `cargo test` 全绿；前端零改动。

## 技术笔记 / 约束

- 坐标口径：虚拟桌面物理像素；持久化 position/size 维持 96 基准逻辑像素不变，
  monitor.rel 是无量纲比例，天然免疫 DPI/分辨率小幅波动。
- `wnd_proc.rs`（1953 行）与 `grid_window.rs`（2624 行）已超 1500 行红线，
  本轮只加最薄钩子，主体逻辑进 `display_reconcile.rs` + `box_core/monitor_remap.rs`。
- 玻璃块单屏模型：搬回/重定位落点必须整框留在单屏内（plan 已保证）。
- 不做 git commit（红线，等用户明确指令）。

## 变更文件清单（预估）

| 文件 | 变更 |
|---|---|
| `src/box_core/monitor_remap.rs`（新） | 等比映射 + 决策纯函数 + 单测 |
| `src/box_core/mod.rs` | 注册模块 |
| `src/platform/windows/display_reconcile.rs`（新） | 对账编排（窗口线程执行） |
| `src/platform/windows/mod.rs` | 注册模块 |
| `src/platform/windows/wnd_proc.rs` | WM_DISPLAYCHANGE / WM_TIMER / WM_USER_* 钩子、收尾收养调用 |
| `src/platform/windows/grid_window.rs` | 新字段 + 收养/持久化 + create hint + 会话收尾 rehome |
| `src/platform/windows/monitor_utils.rs` | get_monitors_named + 设备名解析 |
| `src/platform/windows/ffi_types.rs` | MONITORINFOEXW / 新 WM_USER / TIMER id |
| `src/platform/windows/glass_engine.rs` | MonitorsChanged → PostMessage 广播 |
| `src/config/app_config.rs` | BoxMonitorOrigin 字段（serde default 兼容） |
| `src/tauri_commands/platform_runtime.rs` | restore_layout 按屏身份映射 |
| 其余 GridCreateParams 构造点 | 补 hint 字段 |

## 复测与自检记录（2026-11-27，两轮实机反馈后）

### 第一轮复测缺陷与修复
1. **双重 DPI 换算（尺寸忽大忽小）**：对账层预换算 + 系统 `WM_DPICHANGED` 建议矩形
   再换算相互叠加。定案：换算只由 `apply_relocation` 做一次（plan 层
   `target_physical_size` 已按逻辑口径算好目标物理尺寸），搬窗前挂
   `dpi_suppress_*` 抑制窗口（1s，命中即消费），`WM_DPICHANGED` handler 在窗口内
   只刷口径不缩尺寸。系统救援自身的 DPI 消息不受影响。
2. **归属被系统救援污染（重连不搬回）**：Win11 拔线会把窗口自动搬到剩余屏，
   对账 `Settled` 若照常收养即丢失原屏身份。修复：`plan_for_box` fits 分支中
   「origin 存在且不在岗」时保留原身份并置 relocated=true（等待搬回），
   只有用户驱动收尾（adopt）才允许改判归属。
3. 运行环境坑：shell 继承 `RUST_LOG=warn` 导致 INFO 对账日志不可见，
   排障时须用干净环境启动。

### 三维自检结论（代码审查定案）
- **不过度**：`origin_matches` 三规则各对应一种真实 Windows 行为（改名/挪位/改分辨率）；
  `Snap` 分支服务历史配置兼容；`ReconcileOutcome` 四态是定时器收敛所需；
  对账拍=每轮显示事件最多 2 拍（首轮 + 1.5s 确认），Busy 不消耗拍。
- **Rust**：全 saturating / i64 / i128 防溢出、生产代码 0 unwrap/expect、
  `#[must_use]` 覆盖纯函数、MONITORINFOEXW 头部兼容指针转换是 Win32 标准惯例
  （cbSize 用 size_of 全结构）、f32 相等比较仅用于同窗自算记账的幂等对比、
  `From<&T>` 双向转换无孤儿规则问题。
- **架构**：几何纯逻辑落 box_core（wnd_proc / grid_window 只加薄钩子）；
  box_core→config 有 view_engine 先例且 config 未反向依赖；写回全部经
  `drag_handler::save_config` → `write_platform_config` 唯一入口；
  对账动作全部在窗口线程（无跨线程 SetWindowPos）；玻璃会话期间让位，
  不重复实现库判据（tauri-window-contract）。

### 已知残留风险（如复现再修，勿盲改）
- `cur_dpi = GetDpiForWindow` 竞态：对账拍若晚于系统救援完成，读到的是救援后 DPI；
  逻辑尺寸推导此时依赖「救援消息已把尺寸换算到位」。实机两轮通过；若再出现
  「拔屏后尺寸不对」，加固方向 = 对账尺寸改用 96 逻辑基准现算（不依赖窗口当前上下文）。
- 启动恢复 `monitor_dpi_hint` 在初始落点跨屏的错误 DPI 组合下有理论漂移面
  （本机布局不触发）。
- 抑制窗口未命中时残留 1s：期间恰好的真实跨 DPI 用户拖框会被跳过一次换算
  （概率极低，命中即清除，下一次消息恢复正常）。
