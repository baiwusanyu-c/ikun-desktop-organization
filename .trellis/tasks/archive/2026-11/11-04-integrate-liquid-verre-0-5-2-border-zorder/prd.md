# 接入 liquid-verre-rs 0.5.2：修「下层框 border 盖到上层框」

## 目标

依赖从 liquid-verre-rs 0.5.1 升到已发布的 0.5.2（库侧 8 条修复已 release，HEAD 8af57b8），
并按《liquid-verre-rs-0.5.2-库侧修复结果与应用层接入指南》退役宿主侧对应的临时绕行，
落地修复本次主诉：**缩放反馈 border 曾是「独立叠加 pass、画在全部玻璃之上」，
两块重叠时拖/悬停下层框边缘，白框画到上层框身上**。

## 需求

### R1 [#1 P0 本次主诉] border 交回块 z 序
- Cargo.toml：liquid-verre-rs 0.5.1 -> 0.5.2。
- 删除 grid_window.rs::begin_glass_resize_session 里**专为 border** 加的 raise_box 绕行
  （库 CHANGELOG 明确记录了宿主复验结论：raise_box 修不了这个缺陷）。
- border 发射序由库 core::draw::border_emit_plan 保证：逐块循环内紧跟自己那块玻璃，
  早于任何更高块。语义变化：**下层框的白边会被上层框的玻璃正确遮挡**（谁在上谁赢）。
- glass_engine.rs::raise_box 本体保留（wnd_proc.rs 的 Z 序不变量仍在用），只删 border 专用调用点。

### R2 [#5 升版必改] 双击改由库相位驱动
- 库画布类补齐 CS_DBLCLKS 后，物理双击的事件序列是 Down -> Up -> DoubleClick -> Up，
  **第二次按下不再产生 Down**。sink 里 is_double_click + last_down_* 合成逻辑变成死代码，
  且与新相位并存有重复投递风险 -> 整体删除，只匹配 PointerPhase::DoubleClick。
- hand_off_move 判定去掉 double_click 相关项。

### R3 [#2 + #3] 层级压底改走库 API
- 库侧：显示与层级同一次 SetWindowPos；等值 set_window_level 也重发（幂等）。
- 退役 4ms 高频压底守护线程（spawn_bottom_guard）。
- 退役「自己拿 canvas_handle 发 SetWindowPos(HWND_BOTTOM)」，改为：O(1) 位次判据不变，
  真的被抬起来时调 Engine::set_window_level(AlwaysOnBottom)。
- 保留启动兜底点：引擎启动后、Ready、MonitorsChanged、引擎重建后、
  新建块显示后、WM_USER_PUSH_GRIDS_BELOW、runtime 100ms tick。

### R4 [#4] 会话快照去掉催发布 hack
- 删除 request_render() 里把 set_default_resize 当「发布催办器」的绕行；只保留 Engine::request_render()。
- 跟随 tick 保留（内容窗口仍需按快照搬动），注释口径更新为「块几何变化 => 同帧快照可见」，
  并记录 moving/resizing 现已能如实回落（会话收尾判据可靠）。

### R5 [#6] 删除 debug overflow 止血
- 删掉 Cargo.toml 里 [profile.dev.package.liquid-verre-rs] overflow-checks = false。
- revive_if_dead 按库侧建议保留一个版本周期，本轮不动。

### R6 [#7 / #8] 不改
- 保留宿主下发前夹 min_width/min_height（库侧语义仍是硬下限）。
- 宿主从未设过 GLASS_FRAME_LATENCY 环境变量，EngineConfig::frame_latency 本轮不接面板。

## 验收标准

**已在提交前完成（有实测数据）**

- [x] `cargo check --all-targets` / `cargo test` **524 passed / 0 failed** / `cargo fmt --check` 干净 /
      clippy 无新增告警（基线 125（bin）与 139（test bin）持平，且逐条确认没有告警指向本轮改动的函数）
