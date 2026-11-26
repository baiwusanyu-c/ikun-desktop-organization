# liquid-verre-rs 0.5.4 缺陷报告：非 1.0 scale 屏上缩放「锚定边」漂移

> 归属：`chapanda-ds-organizers`（宿主）在排查用户缺陷单「副屏拖拽分类框左边缘时，
> 框一边变小一边向右漂移，主屏正常」时定位到库侧根因。
> 库版本：`liquid-verre-rs = 0.5.4`（crates.io 最新版，仓库 master HEAD = `2d7f8b88`，
> 即 v0.5.4，**上游尚无修复**）。所有行号均按 0.5.4 发布包源码。

## 1. 一句话结论

`ResizeState::apply` 在缩放时把**逻辑像素**的尺寸增量直接加到**物理像素**的 `x/y` 上，
违反库自己在 `src/input/drag.rs:8` 立下的不变量（「缩放时锚定不动边……固定边必须纹丝不动」）；
同时运行时每帧取的 `drag_scale` 来自 `app.scale_of(b.monitor)`，与会话原点/快照用的 `b.scale`
不是同一个值。两条叠加后：**只要块所在屏的 scale 不等于 1，被拖边的对侧边就会随光标同向漂移**，
主屏（scale=1）完全看不出问题——与用户描述「主屏没有问题，副屏会出现」精确对应。

宿主侧无法修复，已放弃补偿方案（详见第 6 节）。

## 2. 复现环境

| 屏 | 物理矩形 | DPI | scale = 该屏DPI/主屏DPI |
|---|---|---|---|
| DISPLAY1（主） | (0,0)-(3120,2080) | 192 | 1.0 |
| DISPLAY2（副） | (3120,0)-(5040,1080) | 96 | 0.5 |

进程为 Per-Monitor-V2，分类框 = 宿主代理窗口（隐形，只承载命中层与 D2D 内容）+
库画布上的一块玻璃。缩放由库内建把手驱动（`resize.enabled = true`）。

## 3. 量化证据

### 3.1 合成输入逐帧取点（副屏块，拖左缘向右 200 物理像素）

```text
光标    3139 -> 3339   (+200)
块左缘  3139 -> 3339   (+200)   正确：跟手
块右缘  3637 -> 3737   (+100)   错误：锚定边位移应为 0
```

### 3.2 真实鼠标操作（宿主已修掉 min 尺寸双重换算后的版本，11:48:51 -> 11:48:53）

```text
块矩形 (4056,31)-(4412,327)   物理宽 356
块矩形 (4030,31)-(4399,327)   物理宽 369
=> 左缘位移 -26 = 光标位移；宽度变化 +13 = 26 x 0.5；右缘位移 -13 (= 26 x (1 - 0.5))
```

**规律：Δ(被拖边) = δ，Δ(物理宽) = δ x scale，Δ(锚定边) = δ x (1 - scale)。**
scale = 1 时锚定边位移为 0，scale = 0.5 时为一半 —— 两轮实测都落在这条线上。

## 4. 源码定位

### 4.1 量纲错误（主因）

`src/input/drag.rs:6-7` 明确了单位口径：

```rust
//! - `origin` 的 `x, y` 是虚拟屏物理像素，`w, h` 是**逻辑单位**（主屏物理像素）
```

`src/input/drag.rs:187-209` 的缩放算术：

```rust
let dx = (cursor[0] - self.start_cursor[0]) / s;   // 逻辑增量
...
} else if self.edges.left {
    w = (w0 - dx).max(min_w);
    x = x0 + (w0 - w);      // <-- 逻辑增量直接加到物理 x，缺 * s
}
if self.edges.bottom {
    h = (h0 + dy).max(min_h);
} else if self.edges.top {
    h = (h0 - dy).max(min_h);
    y = y0 + (h0 - h);      // <-- 同一个错
}
```

`x = x0 + (w0 - w)` 里的 `(w0 - w)` 是逻辑像素，而 `x` 是物理像素。右缘
`R = x + w * s` 的位移因此变成 `dR = ds_logical * (1 - s)`，仅在 `s == 1` 时为 0。

### 4.2 两套 scale 并存（叠加因素）

| 位置 | 取 scale 的方式 |
|---|---|
| `src/platform/window.rs:551`（建会话原点） | `let s = b.scale.max(1e-3);` |
| `src/runtime/mod.rs:740-742`（每帧算增量） | `app.scale_of(app.manager.get(id).map_or(0, |b| b.monitor))` |
| `src/runtime/mod.rs:766-767`（写回物理尺寸） | `b.width = r[2] * scale;`（同一个 `drag_scale`） |
| `src/runtime/commands.rs:123`（宿主 set_rect） | `b.width = rect[2] * scale;`（来自 `b.scale`） |
| `src/runtime/app.rs:239-244`（scale_of） | `.map_or(1.0, |m| m.info.scale.max(0.05))` |

