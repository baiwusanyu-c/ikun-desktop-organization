# Tauri 窗口与原生分类框契约

## 适用范围

本契约覆盖 `frontend/src/lib/api/tauri-api.ts`、`frontend/src/lib/window-lifecycle.ts`、
`src/tauri_commands/config_cmd.rs`、`src/tauri_commands/box_cmd.rs` 与
`src/tauri_commands/platform_runtime.rs` 与 `src/tauri_commands/window_cmd.rs`。WebView 窗口只负责 UI，分类框仍由 Win32/D2D 创建和渲染。

## 共享状态契约

`main.rs` 在 Tauri `setup` 中创建一个 `SharedTauriRuntime = Arc<Mutex<TauriRuntime>>`，先通过 `app.manage(...)` 注入 Commands，再把同一个实例交给 `start_platform_runtime`。`TauriRuntime` 持有唯一的 `AppContext`。

- `config_cmd.rs` 和 `box_cmd.rs` 必须使用 `State<'_, SharedTauriRuntime>` 读写配置。
- 成功保存配置后必须调用 `TauriRuntime::replace_config`，同步 `ctx.config` 与依赖分类规则创建的 `ctx.classifier`。
- 禁止在普通更新命令中重新 `AppConfig::load()` 并维护第二份状态；`load_config` 是用户显式要求从磁盘重载时的唯一例外。
- 互斥锁中毒返回中文错误，不得 `unwrap()` 导致 Tauri 事件循环崩溃。
- **平台线程写配置的唯一入口**：`platform/windows/config_writeback.rs::write_platform_config`。
  拖拽收尾、覆盖层图标位置、菜单删除分类框等场景过去直接 `AppConfig::load()` → 改字段 → `save()`，
  绕开内存真源；`config_cmd` 随后从陈旧内存整文件回写就会冲掉平台新写入的字段（2026-11-04 自检审计 P0：
  布局丢失、被删框复活）。钩子由 `start_platform_runtime` 启动时注入（依赖倒置，平台层不得引用
  tauri_commands）；钩子未注入的早期路径退回「读磁盘最新快照后改写」，**加载失败必须跳过写入，
  绝不允许用默认配置覆盖用户配置**。AI 分类重建分类框后必须同锁把 `collect_grid_layout()` 回写
  内存 + 落盘。

## IPC 契约

| 命令 | 参数 | 成功结果 | 失败行为 |
| --- | --- | --- | --- |
| `get_config` | 无 | `AppConfig` 对象 | 返回中文错误字符串，页面显示加载错误 |
| `update_theme_color` | `{ color: string }` | 最新 `AppConfig` | 不写入不完整颜色 |
| `update_box_style` | `{ style: BoxStyleConfig }` | 最新 `AppConfig` | Rust `normalize` 后持久化并重绘所有分类框 |
| `update_liquid_glass` | `{ config: LiquidGlassConfig }`（嵌套：liquid 37 项 / transparent / resize / engine） | 最新 `AppConfig` | 归一化后实时下发玻璃引擎样式；引擎级参数变化触发引擎自动重建 |
| `update_ai_config` | `{ config: AiConfig }` | 最新 `AppConfig` | 保留其它配置字段，仅更新 AI 部分 |
| `create_box` | `{ name: string, description: string }` | `BoxData` | 空名、超过 16 字符、同名或原生创建失败时返回错误 |
| `edit_box` | `{ id: string, name: string, description: string }` | `BoxData` | 无效 UUID、空名、同名或不存在时返回错误 |
| `get_pending_edit_box` | 无 | `EditBoxPayload` 或 `null` | 无 pending 数据时返回 `null` |
| `show_window` / `hide_window` / `close_window` | `{ label: string }` | `()` | 仅接受三个固定标签；`close_window` 对兼容调用也执行 hide |
| `open_url` | `{ url: string }` | `()` | 仅允许带 host 的 HTTP/HTTPS URL，由 `tauri-plugin-opener` 打开 |

`capabilities/default.json` 只授予 `opener:allow-open-url`，不得恢复通用 shell 执行或文件打开权限。

`EditBoxPayload` 的字段固定为 `{ config_id: string, current_name: string }`。
双击原生分类框标题后，Rust 先写入 pending，再发送 `edit-box-opened` 事件；编辑页先注册监听，再读取 pending，以覆盖首次事件竞态。