- [x] 依赖已升到 **0.5.3**，并在**发布产物**（`~/.cargo/registry/src/*/liquid-verre-rs-0.5.3/`）里逐处
      核对 #9/#10/#11 真落地（`topmost_body_z_at` / `grip_reachable` 过滤句 / `cursor_over_canvas` /
      `point_i32` / `hover_gate_blocks` 合取 + 钉死绕行），不是只信指南
- [x] border **绘制层序**已实机确认：下层框白边不再画到上层框身上（用户 2026-11-04 复验原话）

**提交时仍未做实机复验（不谎报为已验，交接给下一次开机自测）**

- [ ] 「该不该亮」的归属：上层框机身压住下层框边缘 ⇒ 下层框不再亮白边、不再变缩放手型（库 0.5.3 #9 已修，**待宿主实测**）
- [ ] 外来窗口盖住/撤走后**各要动一下鼠标**才生效（库 0.5.3 #10 的时效语义，静止鼠标不算反例）
- [ ] ⚠ 本轮接线读出的新风险：`cursor_over_canvas` 只认画布类名，而宿主标题/图标区是
      「alpha=1 命中捕获层 + `WM_NCHITTEST` 返回 `HTTRANSPARENT`」；若 `WindowFromPoint` 不尊重
      `HTTRANSPARENT`，鼠标点光 / hover 高光会在宿主的框里**整体消失**（库侧 playground 测不到该组合）。
      详见本文件末尾「接线时读出的新风险」，实测 10 秒即可定性，回归则开 0.5.4 清单
- [ ] 启动 12s 内画布低于其他应用窗口；控制面板不整页空白（4ms 压底守护已退役，靠关键时机 + 100ms tick 判据）
- [ ] 双击标题只打开一次编辑框（库真相位接管，宿主合成已删）
- [ ] debug 构建连跑 > 5 min 不 panic（本轮**删掉了** `overflow-checks` 止血段，这项比平时更该实测）

## 技术笔记

- 库仓库 D:/github-project/liquid-verre-rs：8af57b8 release: 0.5.2，crates.io 已发 0.5.2，
  宿主继续用版本号依赖，不需要 path/git。
- 库侧闸门全绿：248 lib + 11 api_surface + 8 host_contract + 5 playground，
  verify_border_zorder.py 4/4（含反向对照）、verify_double_click.py 3/3、
  verify_session_snapshot.py 3/3；gate_perf.py 不劣于历史基线。
- 产品语义确认：本次以「上层框不被下层框的反馈白边污染」为准；操作中的框若确实需要整体在最前，
  由内容窗口激活链路（wnd_proc.rs Z 序不变量）负责，不再为 border 单独抬层。


## 本轮改动清单（2026-11-04 实现完成）

| 文件 | 改动 |
|---|---|
| `Cargo.toml` / `Cargo.lock` | `liquid-verre-rs` 0.5.1 -> 0.5.2；删除 `[profile.dev.package.liquid-verre-rs] overflow-checks = false` 临时止血段（#6） |
| `src/platform/windows/grid_window.rs` | **主诉修复**：`begin_glass_resize_session` 删除「为 border 抬层」的 `raise_box` 绕行，注释改写成 0.5.2 层序契约（#1）；新建块显示后改用 `press_canvas_bottom()`；`follow_glass_session` 的催发布注释更新（#4） |
| `src/platform/windows/glass_engine.rs` | 模块头新增「玻璃块层序」不变量；删除 `spawn_bottom_guard`（4ms 守护线程）与裸 `SetWindowPos(HWND_BOTTOM)` 路径，改为 `apply_canvas_bottom_level()`（走 `Engine::set_window_level`）+ 判据防抖的 `reassert_canvas_bottom()` + 关键时机的 `press_canvas_bottom()`（#2/#3）；`request_render()` 删除 `set_default_resize` 催发布 hack（#4）；删除 `is_double_click` / `double_click_window_ms` / `double_click_slop_px` / `RouteState.last_down_*`，双击只匹配 `PointerPhase::DoubleClick`（#5）；`EngineConfig` 补 `frame_latency: None`（#8，不暴露到面板） |
| `src/tauri_commands/platform_runtime.rs` | runtime tick 的压底兜底注释更新为「等值也生效 + O(1) 判据」口径 |
| `.trellis/spec/guides/tauri-window-contract.md` | 新增三段契约：玻璃块层序与 border、画布 Z 序兜底、指针事件与快照 |

