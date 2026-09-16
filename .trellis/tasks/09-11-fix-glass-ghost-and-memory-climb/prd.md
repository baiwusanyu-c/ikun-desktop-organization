# 修复：分类框白顶残影 + 开启液态玻璃后内存持续爬升

## 症状
1. 每个分类框背后多一个「白色顶条 + 半透明框体 + 旧图标」的方块残影（用户截图红框）。
2. 开启液态玻璃后进程内存以约 100MB/分 爬升，数分钟到 500-700MB。

## 根因 1：DComp 旧视觉泄漏（残影）
`IDCompositionVisual` 一旦 `SetRoot` 上树，仅 drop COM 引用不会下树。
`destroy_all_grid_windows()` 直接 `DestroyWindow`，未先 `teardown_dcomp`：
凡走「销毁全部 + 重建」的流程（控制面板应用配置、AI 分类、清除恢复、配置重载等）
都会给每个框留一个冻结旧帧的视觉 = 白顶残影。

### 修复
- `destroy_all_grid_windows`：先收集句柄、释放锁，再逐个 `teardown_dcomp` + `DestroyWindow`。
- 新增 `teardown_dcomp_state(Box<DcompState>)`；`WM_DESTROY` 兜底摘树。
- `attach_dcomp` 中 `SetRoot` 成功而 `Commit` 失败时回滚（SetRoot(None) + 补 Commit）。

## 根因 2：饱和压测常量 bake 进代码（内存爬升）
工作区里 `GLASS_FORCE_RENDER_TICKS=1`、`GLASS_OCCLUDED_HEARTBEAT_TICKS=1`
（sat.py 暂改值），导致静止/遮挡桌面每 40ms 每框跑一整条渲染管线
（日志实证 heartbeat≈ticks，125 次/秒）。驱动用户态提交内存按渲染次数正比爬升
（约 1.2MB/分 @2.5 次/秒 → 百 MB/分 @125 次/秒）。

### 修复（40ms tick 基准）
- `GLASS_FORCE_RENDER_TICKS = 250`（10 秒心跳，与已提交注释的墙钟意图一致）。
- `GLASS_OCCLUDED_HEARTBEAT_TICKS = 125`（5 秒遮挡自愈）。
- `LIQUID_GLASS_RENDER_INTERVAL_MS = 40` 保留：25fps 下限的反应式触发依赖它；
  拖拽另有 `LIQUID_GLASS_DRAG_INTERVAL_MS = 16`（WM_ENTERSIZEMOVE 配对切换）。

## 验证计划（需解锁真机）
1. `IKUN_ALLOW_CAPTURE=1 IKUN_DEBUG_RECREATE=1 cargo run`：
   启动恢复后额外一轮「销毁全部+重建」，截屏确认无白顶残影。
2. 拖拽模拟 8 秒（SendInput），从 `[分类框渲染]` 日志核算拖拽帧率 ≥ 25fps。
3. 空闲 10 分钟 `[内存分解]` 时间序列：WS/GDI/句柄走平；心跳 ≈ 15 次/30 秒/5 框。
脚本：`%TEMP%\ikun_mem2\verify_goal.ps1`、`%TEMP%\ikun_mem2\log_series.py`。

## 测试
`cargo test --release`：610 通过 / 0 失败（含间隔断言更新与 alloc_probe 并发/折叠加固）。


## 2026-09-14 续：拖拽卡顿 / 内存台阶 / 锁屏陷阱 结论

### 拖拽卡顿（用户反馈"非常卡顿"）
- 根因 1：帧间回读滞后 1 帧 → 快速移动时玻璃背景"滑移"。修复：拖拽/模态移动期
  `sync_read=true`，同步阻塞读当前帧（零延迟）。
- 根因 2：ULW 提交面过大（padding 128 逻辑 → 8MB/帧）。修复：padding 128→48 逻辑
  （d09da5b1 参照帧 alpha 剖面证明 96 物理像素外 alpha≤1/255 不可见），提交面 -40%。
- 根因 3（历史）：拖拽档 16ms 定时器仅挂在 WM_ENTERSIZEMOVE；自定义拖拽未挂 → 已确认
  框移动走原生模态（HTCAPTION），16ms 生效。