## 窗口居中契约

所有 Tauri 绘制的弹窗（`control-panel`、`create-box`、`edit-box`、`ai-classify`）
默认位置必须是**主屏幕水平垂直居中**：

- `tauri.conf.json` 中每个窗口设置 `center: true`，保证创建时即居中。
- `window_cmd.rs::center_on_primary_monitor` 使用 `app.primary_monitor()` 的
  position + size 与 `window.outer_size()`（均为物理像素）计算居中坐标。
- `show_managed_window` 在每次 `show()` 前调用居中，保证“默认位置”契约；
  居中失败只记 `warn!`，不阻断显示。

## 窗口生命周期

`control-panel`、`create-box`、`edit-box` 的关闭都必须是 `prevent_close + hide`，不能调用 `close()`。
Rust `on_window_event` 和前端 `installHideOnClose` 双重拦截；托盘/信号再次打开时使用同一个 WebView 实例并 `show + set_focus`。

真正退出应用必须设置共享运行时的 `quit_requested`，由平台轮询线程保存分类框布局、清理注册表/托盘、销毁原生分类框并恢复桌面图标后再执行 `app.exit(0)`。Tauri Command 不得直接退出进程而绕过这条清理路径。
所有 Rust 入口必须复用 `window_cmd::{show_managed_window, hide_managed_window, toggle_managed_window}`，不得在轮询线程复制标签和窗口操作。

## Svelte 配置状态

`ControlPanel.svelte` 将 `AppConfig` 存入 Svelte 5 `$state` 后，子组件收到的是响应式代理。
`StyleSettings.svelte` 与 `AiSettings.svelte` 必须使用 `$state.snapshot(...)` 生成传给 Tauri Command 的普通对象；
不得对 `$state` 代理直接调用 `structuredClone`，否则 WebView 会抛出 `DataCloneError`，页面因初始化 effect 中断而显示为空。

主题色更新还必须通过 `theme-color-changed` 事件广播到持久的 `create-box` 和 `edit-box` WebView；
两个窗口重新收到 `window-opened` 时应再次调用 `get_config`，作为事件丢失或外部配置变更时的兜底。

## 液态玻璃引擎帧（liquid-verre-rs）

液态玻璃由 `liquid-verre-rs` 引擎（`src/platform/windows/glass_engine.rs` 适配层）在独立渲染线程持续采集/合成；平台层不得自行启动第二个采集器或把玻璃像素画回内容 DIB。分类框内容窗口只是玻璃之上的前景层。

### 玻璃块层序与缩放反馈 border（库 >= 0.5.2）

- 库在逐块 pass 内发射 border：第 i 块的反馈白边紧跟它自己的玻璃，**早于任何更高块**（库 `core::draw::border_emit_plan`）。
- 因此两个分类框重叠时，**下层框的 border 会被上层框的玻璃正确遮挡**，不再画到上层框身上。平台层不得为了「让正在操作的框白边永远可见」而额外抬升玻璃块：0.5.1 时代 `begin_glass_resize_session` 里的 `raise_box` 绕法实测修不了该缺陷，已退役。
- 块抬前只允许由「激活/操作中的框整体在最前」这条产品语义触发，链路是 `wnd_proc` 的 Z 序不变量 → `glass_engine::raise_box`。

**命中层序与绘制层序共用同一套 z 判据（库 >= 0.5.3，#9 / #10；库内 BASELINE 不变量 13）**

- `hit_resize` 带「更上层块本体遮挡」过滤（判据 `topmost_body_z_at` + `grip_reachable`，本体矩形口径与
  `hit_test` 逐字一致）。四个调用点（每帧 hover、`WM_NCHITTEST`、`dispatch_pointer`、`forward_move_to_sink`）
  共用这一处判据 ⇒ **被上层框机身压住的那段边缘，归属整个上移到上层框**：下层框不再亮白边、不再变
  缩放手型，`WM_NCHITTEST` 也不再在属于上层框的像素上声称命中。
- 外来窗口（Chrome/IDE 等）压在光标上时，库用 `WindowFromPoint` 门控清空 hover / 手型。
  **该门控只在「光标移动过」的帧查询** ⇒ 窗口盖上来要动一下鼠标才灭边，撤走也要动一下鼠标才恢复。
  **复验时不要拿静止鼠标当反例。**
