# brainstorm: 分类框对齐参考线功能

## 目标
当用户拖动/缩放分类框时，若其边缘与其他分类框的边缘在垂直或水平方向上对齐（差值 ≤ 吸附阈值），则显示一条穿过屏幕的参考线，经过对齐的两个边缘，辅助用户精确排列分类框。

## 已知信息
* 每个分类框是独立的 `WS_POPUP` 分层窗口（`src/platform/windows/grid_window.rs`）
* 移动通过 `WM_NCHITTEST → HTCAPTION` 实现 Windows 原生拖动（`src/platform/windows/wnd_proc.rs`）
* 缩放通过 `WM_NCHITTEST → HTLEFT/HTRIGHT/HTBOTTOM/HTTOP` 实现 Windows 原生缩放
* `WM_WINDOWPOSCHANGING` 已在 wnd_proc 中处理 Z-order 管理（`wnd_proc.rs:968`）
* 桌面覆盖窗口（overlay）是全屏透明分层窗口，覆盖主显示器工作区（`desktop_overlay/overlay_window.rs`）
* 渲染管线：Direct2D + DirectWrite → 32-bit DIB → `UpdateLayeredWindow`（`d2d_render.rs`, `overlay_render.rs`）
* 分类框位置/尺寸存储在 `LayoutConfig`（`BoxLayoutData`），持久化到 `~/.ikun_organizer.json`
* 现有 Timer 机制：`TIMER_ID_OVERLAY_DRAG`（0x4F56_0001）和 `TIMER_ID_GRID_DRAG`（0x4F56_0002）用于拖拽轮询
* `WM_ENTERSIZEMOVE` / `WM_EXITSIZEMOVE` / `WM_MOVING` / `WM_SIZING` 尚未在项目中使用
* 所有分类框通过 `GRID_WINDOWS: Mutex<Vec<GridWindow>>` 全局管理（`grid_window.rs`）

## 临时假设
* 对齐阈值默认 5 像素（物理像素），可在设置中调整
* 参考线颜色为半透明蓝色（类似 VS Code / Figma 对齐线风格）
* 参考线应覆盖全屏（多显示器场景下仅覆盖主显示器即可，后续可扩展）
* 对齐检测包含 6 种边缘对：top?top, top?bottom, bottom?top, bottom?bottom, left?left, left?right, right?left, right?right
* 同时支持移动和缩放两种操作场景的对齐检测
* 参考线宽度 1px，可选加 2px 阴影增加可见性

## 待解决问题
* 参考线是画在专用 overlay 窗口还是复用现有 overlay 窗口？（推荐专用窗口，职责更清晰）

## 需求（演进中）

### MVP（本次实现）
1. **边缘对齐检测**：移动/缩放分类框时，实时检测其 4 条边与其他分类框的 4 条边的对齐关系
2. **参考线显示**：当检测到对齐时，绘制一条横穿/纵穿屏幕的参考线，经过对齐的两条边
3. **参考线消失**：当边缘不再对齐时，参考线自动消失
4. **吸附功能**：当边缘接近对齐时，自动将分类框位置微调至精确对齐（snap）

### 未来扩展
* 居中参考线（中心线对齐）
* 等间距参考线
* 可自定义参考线颜色和粗细
* 多显示器支持

## 验收标准（演进中）
- [ ] 移动分类框 B，当其 top 边缘与分类框 A 的 top 边缘对齐时，显示水平参考线穿过两条 top 边
- [ ] 移动分类框 B，当其 left 边缘与分类框 C 的 right 边缘对齐时，显示垂直参考线穿过两条边
- [ ] 同时可以显示多条参考线（如 B 的 top 对齐 A 的 top，且 B 的 left 对齐 C 的 left）
- [ ] 松开鼠标后参考线消失
- [ ] 缩放分类框时也能触发对齐参考线
- [ ] 参考线不影响分类框的正常交互（点击、拖拽等）
- [ ] 吸附功能：当距离 ≤ 阈值时自动吸附到精确对齐位置
- [ ] 锁定状态的分类框不显示对齐参考线

## 完成定义（团队质量门槛）
* 添加单元测试（对齐检测算法）
* Lint / typecheck / CI 通过
* 行为变更时更新文档/笔记