`App::scale_of` 在 `b.monitor` 越界/表未刷新时**静默返回 1.0**，而 `b.scale` 可能已经是
0.5，两套值混用会让「增量换算」和「物理换算」各按各的 scale 走。本机测得的
「左缘按 1:1 跟手、宽度按 0.5 折算」正是 `drag_scale = 1.0 / b.scale = 0.5` 的形态。

### 4.3 测试矩阵的盲区

`src/input/drag.rs` 里所有会动到 `x/y` 的锚定用例（`:289` `:308` `:319` `:327` `:338`）
**全部传 `scale = 1.0`**；唯一带 scale 的缩放用例 `:356-362`
`resize_converts_cursor_delta_by_dpi_scale` 用的是 `E::RIGHT` —— 拖右缘根本不改 `x`。
也就是说「left/top + scale != 1」这一格从来没被测过，缺陷因此一路发布到 0.5.4。

## 5. 建议补丁

### 5.1 `src/input/drag.rs`

```diff
-        } else if self.edges.left {
-            w = (w0 - dx).max(min_w);
-            x = x0 + (w0 - w);
+        } else if self.edges.left {
+            w = (w0 - dx).max(min_w);
+            // w0 - w 是逻辑增量，必须 x s 才是物理增量；否则 scale != 1 的屏上
+            // 锚定边会随光标同向漂移（宿主 2026-11-26 实测：副屏 192/96 DPI）。
+            x = x0 + (w0 - w) * s;
         }
         if self.edges.bottom {
             h = (h0 + dy).max(min_h);
         } else if self.edges.top {
             h = (h0 - dy).max(min_h);
-            y = y0 + (h0 - h);
+            y = y0 + (h0 - h) * s;
         }
```

### 5.2 `src/runtime/mod.rs`

```diff
-        let drag_scale = drag_id.map_or(1.0, |id| {
-            app.scale_of(app.manager.get(id).map_or(0, |b| b.monitor))
-        });
+        // 必须与「建会话原点」(`platform::window`) 和「快照物理尺寸」用同一个
+        // scale：`b.scale`。`scale_of(b.monitor)` 在 monitor 下标越界时会静默
+        // 退化成 1.0，导致增量与物理换算按两套比例走。
+        let drag_scale = drag_id
+            .and_then(|id| app.manager.get(id))
+            .map_or(1.0, |b| b.scale.max(0.05));
```

### 5.3 `src/runtime/app.rs`（可选加固）

```diff
     pub fn scale_of(&self, monitor: usize) -> f32 {
         self.monitors
             .get(monitor)
-            .map_or(1.0, |m| m.info.scale.max(0.05))
+            // 越界说明调用方拿着过期的 monitor 下标：宁可返回最后一屏的比例，
+            // 也不要静默 1.0（1.0 在 200% 主屏机器上是「错得最难发现」的值）。
+            .map_or_else(
+                || self.monitors.last().map_or(1.0, |m| m.info.scale.max(0.05)),
+                |m| m.info.scale.max(0.05),
+            )
     }
```

### 5.4 建议补的回归测试（钉死这条不变量）

```rust
#[test]
fn resize_left_edge_anchors_right_side_across_dpi_scales() {
    // x 是物理像素、w 是逻辑像素：右缘 = x + w * s 必须与 scale 无关地钉住
    for s in [0.5f32, 1.0, 1.5, 2.0] {
        let mut d = resize_session(E::LEFT);
        let before = { let r = d.apply([0.0, 0.0], 0.004, s, [24.0, 24.0]); r[0] + r[2] * s };
        let r = d.apply([80.0, 0.0], 0.020, s, [24.0, 24.0]);
        let after = r[0] + r[2] * s;
        assert!((after - before).abs() < 0.5, "scale={s}: 右缘漂了 {}", after - before);
    }
}

#[test]
fn resize_top_edge_anchors_bottom_side_across_dpi_scales() {
    for s in [0.5f32, 1.0, 1.5, 2.0] {
        let mut d = resize_session(E::TOP);
        let before = { let r = d.apply([0.0, 0.0], 0.004, s, [24.0, 24.0]); r[1] + r[3] * s };
        let r = d.apply([0.0, 60.0], 0.020, s, [24.0, 24.0]);
        assert!((r[1] + r[3] * s - before).abs() < 0.5, "scale={s}");
    }
}
```

（`E::LEFT` 在对角把手下同样成立，`resize_corner_*` 系列建议一并补 scale 维度。）

