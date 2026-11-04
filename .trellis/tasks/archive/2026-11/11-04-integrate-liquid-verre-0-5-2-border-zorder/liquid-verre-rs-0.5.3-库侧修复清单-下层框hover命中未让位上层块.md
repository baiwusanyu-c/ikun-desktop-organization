# liquid-verre-rs 0.5.3 库侧修复清单（宿主实测取证）

> 对象：库仓库 `D:/github-project/liquid-verre-rs`，基线 `8af57b8 release: 0.5.2`（crates.io `liquid-verre-rs 0.5.2`）
> 输入：分类框宿主 `ikun_organizer`（`D:/chapanda-project/chapanda-ds-organizers`）在 0.5.2 复验后的新观察
> 一句话：**#1 修对了 border 的「绘制层序」，但没修 border 的「触发判据」** —— 下层框的反馈白边不再盖到上层框身上，
> 却**照样在被上层框机身压住的那条边缘上亮起来**，还照样把手型光标变成缩放箭头；点下去又没反应。
> 本文所有行号来自 0.5.2 实际发布产物（`~/.cargo/registry/src/*/liquid-verre-rs-0.5.2/`），不是本地工作副本。

---

## 0. 三十秒结论

| 条目 | 优先级 | 是什么 | 谁改 |
|---|---|---|---|
| **#9** | **P0** | `hit_resize`（缩放把手命中）不比 z 序之外的遮挡 ⇒ 上层块的**机身**盖不住下层块的**边缘带**：hover/反馈 border/手型光标全落在下层块上 | **库改**（纯函数 + 单测） |
| **#10** | P1 | 每帧 hover 是**纯几何轮询全局光标**，与「画布此刻是否真的在光标下面」无关 ⇒ 被 Chrome/IDE 等**外部**窗口盖住的边缘同样亮边 | **库改**（平台侧一次 `WindowFromPoint`） |
| **#11** | P2 | 三处旧口径没跟上 0.5.2：`runtime/occlusion.rs:26-30` 的「命中测试不读 occluded」推论、`BASELINE.md` 不变量 10 的「border 由独立 pass 绘制」、`core/draw.rs:121` + `gfx/glass_renderer.rs:816` 的「raise_box 修不了 hover」注释 | **库改**（文档/注释，埋雷级别） |

宿主侧本轮**不需要改代码**：#9/#10 都不新增公共 API。

---

## 1. #9【P0】现象与复现

环境：单屏，液态玻璃开，缩放反馈 `feedback.enabled = true`，两块 A（下）/ B（上）重叠，
B 的机身盖住 A 的左边缘中段（`grip_px` 命中带落在 B 的矩形内部）。

| 光标位置 | 期望 | 实测 |
|---|---|---|
| B 机身内、同时落在 A 左边缘带上 | 归 B：不亮任何反馈边，光标是普通箭头 | **A 亮缩放反馈白边**（画在 A 自己的 pass 里 ⇒ 被 B 盖住的那段看不见、**没被盖住的两头露出来**），光标变成 `WZE`/斜向缩放箭头 |
| A 左边缘带上、B 之外 | A 亮、A 手型 | A 亮、A 手型（正确） |
| 上述第一行位置**按下左键** | 什么都不该发生（那点属于 B） | 手型说「能拉」、点下去拉不动（宿主的 alpha 命中带把 `WM_LBUTTONDOWN` 吃了，画布没收到）⇒ **观感与行为割裂** |

那条「两头露出来的白边」正是用户现在看到的：层序对了，但**根本不该亮**。

---

## 2. 判据错在哪（逐处钉死）

1. **`runtime/mod.rs:647-666`（每帧 hover 重算，本条缺陷的发动机）**
   ```rust
   // ---- 鼠标状态（每帧轮询，无 hook；方案 6.3）----
   app.cursor = app.read_cursor();
   let (new_hover, new_handle) = match app.drag.as_ref() {
       Some(d) => (Some(d.id()), d.edges().unwrap_or(ResizeEdges::NONE)),
       None => match app.manager.hit_resize(app.cursor[0], app.cursor[1]) {   // ← 把手无条件优先
           Some((id, e)) => (Some(id), e),
           None => (app.manager.hit_test(app.cursor[0], app.cursor[1]), ResizeEdges::NONE),
       },
   };
   ```
   每帧拿**全局光标**做纯几何判定，完全不看「这个像素此刻归谁」。