## 明确排除范围
* 不包含分类框与桌面图标之间的对齐
* 不包含分类框与屏幕边缘的对齐
* 不包含居中参考线（中心线）
* 不包含等间距参考线
* 不包含用户自定义参考线样式（颜色/粗细）的配置 UI

## 技术方案

### 架构概览

```
移动/缩放分类框
    │
    ▼
WM_ENTERSIZEMOVE → 启动对齐检测模式
    │
    ▼
WM_MOVING / WM_SIZING / Timer 轮询
    │
    ▼
alignment_detect.rs：计算对齐关系
    │
    ▼
guide_overlay.rs：在专用透明窗口绘制参考线
    │
    ▼
WM_EXITSIZEMOVE → 清除参考线，关闭 guide overlay
```

### 新增模块

1. **`src/platform/windows/alignment_guide/`** — 对齐参考线模块
   - `alignment_detect.rs` — 对齐检测算法（纯逻辑，可单元测试）
   - `guide_overlay.rs` — 参考线 overlay 窗口（WS_POPUP + WS_EX_LAYERED + WS_EX_TRANSPARENT）
   - `mod.rs` — 模块导出

2. **修改 `src/platform/windows/wnd_proc.rs`**
   - 添加 `WM_ENTERSIZEMOVE` 处理：记录正在被移动的窗口 HWND，通知 guide overlay 开始
   - 添加 `WM_EXITSIZEMOVE` 处理：清除对齐状态，隐藏 guide overlay
   - 添加 `WM_MOVING` 处理：获取新位置 RECT，运行对齐检测，更新 guide overlay，可选 snap

3. **修改 `src/platform/windows/ffi_types.rs`**
   - 添加 `WM_ENTERSIZEMOVE`、`WM_EXITSIZEMOVE`、`WM_MOVING`、`WM_SIZING` 常量
   - 添加 `TIMER_ID_ALIGNMENT` 常量

### 对齐检测算法（`alignment_detect.rs`）

```rust
/// 对齐关系
pub struct AlignmentMatch {
    /// 对齐方向（水平/垂直）
    pub direction: AlignmentDirection,
    /// 对齐的坐标值（屏幕物理像素）
    pub position: i32,
    /// 匹配的参考框索引列表
    pub matched_box_indices: Vec<usize>,
}

pub enum AlignmentDirection {
    Horizontal, // 水平线（top/bottom 对齐）
    Vertical,   // 垂直线（left/right 对齐）
}

/// 检测移动框与其他框之间的对齐关系
///
/// `moving_rect`: 正在移动的框的屏幕 RECT
/// `other_rects`: 其他框的屏幕 RECT 列表
/// `threshold`: 吸附阈值（物理像素）
pub fn detect_alignments(
    moving_rect: &RECT,
    other_rects: &[(HWND, RECT)],
    threshold: i32,
) -> Vec<AlignmentMatch>
```

### 参考线渲染

- 专用 overlay 窗口：全屏覆盖主显示器工作区
- 窗口样式：`WS_POPUP | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST`
- 鼠标事件穿透（`WS_EX_TRANSPARENT`），不影响用户交互
- 渲染方式：D2D `DrawLine`，颜色 `rgba(0, 120, 255, 180)`，宽度 1px
- 参考线端点：从屏幕一端延伸到另一端

### 吸附逻辑

在 `WM_MOVING` 中：
1. 获取提议的新 RECT
2. 运行对齐检测
3. 如果检测到对齐，修改 RECT 使边缘精确对齐（snap）
4. 将修改后的 RECT 写回 lParam
5. 更新 guide overlay 显示参考线

## 技术笔记
* 检查的文档：`src/box_core/box_model.rs`（分类框数据模型）
* 检查的文档：`src/platform/windows/grid_window.rs`（格子窗口核心组件）
* 检查的文档：`src/platform/windows/wnd_proc.rs`（窗口消息处理）
* 检查的文档：`src/platform/windows/d2d_render.rs`（D2D 渲染）
* 检查的文档：`src/platform/windows/desktop_overlay/`（桌面覆盖窗口）
* 检查的文档：`src/config/app_config.rs`（应用配置）
* 参考模式：`overlay_render.rs` 中的 D2D 绘制方式可复用
* 参考模式：`drag_handler.rs` 中的 Timer 轮询机制可参考
* 对齐检测算法参考：Figma、PowerPoint 的 Smart Guides 实现