- 归属变化的连带效果：依赖「hover 落在哪块」的 UI 现在读到的是**正确的上层块**。平台层目前只有
  `debug_log_blocks()` 读 `WindowSnapshot::hovered`（纯诊断）；业务 hover（图标高亮/按钮态）仍由宿主自己的
  `WM_MOUSEMOVE` 命中判定负责，两套判据互不覆盖，不要在宿主侧重算块层序。
- **禁止**两种旧绕法：为消亮边而 `raise_box` 抬层（用篡改叠放掩盖命中判错，鼠标移回去还得改回来）、
  `set_resizable(false)`（会连该边缘**未被遮挡**的那段一起关掉，且与真实缩放会话抢状态）。
  **同样禁止**宿主自己实现一份遮挡/层序判据 —— 块 z 序与本体矩形只有库掌握，重算必然与库漂移。
- `WindowSnapshot::hovered` 自 0.5.3 起进快照比对键，诊断日志的 `hovered=` 不再陈旧；但它仍是快照
  （最坏滞后 1 帧），不得当实时信号驱动状态机。

### 画布 Z 序兜底（库 >= 0.5.2）

- 库把画布的**显示与层级合进同一次** `SetWindowPos`（`show_request`），并且 `Engine::set_window_level` 等值调用也会重发层级，所以平台层不再自己拿 `canvas_handle` 发 `HWND_BOTTOM`，也不再跑 4ms 高频压底守护线程。
- 关键时机（引擎启动 / 引擎重建 / `Ready` / `MonitorsChanged` / 新建块显示后）调用 `glass_engine::press_canvas_bottom()`；其余位置（runtime 100ms tick、内容窗口被推底之后）调用判据 + 120ms 防抖的 `glass_engine::reassert_canvas_bottom()`。
- 复验判据不变：启动 12s 内画布低于所有其他应用窗口的采样占比应为 100%；若出现「控制面板整页空白」，先查这条链路。

### 引擎重建/复活的最低恢复清单（2026-11-04 自检审计 P1）

引擎重建（DPI 热插拔、引擎级配置变化、自动复活）会让库侧块句柄全部作废。
`spawn_holder_and_reattach` 成功后必须广播 `WM_USER_GLASS_REBUILT`（`grid_window::notify_glass_rebuilt_all()`），
每个分类框在**自己线程**完成三件事，缺一不可：

1. 重新发布命中几何 `update_block_geom`（ROUTES 对新块初始为 `geom: None`，
   漏发则标题拖拽/按钮手型判定退化）；
2. 置 `content_valid = false` 并触发一次全量重绘（库纹理已空，漏发则玻璃块只剩底板）；
3. DPI 换算使用重建元组里的 `old_ratio`（不得回读已清空的全局 HOLDER，历史上因此换算从未生效）。

其余强制口径：缩放最小下限的托底夹取只有 `clamped_resize_options` 一个实现，
`block_config_for`/`apply_settings`/`set_box_locked` 三个入口共用（漏一处 = 锁定再解锁就把框钉死）；
重建入口有 `REBUILDING` 单飞；死亡判定必须连续两拍 `is_running()==false`（防误杀刚启动的引擎）。

### 指针事件与快照契约（库 >= 0.5.2）

- **sink 合成消息的坐标空间**：`glass_engine` 输入路由投递给代理窗口的
  `WM_MOUSEMOVE/LBUTTONDOWN/LBUTTONUP/LBUTTONDBLCLK/RBUTTONUP` 的 lparam 是
  **块（= 内容矩形）局部坐标**，不是代理窗 client 坐标。消费端一律经
  `wnd_proc::message_content_point()` 统一换算（引擎模式直取、回退模式走
  `ClientToScreen` 链路）；任何新 handler 直接对合成消息用
  `content_point_from_lparam` 都会在 200% DPI 下偏移一个不可见边框（实测 12px，
  2026-11-04 自检审计 P1：旧修复只覆盖了 marquee 一条路径）。

