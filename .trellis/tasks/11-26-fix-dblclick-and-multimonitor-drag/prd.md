# 修复：分类框图标双击偶发无反应 + 多屏拖拽显示异常/漂移出屏

## 目标

1. 分类框内**双击图标打开文件**必须稳定生效（当前偶发无反应）。
2. 分类框在主/副屏之间拖拽时不再出现显示异常、不再漂移出屏幕：
   允许跨屏切换，但盒子始终被约束在显示器范围内。

## 背景与根因（代码走查结论）

### Bug 1：双击图标偶发无反应

- `WM_LBUTTONDOWN` 命中图标后置 `drag_pending=true`（`wnd_proc.rs`）。
- 随后**任何一次** `WM_MOUSEMOVE` 只要位移 > `DRAG_THRESHOLD = 5`
  （`ffi_types.rs:721`，硬编码、不随 DPI 缩放）就置 `drag_active=true`、
  创建残影并起 8ms 定时器 `TIMER_ID_GRID_DRAG`。
- `handle_grid_drag_timer`（`drag_handler.rs:446+`）里
  `find_external_window_at_point` 只看「光标屏幕坐标是否落在某个
  `CabinetWClass` 窗口矩形内」，**不判断光标是否离开了本分类框**。
  分类框常驻 `HWND_BOTTOM`，资源管理器窗口几何上覆盖它时，
  单击图标 + 手抖 6px → 立即进入阻塞式 `DoDragDrop`，
  并把本框置为 `WS_EX_TRANSPARENT`（对鼠标透明）。
  → 双击的第二次按下/`WM_LBUTTONDBLCLK` 被拖拽模态循环吃掉，表现为「没反应」，
  同时还会误发起一次「复制文件到资源管理器」的危险操作。
- 引擎（液态玻璃）路径下：库画布 `hit_test` 命中块才会派发
  `PointerPhase::DoubleClick`，块被钳到「中心不出本屏」而代理窗口位置未同步时，
  块与代理窗错位 → 双击坐标落在块外 → 整条双击链路静默丢失。

### Bug 2：多屏拖拽显示异常 + 漂移出屏

- `monitor_utils::get_virtual_desktop_bounds()` 存在但**全项目零调用**，
  分类框拖拽完全没有任何边界约束：可拖到屏外并停在那里（重启后 `collect_grid_layout`
  还会把这个非法位置落盘，下次启动继续看不见）。
- 玻璃块是**单屏模型**（liquid-verre-rs `core/block.rs::rehome_block`：块只归属
  中心所在的那块屏画布，并把中心钳到该屏内，「不做跨屏大玻璃」）。
  代理窗口却可以被宿主/库会话放到跨屏位置，两侧一旦不一致：
  - 内容窗口与玻璃块错位 → 一半主屏一半副屏时「显示异常」；
  - 库按中心钳制块位置，宿主却按未钳制的矩形摆窗 → 观感上「漂移出屏幕」。
- DPI 跨屏换算口径：`phys_to_logical_rect` / `logical_to_phys_rect` 约定
  x/y 为虚拟屏物理像素、w/h 为逻辑（主屏物理）像素，宿主复用点必须保持同一口径。

## 需求

### R1 拖拽阈值 DPI 化 + 起拖位置判据（Bug 1）

- 新增纯函数：按窗口 DPI 计算拖拽启动阈值（物理像素），
  以系统 `SM_CXDRAG/SM_CYDRAG` 为基准并给出下限兜底，替换硬编码 `DRAG_THRESHOLD` 比较。
- `handle_grid_drag_timer` 中「检测到外部窗口即启动 OLE 拖拽」增加判据：
  **光标必须已离开本分类框内容矩形**才允许起 OLE；未离开时保持自定义残影跟随，
  不阻塞消息循环、不置 `WS_EX_TRANSPARENT`。
- 上述判据与阈值计算都写成可单测的纯函数（输入坐标/矩形/DPI，输出 bool/阈值）。

### R2 双击优先于拖拽（Bug 1）

- `WM_LBUTTONDBLCLK` 进入时，若本框存在进行中的自定义拖拽
  （`drag_active` 或 `drag_pending`），必须先干净收尾（杀定时器、销毁残影、
  复位拖拽字段），再执行打开文件，避免拖拽吞掉双击。

### R3 拖拽边界约束（Bug 2）

- 新增纯逻辑模块（`box_core` 层，0 unsafe、可单测）：
  - `clamp_rect_to_display_bounds(rect, bounds)`：把矩形整体夹进外接范围，只改位置不改尺寸；
  - `monitor_for_rect_center(rect, monitors)`：按中心点选择归属显示器；
  - `single_monitor_clamp(rect, monitors, virtual_bounds)`：
    优先整框留在归属屏内；屏容不下该尺寸时退回「夹在虚拟桌面内」。
- 落点（宿主驱动的几何推进必须先钳制再使用/下发）：
  - `WM_MOVING`（回退模式模态拖拽 + 宿主 SetWindowPos 链路）；
  - `WM_WINDOWPOSCHANGED` 中宿主反推下发块几何之前；
  - `push_geometry_for_window` / `push_box_rect` 入口；
  - 新建窗口与启动恢复布局（`create_grid_window` / 布局恢复）非法位置兜底；
  - `follow_glass_session` 镜像库会话矩形时，先夹虚拟桌面边界（防真出屏），
    仍保持「库为几何真源」，不与库会话锚点抢夺。
- 约束语义：**不得把框夹出屏幕**（含任务栏外沿的完整显示器矩形），
  同时保持可在主屏/副屏之间自由拖拽切换。

### R4 跨屏显示异常的宿主侧兜底（Bug 2）

- 玻璃会话结束后，若库权威块矩形与内容窗口尺寸/DPI 换算不一致
  （跨屏 DPI 重采样导致块物理尺寸变化），必须触发一次全量重绘与内容重推，
  避免纹理按旧尺寸被拉伸/错位。
- 不重复实现库的遮挡/层序判据（见 `tauri-window-contract.md` 禁令）。

## 验收标准

- [ ] 100% / 150% / 200% DPI 下单击图标后轻微手抖（< 系统拖拽阈值）不再误起 OLE 拖拽。
- [ ] 资源管理器窗口打开并几何覆盖分类框时，双击图标仍能稳定打开文件。
- [ ] 玻璃模式与非玻璃（回退分层窗）模式两条路径双击图标均可打开文件。
- [ ] 分类框无法被拖出虚拟桌面范围（含负坐标副屏），松手后仍完整可见。
- [ ] 分类框可在主屏与副屏之间正常拖拽切换，切换后显示正常、可继续交互。
- [ ] 跨界停顿时内容窗口与玻璃块不错位（不再一半主屏一半副屏的错位残影）。
- [ ] `cargo fmt` / `cargo clippy` / `cargo test` 全绿；前端无改动。
- [ ] 规范文档补充多屏边界与拖拽阈值口径。

## 技术笔记 / 约束

- 单文件 ≤1500 行（`wnd_proc.rs` 已 1814 行，几何纯逻辑必须放 `box_core`，
  不要在 `wnd_proc.rs` 内堆判据）。
- 坐标口径：所有拖拽几何为虚拟屏物理像素；`phys_to_logical_rect` 的 x/y 物理、
  w/h 逻辑约定不得改动。
- 平台层禁止引用 `tauri_commands`；配置写回只能走 `config_writeback::write_platform_config`。
- 不使用 `unwrap/expect` 处理可恢复错误；日志中文。
- 不做 git commit（等用户明确指令）。
