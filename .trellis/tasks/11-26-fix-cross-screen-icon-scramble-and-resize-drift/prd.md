# 修复：跨屏拖拽后图标位置变乱 + 副屏拖左边缘时右边缘向右漂移

## 目标

承接 11-26-fix-dblclick-and-multimonitor-drag（已提交 48d2619）后的两条实机反馈：

1. 主屏 ↔ 副屏之间拖拽后，**分类框内图标位置会变乱**；
2. 从**左侧边缘**拖拽改大小时，分类框一边变小一边**向右漂移**
   （主屏正常，副屏出现）。

## 实测环境（本机取证）

| 显示器 | 虚拟桌面矩形 | DPI | 库 scale |
|---|---|---|---|
| DISPLAY1（主，Windows primary） | (0,0)-(1560,1040) | 192（200%） | 1.0（基准） |
| DISPLAY2（副） | (3120,0)-(5040,1080) | 96（100%） | 0.5 |

- 三个分类框当前全部位于副屏，内容尺寸约 355x309 / 377x347 / 342x221；
- 配置 `box_style.liquid_glass.resize = { enabled: true, edges/corners: true,
  grip_px: 26, min_width: 605, min_height: 1150 }`。

## 根因

### 共同根因：玻璃缩放下限被 DPI **双重换算**

`glass_engine::glass_resize_floor(dpi)` 旧实现：

```rust
let ratio = monitor_dpi_ratio(dpi);            // dpi / 主屏 DPI
let min_w = scale_for_dpi(200, dpi) as f32 / ratio;  // 200*dpi/96 / ratio
```

库侧语义（已核库源码）：`scale = 该屏 DPI / 主屏 DPI`，
块**物理**尺寸 = 逻辑尺寸 × scale，且 `ResizeOptions.min_width/min_height`
是**逻辑像素**硬下限（`input::drag::ResizeState::apply` 里 `w.max(min_w)`）。
而 `monitor_dpi_ratio(dpi)` 已经等于库的 scale，所以正确换算只需
`scale_for_dpi(200, dpi) / ratio`，**当且仅当主屏 = 96 DPI 时**才等于 200。

本机主屏 192 DPI → 副屏（dpi=96，ratio=0.5）：
`scale_for_dpi(200,96)/0.5 = 400` 逻辑像素 = **400 物理像素**，
比历史最小框（355/377）还宽 → 副屏上每个框**天生低于硬下限**。

由此派生两个现象：

1. **Bug 2（右边缘向右漂移）**：拖左边缘时 `w = (w0 - dx).max(400)` 立刻被抬到 400，
   而锚定式 `x = x0 + (w0 - w)` 因 `w > w0` 变成负偏移 → 左边缘反而外扩、
   **右边缘持续右移**；主屏 ratio=1 时下限 200 < 框宽，故无此现象。
2. **Bug 1（图标变乱）之一**：尺寸被硬下限强行改动 → 列数变化 → 触发图标重排。

### Bug 1 的第二条独立缺陷：重排用「猜」的旧列数

`box_core::layout::reflow_positions` 旧实现用 `max(grid_col)+1` 当**旧列数**。
框里只要存在空洞（删过图标、手动留空位）或最后一行不满，该推断值就**偏小**
（列数变多时又必然偏大），按线性索引重映射随之整体错位，且**不可逆**。
跨屏拖拽途中 `WM_SIZE` 会多次触发重排，一次错位就永久留在配置里。

另外缺少「列数没变就不重排」的短路：跨屏 DPI 重采样后逻辑尺寸不变 → 列数通常不变，
此时任何一次线性重映射都是纯伤害。

## 需求

- R1 `glass_resize_floor` 不再按窗口 DPI 换算，直接返回库逻辑像素常量口径
  （宽 200、高 140 = 40 标题 + 100），与 `WM_GETMINMAXINFO` 的 96-DPI 基线一致；
  单测必须钉死「与 dpi 无关」且「不得被 DPI 二次放大」。
- R2 `reflow_positions` 新增 `old_cols_hint`（上一轮**真实生效**列数）：
  - 给了 hint 就**直接采信**（不与本轮列数取较大、不做双向 max）；
  - 没给才退回 `max(col)+1` 推断；
  - 列数不变 / 无固定位项 → 原样返回（幂等短路）。
- R3 `GridWindowData` 新增 `layout_cols`，由 `recalculate_layout` 每轮写入，
  `reflow_items` 读取并作为 hint 传入；<=0 时退回推断（兼容历史数据）。

## 验收标准

- [ ] 副屏（96 DPI）上从左侧边缘向内拖：右边缘**保持不动**，宽度按光标收敛到真实下限。
- [ ] 主屏 ↔ 副屏来回拖拽后，框内图标行列与拖拽前一致（有空洞的框尤其要验）。
- [ ] 列数未变的跨屏重采样不触发任何重排（幂等）。
- [ ] `cargo fmt --check` / CI 严格 clippy 组合 / `cargo test` 全绿。

## 技术笔记

- 库逻辑像素 = 主屏物理像素；`phys_to_logical_rect` 的「x/y 物理、w/h 逻辑」口径不变。
- 遗留（下一步候选）：布局序列化仍存「物理坐标 / 当时窗口 DPI」，
  跨 DPI 屏恢复的缩放基准可能与目标屏不符（需「显示器身份 + 屏内相对坐标」）。
- 不做 git commit，等用户明确指令。

## 定案（第 3 轮，2026-11-26）

| 问题 | 归属 | 处置 |
|---|---|---|
| 主副屏拖拽后图标位置变乱 | **宿主缺陷** | 已修：`glass_resize_floor` 的 DPI 双重换算（改常量逻辑值 200x140）+ 重排「旧列数」记账（`reflow_positions` 第三参 `old_cols_hint`，`layout_cols` 记账）。 |
| 副屏拖左边缘时右边缘持续向右漂 | **`liquid-verre-rs` 0.5.4 库缺陷** | 宿主侧无法修复：曾实现的锚点补偿（`glass_resize_anchor` / `apply_resize_anchor` / `session_content_rect`）已整体删除；根因、补丁、更新流程见同目录 `liquid-verre-rs-defect.md`。 |

**验收口径调整**：副屏（scale != 1）拖边缘仍会漂 = 库缺陷复现，**不再作为宿主回归项**；
主屏缩放、跨屏后图标行列、双击打开三项继续按上面的清单验收。