2. **`core/manager.rs:96-126` `hit_resize`** —— 只在**把手候选之间**比 z 序：
   ```rust
   match best { Some((z, _, _)) if z >= b.z_order => {} _ => best = Some((b.z_order, b.id, edges)) }
   ```
   B 根本不是把手候选（光标在 B 机身内部，不在 B 的边缘带），于是 B **连参与比较的机会都没有**，
   A 作为唯一候选当选。
3. **`core/handle.rs:169` `hit_handle`** —— 命中带向外扩 `grip_px * scale`，所以「B 机身内部的一个点」确实可以同时在 A 的把手带里。这是**特性**（块外一点也能抓边），不是 bug；缺的是下一步那句遮挡检查。
4. **`core/manager.rs:81-88` `hit_test`** —— 本体命中是认 z 序的（`max_by_key(|b| b.z_order)`）。
   也就是说**库里已经有正确判据**，只是 `hit_resize` 没复用它。这条决定了 #9 的修法很小。
5. **同族调用点一共四处，必须一次改到底**（否则四处会互相漂移）：
   - `runtime/mod.rs:654-662` 每帧 hover（本条主诉）
   - `platform/window.rs:252-255` `WM_NCHITTEST`：`hit_test(...).is_some() || hit_resize(...).is_some()` —— 下层框的把手带会让**画布在属于上层框的像素上声称自己有命中**
   - `platform/window.rs:361-366` `dispatch_pointer`：`hit_resize` 无条件优先于 `hit_test` ⇒ Down 归属同样会判给下层块（现在被宿主 alpha 命中带侥幸挡住，但库不该依赖宿主窗口来兜底）
   - `platform/window.rs:492-497` `forward_move_to_sink` 用的是 `hit_test`（**对的**）⇒ 于是出现「Move 转发给上层块、亮边亮在下层块」的自相矛盾，宿主 sink 与视觉反馈拿到的是两个不同的块。

---

## 3. 改法（推荐 R-A：把手命中加「更上层本体遮挡」过滤）

### 3.1 判据

> 一个把手候选 `b` 在该点**够得着**，当且仅当：不存在「`visible` 且 `z_order > b.z_order` 的块的本体矩形包含该点」。

本体矩形口径与 `hit_test` **完全一致**（用 `b.x/b.width` 精确矩形，不含阴影外扩、不含 grip 外扩），
这样「绘制层序」`border_emit_plan`、「本体命中」`hit_test`、「把手命中」`hit_resize` 三者共用同一套 z 语义。

### 3.2 实现形状（热路径约束必须守住）

`hit_resize` 每帧调用（`runtime/mod.rs` + `WM_NCHITTEST`），库里 spec 明确**不得分配**。建议两趟无分配扫描：

```rust
pub(crate) fn hit_resize(&self, x: f32, y: f32) -> Option<(u32, ResizeEdges)> {
    // 第一趟：该点被「最上层本体」覆盖到的 z（只看 visible；不看 interactive）。
    //   没有块覆盖时取 i32::MIN，第二趟自然全部放行。
    let mut top_body_z = i32::MIN;
    for b in self.blocks.iter() {
        if b.visible && x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height {
            top_body_z = top_body_z.max(b.z_order);
        }
    }
    // 第二趟：现有那段候选循环，多一句遮挡过滤，其余一行不动（包括 keep 那段方向判定）。
    let mut best: Option<(i32, u32, ResizeEdges)> = None;
    for b in self.blocks.iter() {
        if !(b.interactive && b.visible && b.resize.enabled) { continue; }
        if b.z_order < top_body_z { continue; }        // ← 本条修复的唯一新增判据
        /* …原有 hit_handle / keep / best 逻辑不变… */
    }
    best.map(|(_, id, edges)| (id, edges))
}
```