- 双击：库画布类已带 `CS_DBLCLKS`，物理双击到达 sink 的序列是 `Down -> Up -> DoubleClick -> Up`，**第二次按下不再产生 Down**。sink 侧禁止再自行按 `GetDoubleClickTime` 合成双击（会与真相位重复投递）。
- 快照：块几何变化 ⇒ **同帧**快照可见，会话位 `moving`/`resizing` 也会如实回落。跟随内容窗口时直接读 `GlassWindow::snapshot()`，不得再用 `set_default_resize` 之类「催发布」hack。
DXGI 初始化阶段可能返回平均 RGB 接近 `(1,1,1)` 的不透明占位黑帧；渲染器最多跳过前三帧，并由
`src/platform/windows/d2d_render.rs` 使用白色半透明回退，取得真实桌面帧后立即缓存。单帧捕获失败继续使用上一成功帧。

## 多屏拖拽与几何边界契约（2026-11-26）

液态玻璃块是**单屏模型**（库 `core::block::rehome_block`：块只归属中心所在屏的画布，
中心钳到该屏内，明确「不做跨屏大玻璃」；每屏一张整屏画布并 `SetWindowRgn` 裁剪）。
因此「分类框停在跨屏位置」时，玻璃只渲染落在归属屏内的那一半，而代理窗口横跨两块屏
—— 用户看到的「一半主屏一半副屏显示异常」就是这两者不一致，不是渲染 bug。

宿主的边界职责（口径唯一，改前先看 `.trellis/spec/backend/windows-platform-guidelines.md`
的「多显示器拖拽边界与双击保护」）：

- 纯算术判据全部下沉 `src/box_core/monitor_geom.rs`（0 unsafe、可单测），
  平台层只做 `RECT` ↔ `[l,t,r,b]` 薄转换；坐标口径恒为**虚拟桌面物理像素**
  （副屏可能在负坐标区），`phys_to_logical_rect` 的「x/y 物理、w/h 逻辑」约定不得改动。
- **宿主驱动的几何推进必须单屏吸附**（整框留在中心所属屏内）：
  建窗 / 恢复布局 / `WM_MOVING` / `WM_WINDOWPOSCHANGED` 反推下发 / 布局落盘前。
  注意引擎模式下标题拖拽由本窗口 `SetWindowPos` 驱动，**`WM_MOVING` 不会触发**，
  只在 `WM_MOVING` 加边界等于没加。
- **库会话（移动/缩放）进行中不得反向拽块**：块矩形是几何真源，跟随侧
  （`follow_glass_session` / `end_glass_resize_session`）只允许「夹虚拟桌面边界」
  这一层兜底，以及会话结束后做一次「块与代理窗同屏」一致性纠偏
  （不一致时按块的中心吸附窗口并重发块矩形）。
- 回读块矩形一律用 `snapshot.scale`（块自己的权威 DPI 比例），
  不得依赖宿主 `entry.ratio`（跨屏那一拍代理窗 DPI 常是旧屏值）；
  检测到比例漂移要 `sync_block_ratio` + 作废内容层缓存强制全量重绘。
- `collect_grid_layout` 可能被 Tauri 命令线程调用：该路径**禁止**跨线程
  `SetWindowPos` / 渲染，只吸附要写入配置的坐标值。

与双击的耦合（同一批缺陷）：

- 吸附后代理窗口与块恒等对齐，否则 `WM_LBUTTONDBLCLK` / `WM_LBUTTONDOWN`
  的命中坐标落在块外，双击链路静默失效。
- 图标拖拽阈值必须按 `GetDpiForWindow` 折算（`drag_logic::drag_threshold_physical_px`），
  且「光标已离开本框内容矩形」才允许升级为阻塞式 `DoDragDrop`
  （`DoDragDrop` 期间本框被置 `WS_EX_TRANSPARENT`，会吞掉双击的第二次按下）；
  `WM_LBUTTONDBLCLK` 进入时必须先收尾进行中的自定义拖拽
  （杀 `TIMER_ID_GRID_DRAG` + 销毁残影 + 复位 `drag_*` 字段）。
- 借纪律：`WM_WINDOWPOSCHANGED` / `follow_glass_session` / `end_glass_resize_session`
  都必须「读值 → 结束 `get_data` 借用 → 再调 `SetWindowPos`」；`SetWindowPos` 会同步重入
  窗口过程，持引用调用等于造出两个 `&mut GridWindowData`（UB）。