**保留未动**：`revive_if_dead`（库侧建议再留一个版本周期）、`wake_canvas_idle` 双保险、
下发前夹 `min_width/min_height`（#7）、`wnd_proc` 的 Z 序不变量 `raise_box`（产品语义：激活框整体在最前）。

## 待用户实机复验（无法在 CI 里做的三项）

1. **border 层序（本次主诉）**：两个框重叠，鼠标悬停/拖拽**下层**框的边缘白边不再出现在上层框身上。
2. **启动层级**：冷启动后玻璃不再压在 Chrome/IDE 等窗口之上、控制面板不整页空白
   （4ms 守护退役后只靠关键时机 + 100ms tick 兜底，这项是本轮最大的回归风险点）。
3. **双击标题**：快速两击只打开一次编辑框（库真相位接管后若出现「开了又关」说明相位重复投递）。

## 后续（2026-11-04 复验后的新观察，已出库侧清单）

用户复验：0.5.2 之后**白边不再盖到上层框身上（#1 已修对）**，但**白边仍然亮在被上层框机身压住的下层框边缘上**，
手型光标同样错。定位结论：这是**命中判据**缺陷，不是绘制判据，且**宿主无法正确修**（宿主没有每帧钩子，
亮边/把手/手型三样都来自库内部 `hover_id` + `hover_handle`，唯一旋钮 `set_resizable(false)` 会连没被盖住的那段
边缘一起关掉）。根因三条已在 0.5.2 发布产物里逐处核对：`runtime/mod.rs:647-666`（每帧纯几何轮询光标，
`hit_resize` 无条件优先于 `hit_test`）、`core/manager.rs:96-126`（`hit_resize` 只在把手候选之间比 z，
从不问「这点有没有被更高块本体盖住」）、`core/handle.rs:169`（把手带外扩 `grip_px * scale`）。
库里其实已经有正确判据（`core/manager.rs:81-88` 的 `hit_test` 认 z 序），只是没复用。

=> 已生成库侧修复清单：`.trellis/tasks/11-04-integrate-liquid-verre-0-5-2-border-zorder/liquid-verre-rs-0.5.3-库侧修复清单-下层框hover命中未让位上层块.md`
   （桌面同名副本一份）。条目 #9（P0 把手命中加本体遮挡过滤 + 6 条纯函数单测 + `verify_grip_occlusion.py`）、
   #10（P1 `WindowFromPoint` 门控，管外部窗口遮挡）、#11（P2 三处旧口径：`occlusion.rs:26-30` 推论、
   `BASELINE.md` 不变量 10、`draw.rs:121`/`glass_renderer.rs:816` 注释 + 新增不变量 13「绘制层序与命中层序同判据」）。
   **本轮宿主侧不再改动**，等 0.5.3 发版后只做接线与复验。

---

## 0.5.3 接线记录（2026-11-04 第二轮）

库侧按《0.5.3 库侧修复清单》把 #9 / #10 / #11 全部落地并发版：`8f18a6c release: 0.5.3`（tag 已打、
crates.io 已有 0.5.3）。**宿主本轮只接线，代码零逻辑改动**：