复杂度 O(2n)、零堆分配；n 是块数（宿主典型 4~20，极限 32）。**不要**写成「收集候选再排序」，
那是 `#1` 那次 `border_emit_plan` 每帧新建 `Vec` 把 `own_us` 从 14 抬到 142 的同款坑。

判据本体抽成纯函数（与 `core::draw::border_emit_plan` 同构，便于无 GPU 单测）：

```rust
/// 该点被哪一层「本体」压着：返回覆盖该点的块里最大的 z_order；无覆盖返回 `i32::MIN`。
#[must_use]
pub(crate) fn topmost_body_z_at(blocks: &[Block], x: f32, y: f32) -> i32

/// 遮挡过滤是否放行这个候选（单独抽出来，让单测能直接钉语义而不必构造整个 Manager）。
#[must_use]
pub(crate) fn grip_reachable(candidate_z: i32, top_body_z: i32) -> bool  // = candidate_z >= top_body_z
```

### 3.3 语义细则（六条，缺一条就会二次漂移）

1. **严格小于才抑制**（`candidate_z < top_body_z`）：z 相等不抑制 —— 否则 A 自己机身边缘内侧那一圈会把自己判死，`#9` 修完立刻出现「边缘内侧 1px 抓不动」的新缺陷。
2. **`visible == false` 的块不参与遮挡**：隐藏的块没有像素。
3. **`interactive == false` 的块参与遮挡**：它的玻璃**确实盖在上面**（锁定态、`set_interactive(false)` 的块都是这种），遮挡是视觉事实，与可交互性无关。
4. **会话中不改判**：`runtime/mod.rs:654` 已经在 hover 之前分了 `app.drag`，会话期间 hover 恒等于会话块，正确，别动。
5. **被抑制的候选直接丢弃，不降级成「该点命中上层块本体」**：那点的归属由后续 `hit_test` 自己给出（上层块机身 ⇒ 拖拽移动归上层块），`hit_resize` 只负责把手这条路。
6. **不要用 `Block::occluded`（8 Hz 遮挡轮询）当这个判据**：它是「**整块被完全覆盖** ⇒ 跳渲染」的粗粒度优化（`core/geometry.rs::rect_fully_covered` + `core/block.rs::occlusion_step` 迟滞两轮才置位，最坏 250 ms），而兄弟块之间是**逐像素局部覆盖**，用 `occluded` 判必错。顺带把 `runtime/occlusion.rs:26-30` 那段推论改掉（见 §7）。

---

## 4. 单测（`core/manager.rs` 现有 hit_resize 测试同风格，无 GPU）

现有参照：`core/manager.rs:341/345/358/360/373/383/398/401` 那批断言（`m.hit_resize(104.0, 250.0) == Some((id, ResizeEdges::LEFT))`）。新增：

| 测试名 | 钉住的语义 |
|---|---|
| `grip_under_higher_body_does_not_hover` | A 左边缘带里的点，B（z 更大）机身覆盖 ⇒ `hit_resize` 返回 **None** 或归 B，**绝不返回 A**（主诉本体） |
| `grip_outside_higher_body_still_hovers` | 同一条边缘、点在 B 矩形之外 ⇒ 照常 `Some((A, LEFT))`（防「修过头」把边缘带整个关掉，反向对照） |
| `grip_inside_own_body_is_not_self_occluded` | 只有 A 一块时，点 A 边缘内侧 1px ⇒ 仍命中 A（细则 1：相等不抑制） |
| `hidden_higher_block_does_not_occlude_grip` | B `visible=false` 时 A 的边缘照常亮（细则 2） |
| `non_interactive_higher_block_still_occludes_grip` | B `interactive=false` 但可见 ⇒ A 不该亮（细则 3） |
| `topmost_body_z_at_is_single_source_of_truth` | 纯函数逐点：多块叠加取最大 z、无覆盖取 `i32::MIN`、`grip_reachable` 真值表（含相等放行） |

反向对照要求：把 §3.2 那句 `if b.z_order < top_body_z { continue; }` 注释掉，第 1 条测试必红、其余 5 条仍绿。
这条「判别力」要求在库里有前科可循（`border_emit_plan` 的断言 4 就是同类护栏）。