## 6. 为什么宿主侧修不了（已试过的三条路，全部回退）

1. **会话开始时把校正后的矩形 push 回库**（纠正库的会话原点）：无效。库每帧都用
   `ResizeState.origin` 重算整块几何并覆盖 `b.x/b.y/b.width`，宿主的 push 下一帧即被抹掉。
2. **只夹代理窗口**（宿主把对侧边钉住）：无效且更难看。可见的那一侧是**库画的玻璃块**，
   结果是「玻璃走、内容不走」的双层错位。
3. **宿主 `glass_resize_floor` 的 DPI 双重换算**：这是**宿主的真 bug**，已修（改成常量
   逻辑值 200x140）。修完后主屏彻底正常，但副屏漂移依旧 —— 反证剩余部分在库。

结论：缩放会话的几何真源在库手里，宿主只能观测。宿主侧已改为「会话期只做虚拟桌面边界
夹取 + 收尾时一次性单屏归位 + 结束后 300ms 反向下发宽限期」，不再与库抢几何。

## 7. 附带发现（同一次日志取证里抓到的）

- **一次拖拽内 `ResizeStarted / ResizeEnded` 反复成对触发**：日志表现为
  宿主 `[GLASS-DIAG] 事件丢失兜底：库块已在会话中但本地未置位` 之后紧跟
  `resizing=true`。库侧 `src/platform/window.rs:482-502` 是「收到 Up 就无条件
  `app.drag.take()` 收尾」，会话被反复重开意味着**每一轮都以已漂移的块作新原点**，
  漂移因此逐轮累积（实测一轮 +100px，同一次操作三轮）。建议库：Up 之前校验
  `GetCapture() == hwnd` / 按键状态，或给会话加单调 session id，让宿主能识别「重开」。
- **`src/runtime/commands.rs:123` 的 `set_rect` 不会结束会话**，但会覆盖 `b.x/b.width`，
  与进行中的 `ResizeState.origin` 形成「两个真源」；建议库在会话中对 `set_rect` 要么
  忽略要么同步重置 origin，并在文档里写明。
- `Cargo.toml` 元数据：crates.io 上登记的 `repository` 是
  `https://github.com/baiwusanyu-c/tarui-glass`，实际仓库是
  `https://github.com/baiwusanyu-c/liquid-verre-rs`（404 与仓库名不一致，提 issue 时容易绕路）。

## 8. 宿主这边「如何更新 liquid-verre-rs」

当前锁定：`Cargo.toml:39` `liquid-verre-rs = "=0.5.4"`（精确锁，注释里写明
「升级必须配合适配层回归（2026-11-04 自检审计 P2）」），`Cargo.lock` 同版本。

### 方案 A：本地改库先验证（最快，推荐做 A/B）

```powershell
# 1. 克隆库（宿主仓库同级目录，别放进本工程目录树）
git clone https://github.com/baiwusanyu-c/liquid-verre-rs D:\chapanda-project\liquid-verre-rs
# 2. 应用第 5 节补丁（drag.rs 两处 * s + mod.rs 的 drag_scale + 两条回归测试）
cd D:\chapanda-project\liquid-verre-rs
cargo test
# 3. 宿主侧用 [patch] 指过去，不动依赖行，随时可摘掉
```

`Cargo.toml` 末尾临时加：

```toml
[patch."https://github.com/rust-lang/crates.io-index"]
liquid-verre-rs = { path = "../liquid-verre-rs" }
```

> 用 `[patch]` 而不是直接把 `=0.5.4` 改成 path 依赖：保留原依赖行不丢注释语义，
> 验证完删掉这两行即可回到干净状态。若报「patch 未使用」，说明 `=0.5.4` 的精确
> 要求把补丁挡了，此时改成 `liquid-verre-rs = { path = "../liquid-verre-rs" }`
> 直接验证（验证完还原）。

验收动作（本机的判据）：副屏拖左缘 3 次、每次 200px，看日志里块矩形
`x + w x scale` 是否恒定；再主副屏来回拖一次，确认玻璃与图标不错位。

### 方案 B：直接吃未发版的 master

```toml
liquid-verre-rs = { git = "https://github.com/baiwusanyu-c/liquid-verre-rs", rev = "<commit>" }
```

适用于「库已修好但还没发版」。注意：走 git 源时 `=0.5.4` 这种精确版本约束写法不适用，
需要临时改掉依赖行，并且**发版后必须换回 crates.io 版本**，否则 CI/其他人拉不到私有环境路径。

### 方案 C：库正式发版 0.5.5（最终形态）

库侧（`liquid-verre-rs` 仓库）：