| 动作 | 内容 |
|---|---|
| 依赖 | `Cargo.toml` / `Cargo.lock`：`liquid-verre-rs` 0.5.2 -> 0.5.3（`EngineConfig` 无新增必填字段，`cargo check` 一次通过） |
| 发布产物核对 | 不轻信文档：在 `~/.cargo/registry/src/*/liquid-verre-rs-0.5.3/` 里逐处确认 `core/manager.rs:36 topmost_body_z_at`、`:53 grip_reachable`、`:158 if !grip_reachable(..) { continue; }`、`platform/window.rs:756 cursor_over_canvas` + `:776 point_i32`、`runtime/mod.rs:69-70 hover_gate_blocks(pinned, cursor_on_canvas) = !pinned && !cursor_on_canvas`（合取 + 钉死光标绕行都在） |
| 影响面 | 全仓只有 `glass_engine.rs:1388`（`debug_log_blocks`）读库侧 `hovered`，纯诊断；业务 hover 走宿主自己的 `WM_MOUSEMOVE`，所以「归属上移到上层块」不改宿主行为，只让诊断日志变准 |
| 注释口径 | `glass_engine.rs` 模块头「玻璃块层序」补 0.5.3 命中同判据；`grid_window.rs::begin_glass_resize_session` 补一句「争议已从画在哪变成该不该亮」 |
| 规范 | `tauri-window-contract.md` 的「已知未修（等库 0.5.3 #9）」整段替换为新契约：命中层序=绘制层序、`WindowFromPoint` 门控的**时效语义**（要动一下鼠标）、禁止宿主重算判据、禁止两种旧绕法、`hovered` 是快照不是实时信号 |

门禁：`cargo fmt --check` 干净、`cargo test` 524 passed / 0 failed、`cargo check --all-targets` 与
`cargo build` 无新增告警（基线 47 / 57 持平）。

**复验两条新须知（来自库侧指南 §4，不是缺陷）**：
1. 外来窗口盖上来 / 撤走，**都要动一下鼠标**才生效（门控只在光标移动过的帧查）。别拿静止鼠标当反例。
2. 被上层框压住的那段边缘，归属整个上移到上层框 ⇒ `debug_log_blocks` 里 `hovered=true` 的会是上层那个 id。

**库侧遗留（不阻塞宿主）**：0.5.3 的性能结论按 `measurement-and-gates.md` §2 还欠一次
「轮转 A/B + ≥3 重复 + 中位数」（本轮机器明显更忙，`own_us` run-to-run 散布 ±25 us）。
若日后出现「拖拽时玻璃变慢」的怀疑，先复测这一项，别先动宿主代码。

## 接线时读出的新风险（0.5.3 #10 门控 vs 宿主窗口架构，待实测定性）

库的 `cursor_over_canvas` = `WindowFromPoint → GetAncestor(GA_ROOT) → 类名是不是画布`。
而宿主这边（`d2d_render.rs:1352-1356`）：**引擎激活时整块矩形（除把手带）铺 alpha=1 命中捕获层**，
标题空白区靠 `WM_NCHITTEST → HTTRANSPARENT` 把鼠标让给画布（`wnd_proc.rs:377+`、
`glass_engine.rs` 的 `down_routes_title_drag` → `DragMove`）。

⇒ 如果 `WindowFromPoint` **不看** HTTRANSPARENT（只做区域 + 分层 alpha 判定），那么光标压在**宿主自己的
分类框内容窗口**上时 `cursor_over_canvas == false`，门控会清掉 `app.hover` ——
而块本体正是靠 alpha=1 捕获输入的，于是「鼠标点光 / hover 高光」在宿主的框里**几乎永远不出现**，
只剩把手带那一圈还有。这个回归库侧测不到：playground 的块上面**没有**别的内容窗口。

两种判定的实测差异只要 10 秒：光标停在一个框的标题空白/图标区，看玻璃里的鼠标点光与 `hovered=` 日志。

- 若确认回归 → 给库开 0.5.4 清单，三个候选改法（按优先级）：
  (a) `cursor_over_canvas` 放行「该块注册过的内容窗口」——宿主早已用
      `GlassWindow::set_focus_hwnd(hwnd)` 把这些 HWND 交给库了（`glass_engine.rs:1023`），数据现成；
  (b) 门控只清把手态（`hover_handle` / feedback），不清 `hover` 本体高光；
  (c) 按压态 `RouteState::Pressing` 与 `app.drag` 同等豁免（会话中不改判，库已有细则 4，但只认 `drag`）。
- 若无回归（说明 `WindowFromPoint` 走的是完整命中链、尊重 HTTRANSPARENT）→ 什么都不用做，
  把这条结论回填本节并写进 `tauri-window-contract.md` 的门控口径里。