---

## 5. 取证脚本 `tools/verify_grip_occlusion.py`（登记进 `verify_all.py` 第 9 项）

- 场景：playground 两块，A `z_order` 低、B 高，B 覆盖 A 左边缘中段。
- **oracle 用状态不用像素**：每块 `WindowSnapshot::hovered`（`api/handle.rs:164`）+ 宿方可读的把手态。
  理由是 `hovered` 是本条缺陷的直接判据，比像素稳定，也不与 `verify_border_zorder.py`（管绘制）重叠。
- 采样点：`p_covered`（A 左边缘带 ∩ B 机身）、`p_free`（A 左边缘带 ∩ B 外）。
- 断言：
  1. `p_covered`：`A.hovered == false`（**核心**）
  2. `p_free`：`A.hovered == true`（反向对照，防修过头）
  3. `p_covered → p_free → p_covered` 来回：两次都收敛，无粘滞（每帧轮询的复位正确性）
  4. （#10 落地后加一条）用外部窗口盖住 A 边缘时 `A.hovered == false`
- 首跑请把「删掉 §3.2 过滤句」的反向对照结果一并贴进 CHANGELOG（库里 `#1` 那节的格式很好，照抄）。

---

## 6. #10【P1】外部窗口盖住时同样亮边（建议同一版顺手做）

R-A 只管**同一画布内的兄弟块**。光标被 Chrome/IDE/宿主自己的其他窗口盖住时，画布每帧仍在轮询同一个全局光标 ⇒ 被盖住的边缘照样亮。这类现象在宿主上表现为「玻璃框在白边状态下被人家用窗口压住了，鼠标在人家的窗口里，白边还在亮」。

- 判据：每帧重算 hover 前，若 `WindowFromPoint(虚拟屏绝对坐标) != 本屏画布 hwnd`，则清空 `app.hover` / `app.hover_handle`（不亮、不改手型）。
- 成本：只在 `cursor_moved` 为真时查一次；`WindowFromPoint` 是 O(1)，且**天然尊重分层窗口的 per-pixel alpha 穿透** —— 宿主的 alpha=1 命中带正是靠这个语义工作，判据口径与宿主一致。
- 三条坑（写下来免得二次踩）：
  1. 多屏：`WindowFromPoint` 吃虚拟屏绝对坐标，块坐标是画布局部 + 屏原点，换算走 `runtime/occlusion.rs::to_abs` 那套 widen-then-clamp，别裸加。
  2. 画布自己可能正处于 `WS_EX_TRANSPARENT`（采集停产/全屏让路时库会置穿透）：那时 `WindowFromPoint` 不返回画布 ⇒ 结论也是「不该亮」，方向一致，无需特例。
  3. 不要用 8 Hz 的 `Block::occluded` 替代这条：粒度与时效都不对（见细则 6）。
- 分工建议：**R-A 是必修**（纯函数、可单测、直接消掉本次主诉）；**#10 是 P1**，若本轮不做，请在 CHANGELOG 与 `occlusion.rs` 注释里写明「外部窗口遮挡下的 phantom hover 已知未修」，别让下一条宿主清单再提一遍。

---

## 7. #11【P2】三处旧口径（不跟着改，就是给下一个改库的人埋雷）

1. **`runtime/occlusion.rs:26-30`**
   > 「命中测试与点击穿透（`inside`）也**不读** `occluded`：块被盖住时鼠标本来也落在上面那个窗口上，改穿透语义只会引入新的不一致。」

   前半句（不读 `occluded`）**结论保留**，后半句的**理由对兄弟块不成立**：两块玻璃在**同一个画布 HWND** 里，
   不存在「上面那个窗口」去吸收鼠标，而库是**每帧自己轮询全局光标**（`runtime/mod.rs:647`）。
   请改成：「兄弟块之间的遮挡由 `hit_resize` 的本体覆盖过滤判据负责（#9，`topmost_body_z_at`）；
   外来窗口遮挡走 #10 的 `WindowFromPoint` 门控；`Block::occluded` 仍然只是渲染级粗粒度优化，两者都不读它。」