```bash
cargo test                       # 含新加的两条 scale 回归测试
# 改版本号：Cargo.toml version = "0.5.5" + 更新 CHANGELOG/注释里 0.5.4 的行为约定
cargo package --list             # 确认没漏文件
cargo publish
git tag v0.5.5 && git push --tags
```

宿主侧：

```powershell
cd D:\chapanda-project\chapanda-ds-organizers
# 1. 改依赖行：Cargo.toml 里 =0.5.4 -> =0.5.5，并把 36-38 行的注释版本一并更新
cargo update -p liquid-verre-rs
cargo check --all-targets
cargo test
cargo clippy --all-targets -- -D warnings <既有 allowlist>
cargo fmt -- --check
```

适配层回归清单（`glass_engine.rs` 依赖的库行为约定，逐条过）：

1. `block_content_rect` 的 `snapshot.rect` 单位 = 库逻辑像素、`snapshot.scale` = 该屏比例
   （本次已按此口径改过 `glass_resize_floor`，别再按窗口 DPI 二次换算）；
2. 块几何变化 => **同帧快照可见**（0.5.2 契约），宿主的会话跟随 tick 依赖它，
   退化成 250ms 兜底档就会复现「图标比玻璃慢半拍」；
3. 画布显示与层级同一次 `SetWindowPos`（0.5.2/0.5.3 约定，影响「下层框白边盖上层框」）；
4. 库内建 100ms 唤醒定时器（宿主 `switch_glass_timer` 的档位切换与之配合）；
5. 事件：`ResizeStarted.handle` / `MoveStarted` / `ResizeEnded.rect` 是否仍是现有形状
   （本次已把 `handle` 转成位标志随 `wparam` 透传，方向若变要同步改）；
6. 多屏三场景：主屏缩放、副屏缩放（本次缺陷）、跨屏过渡（一半主屏一半副屏）。

## 9. 给库维护者的最小复现（可直接贴 issue）

- 双屏：主 192 DPI（3120x2080）、副 96 DPI（1920x1080，x 偏移 3120），进程 PMv2；
- 在**副屏**放一块玻璃（`resize.enabled = true`，`min_width/min_height` 用逻辑像素 200/140）；
- 用鼠标按住块的**左缘**向右拖 200 物理像素（或 `SetCursorPos` + `mouse_event` 合成）；
- 期望：块右缘不动（`x + w x scale` 守恒）；
- 实际：块右缘向右漂约 100px（= δ x (1 - scale)），且一次拖拽内 `ResizeStarted/ResizeEnded`
  会成对多次触发，漂移逐轮累积。主屏（scale = 1）不复现。
## 10. 结案：0.5.5 已根修（2026-11-26 13:08）

- 库侧 commit `14751faf`（tag `v0.5.5`，13:08 发布到 crates.io），修法比本报告 5.1 更彻底：
  **缩放会话全程改在物理域计算** —— 会话原点改存「按下时刻物理矩形」
  （`platform/window.rs:549`），`dx/dy` 不再 `/ s`，返回时 `/ scale`、写回端
  `b.width = r[2] * scale` 乘**同一个** scale（`runtime/mod.rs:771`），乘除相消 ⇒
  物理矩形与会话中途 scale 无关；`min_size` 仍是**逻辑**下限（库内部 `× scale`
  再垫 `MIN_SIZE`，与写回端 `b.clamp()` 同口径）。
- 新增 4 条判别单测（0.5 / 2.0 锚定零漂移、会话中途 scale 翻转物理不变、
  min-clamp 半 scale 锚定精确）—— 旧实现逐条必红，补上了第 4.3 节的测试盲区。
- 本报告 5.2 提到的 `app.scale_of(b.monitor)` 与 `b.scale` 双口径**未单独改**，
  但已被物理域方案消解（锚定律不再依赖 scale 取值正确），残留影响仅是
  「逻辑尺寸折算」的瞬态偏差，写回即还原。
- 宿主升级：`Cargo.toml` `=0.5.4 -> =0.5.5`（依赖行注释同步记录新契约），
  `cargo update -p liquid-verre-rs`；适配层**零代码改动**即通过
  `cargo check --all-targets` / `cargo test`（535 passed）/ CI 严格 clippy 组合 / `cargo fmt --check`。
- 适配层回归逐条结论：同帧快照可见（未变）/ 画布与层级同一次 SetWindowPos（未变）/
  100ms 唤醒定时器（未变）/ `ResizeStarted.handle`、`MoveStarted`、`ResizeEnded.rect`
  事件形状（未变）/ `snapshot.rect` 逻辑 + `snapshot.scale`（未变）/
  多屏三场景（主屏缩放、副屏缩放、跨屏过渡）待用户实机复验。