- 任何回推块几何的入口（含 `WM_MOVING`）都必须判
  `!glass_session_active && !glass_engine::block_in_session(id)`。
- 仍然禁止：宿主重算块的遮挡/层序判据、在 sink 里自行合成双击、
  用 `set_default_resize` 之类「催发布」hack。

### 缩放会话量纲契约（库 >= 0.5.5，2026-11-26 实装）

0.5.5 起库把缩放会话**全程改在物理域**计算，宿主必须按下面的成对契约理解块矩形：

- 会话原点 = **按下时刻的物理矩形**（不再存「按下 scale 下的逻辑尺寸」），
  因此会话中途跨屏（主 192 DPI / 副 96 DPI，scale 1.0 ⇄ 0.5）**物理尺寸逐帧不变**，
  「半主半副过渡跳尺寸」与「锚定边漂移」在库侧根修；
- `ResizeState::apply` 返回**逻辑**尺寸（内部 `/ scale`），写回端 `b.width = r[2] * scale`
  乘**同一个** scale —— 乘除相消。宿主看不到这一步，但**不得**再自己按窗口 DPI
  二次折算 `ResizeOptions.min_width/min_height`：它们仍是**逻辑像素**下限
  （库内部 `× scale` 进物理域再垫 `MIN_SIZE`），本仓 `glass_resize_floor` 因此固定返回
  常量逻辑值 200 x 140；
- 会话期间块的 `x/y`（物理）与 `w/h`（逻辑）由库独占写入，宿主只做
  `clamp_content_to_desktop` 与收尾单屏归位，**禁止**回推校正（见
  `spec/backend/windows-platform-guidelines.md`「副屏缩放锚定边漂移」一节）。

判别法：拖左缘时块右缘必须**纹丝不动**，即 `x + w × snapshot.scale` 守恒；
不守恒 = 库里量纲又漂了，去库侧修，不要在宿主加补偿。

## 验证用例

- Good：修改圆角/背景色/玻璃开关后，`update_box_style` 或 `update_liquid_glass` 返回成功，现有分类框立即重绘。
- Good：关闭任意 WebView 后再次通过托盘或信号打开，窗口仍可见且编辑页数据正确。
- Good：UI 更新配置后，下一次托盘 AI 分类和原生恢复布局读取同一个 `AppContext` 中的新值。
- Base：DXGI `WAIT_TIMEOUT`、`ACCESS_LOST` 或单帧 D2D 错误只保留上一成功帧，不进入黑色回退；显示器 DPI 变化会使快照失效并重新采集。
- Base：Svelte 配置使用 `$state.snapshot` 后再跨 IPC；初始化 effect 不得抛出 `DataCloneError`。
- Bad：DXGI 占位黑帧不得进入成功帧缓存，也不得作为分类框首次可见画面提交。
- Bad：缺失命令参数、非法名称或不存在的分类框不得静默成功，前端必须显示错误状态。
- Bad：未知窗口标签、`file:`/`javascript:` URL 和锁中毒不得触发窗口或进程级副作用。
- Good：分类框可在主屏与副屏之间拖拽切换；松手后整框完整落在其中一块屏内，
  不会停在「一半主屏一半副屏」，也不会漂出虚拟桌面（含负坐标副屏与拔掉副屏后的旧位置）。
- Bad：资源管理器窗口几何上覆盖分类框时，双击图标仍必须打开文件
  （不得因手抖几像素就升级为阻塞式 OLE 拖拽而吞掉双击）。
- Good：副屏（scale != 1）按住左边缘向内拖，块右缘必须钉住不动
  （`x + w × scale` 守恒，库 >= 0.5.5）；主副屏来回拖不再跳尺寸。
- Good：把分类框拖窄 -> 图标自动换到下一行且始终落在框内；拖宽 -> 自动换回上一行；
  主屏 <-> 副屏之间切换时列数不变，因此图标位置不变（需求 1 + 2.1）。
- Good：内容高度超出可视区时右侧出现滚动条，滚轮按行上下查看，滚到底时最后一行完整可见；
  装得下时滚动条消失且滚轮不动（需求 2.2）。

## 必需检查

```text
npm run check
npm run build
cargo check
cargo test
```