2. **`BASELINE.md` 不变量 10**
   > 「border 由独立 pass（`border_ps.hlsl`）绘制，只作用于 hover 块」

   0.5.2 #1 之后 border 已收进逐块 pass（`gfx/glass_renderer.rs::draw_blocks` + `core::draw::border_emit_plan`），这句现在是错的。
   改写为：「border 在第 i 块玻璃之后、任何更高块之前发射；判据 `core::draw::border_emit_plan`；取证 `verify_border_zorder.py`」，
   并新增**不变量 13**：**绘制的层序与命中的层序必须共用同一套 z 判据** —— 只修绘制不修命中，就是本次 #9 的根因。
3. **`core/draw.rs:121` 与 `gfx/glass_renderer.rs:816`** 里「宿主 `raise_box` 修不了 hover 阶段的层间关系」
   这句在 #9 落地后要更新成「修不了的原因是把手命中判据缺遮挡项（#9），不是抬层能力缺」，
   否则下一个宿主会以为抬层仍然是可用绕法。

---

## 8. 宿主侧影响（ikun_organizer）

- **无公共 API 变化 ⇒ 宿主本轮不改代码**。#9 的判据完全落在 `core/manager.rs` 内部；#10 落在 `platform/window.rs`。
- 宿主复验清单（#9 发版后我来跑）：
  1. 两框重叠，光标放进 B 机身内正好压住 A 左边缘 ⇒ A 不再亮白边、手型不变缩放箭头。
  2. 同一条边缘移到 B 之外 ⇒ A 立刻亮（防修过头）。
  3. 锁定态 B（`interactive=false`）盖住 A 边缘 ⇒ A 不亮（细则 3）。
  4. 上层框缩小到只剩边缘带交叠的极端位置 ⇒ 归属仍由 z 序决定，与 `WM_NCHITTEST`/`forward_move_to_sink` 给宿主的块 id **一致**（消掉 §2.5 那个「Move 归 B、亮边归 A」的自相矛盾）。
- 取证通路：宿主侧 `glass_engine::debug_log_blocks()` 每秒打每块 `hovered=`（`RUST_LOG=debug`，日志 `~/.ikun_organizer_log`），发版前后各录一次即成对照。
- 建议版本号：**0.5.3**（纯 bugfix，无新增公共项；#10 也只在平台内部）。按库里的规矩，提交与发版由人工发起。

---

## 9. 闸门清单（照旧，一条不能少）

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --release -- -D warnings
cargo test --release --workspace          # 248 lib + 11 api_surface + 8 host_contract + 5 playground + 2 doctest，再叠加 §4 的 6 条
node scripts/check.mjs
python tools/gate_perf.py                 # 关键：每帧多一趟 O(n) 块扫描，own_us 必须不劣化（历史基线 single 14us / many-drag 30us）
python tools/verify_grip_occlusion.py     # 新增，登记进 verify_all.py 第 9 项
python tools/verify_border_zorder.py      # 既有回归：绘制层序不得回退
python tools/verify_mouse_isolation.py    # 既有回归：闭包外 0 差异
python tools/verify_content.py            # 既有回归：交错层序未回退
```

---

## 10. 本清单的证据边界（诚实声明）

- §2 的行号与代码摘录来自 **crates.io 0.5.2 实际发布产物**（`~/.cargo/registry/src/rsproxy.cn-*/liquid-verre-rs-0.5.2/src/`），逐处读过原文。
- §1 的「实测」列来自宿主侧 2026-11-04 的肉眼复验（层序已修对、亮边仍在下层框）。**本轮没有跑 GUI 取证脚本**，
  也没有做每帧 `own_us` 的成本实测 —— 所以 §3.2 的「两趟无分配」是按库内既有热路径规矩给的保守写法，
  代价请由 `gate_perf.py` 实测确认，别把我的话当数据。
- 若 §3.3 的细则 3（非交互块参与遮挡）与库内其它设计冲突，以「绘制与命中同判据」这条新不变量为准绳取舍，别退回抬层绕法。