- 测量设施：`mem_probe::record_drag_frame/flush_drag_stats` → 日志行 `[拖拽帧统计]`
  （帧数/平均/p95/max 间隔、>33ms 与 >50ms 掉帧数、单帧成本 avg/max），
  在 WM_EXITSIZEMOVE 与 end_drag 时 flush。待解锁后用 `flickdrag.ps1`
  （4ms 事件、~10px/ms、8 程 700-1000px 急甩）实测掉帧。

### 内存台阶（首次拖拽 +~95 GDI / +300 句柄 / +200MB WS，一次性）
- 已排除：guide overlay（已改拖拽结束 PostMessage 延迟销毁，验证销毁生效）；
  min/res 循环；壁纸 burst。
- 待查（需解锁）：1s 粒度 GDI 采样对齐日志事件；候选：图标重抽取旧 HICON 未 DestroyIcon、
  desktop_overlay 图标缓存替换。

## 2026-09-15 补充
- 「分类框中心不在任何输出」复现原因修正：非多-GPU 选择缺陷，而是
  非交互会话启动的实例无法创建任何桌面复制（outputs 为空）。
  结论：测量必须在交互会话启动应用；沙箱/服务会话启动的实例
  玻璃必然失效（DXGI duplication 需交互桌面）。
- 掉帧测量方案改为：用户交互会话启动 + 自然甩动，应用自记
  `[拖拽帧统计]` 日志为客观数据源，无需输入注入。

- 非无限增长：台阶后走平；空闲 11 分钟 151→154MB 走平实证。

### 锁屏陷阱（多次误判为"实例卡死"）
- 锁屏 → DXGI DuplicateOutput 拒绝访问(0x80070005) → 渲染/遥测全停；
  SendInput 无效；CopyFromScreen 失败。所有锁屏期间的"卡死/无日志"均为此因。
- 解锁监视：`%TEMP%\ikun_mem2\wait_unlock.ps1` 写 `unlocked.flag`。

### DComp 结论
- DPI 缩放修复（visual SetTransform2 96/dpi）后帧 dump 几何正确；但本机 Intel Arc 上
  DComp 着色 15-45ms/帧，差于 ULW 4-5ms → 默认 ULW，DComp 仅 `IKUN_DCOMP=1` 实验。

### 当前基线（5 框、玻璃开、192DPI）
- 可见态 ~311MB / 遮挡态 ~151MB；空闲走平；拖拽单帧成本 8-11ms（16ms 档内）。


## 2026-09-15 拖拽帧率最终结论（flick 急甩实测，11.5s 拖拽窗口）
| 配置 | 帧数 | 平均间隔 | p95 | 掉帧>33ms | 单帧成本 |
|---|---|---|---|---|---|
| 修复前(定时器饿死) | 37/8s | ~216ms | - | 全程 | 6.6ms |
| WM_MOVING 8ms 同步渲染 | 631 | 18.1ms | 54ms | 40 | 3.9ms |
| +同步读当前帧(错误) | 18 | 751ms | 1717ms | 18 | 165ms |
| +零等待读上一帧 | 627 | 19.0ms | 58ms | 52 | 4.0ms |
| +参考线 bbox+33Hz | 115 | 107.8ms | 394ms | 68 | 16.7ms |
| **+参考线 bbox+10Hz(最终)** | **704** | **17.2ms(58fps)** | **31ms** | **19** | **2.33ms** |
| 无参考线对照 | 828 | 14.0ms | 28ms | 12 | 2.64ms |

结论：拖拽玻璃背景 58fps（下限 25 达标，余量 2.3x）；对齐参考线保留（10Hz 更新、
包围盒细条窗口，ULW 面 42MB→~100KB）；内存空闲走平、拖拽瞬时峰值回落、无累积。

## 2026-09-15 高速拖拽补强
- 合成 5000px/s 急甩暴露 Windows 模态移动循环在 125Hz 消息风暴下饿死渲染（3fps）。
- 修复：标题拖拽改自定义 SetCapture 移动（WM_NCLBUTTONDOWN(HTCAPTION) 拦截 +
  WM_MOUSEMOVE 直接 SetWindowPos + 8ms 节流渲染 + WM_LBUTTONUP 收尾），
  不经过模态循环；锁定/更多按钮（HTCLIENT）与边缘缩放（HTEDGE 模态）不受影响。
- 模态移动期间 WM_MOUSEMOVE 跳过 hover/tooltip/阈值逻辑。
- 测试 610 全绿；5000px/s 复测待解锁。
