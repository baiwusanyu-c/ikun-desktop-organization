# Journal - codex-agent (Part 1)

> AI development session journal
> Started: 2026-10-23

---



## Session 1: 使用 liquid-verre-rs 重构分类框渲染（后端+前端+冒烟验证）

**Date**: 2026-10-23
**Task**: 使用 liquid-verre-rs 重构分类框渲染（后端+前端+冒烟验证）
**Branch**: `master`

### Summary

(Add summary)

### Main Changes

## 会话：使用 liquid-verre-rs 重构分类框渲染（任务 10-23-refactor-box-render-with-liquid-verre-rs）

### 完成内容
- 分类框渲染基座从自研 `liquid_glass.rs`（DDA + D2D Effect Graph + 自研 HLSL）整体迁移到 crates.io `liquid-verre-rs = 0.4.2`：
  - 新增适配层 `src/platform/windows/glass_engine.rs`（引擎生命周期/注册表/坐标换算/事件转中文日志/Z 序重压底/样式缓存/把手带查询）；
  - 删除 `liquid_glass.rs`、`occlusion.rs`（遮挡剔除交由库内实现，layered 窗口天然不触发误剔除）。
- 交互分工（决策记录见 PRD）：玻璃底板+边缘缩放会话+悬停加粗 border 反馈=库画布（AlwaysOnBottom）；标题拖拽=原生移动 + WM_MOVING 同步库块；内容层「命中捕获层」（alpha=1 + 把手带透明穿透）；锁定联动 set_interactive。
- 配置链路：`LiquidGlassConfig` 重写为库 API 镜像（liquid 37 项 / transparent / resize+feedback / engine），serde snake_case + 向后兼容旧 JSON + clamp_finite 语义 normalize。
- 前端：`tauri-api.ts` 新嵌套类型；`StyleSettings.svelte` 新增「玻璃效果（liquid-verre-rs）」11 组 61 控件；前后端字段程序化核对零偏差。
- 日志：库 log facade 直入项目 env_logger 滚动文件；EngineEvent 全量映射中文日志；mem_probe `[内存分解]` 改报引擎指标（FRAME_TIMINGS 改全局锁）。
- 规格同步：spec/index、directory-structure、quality-guidelines、tauri-window-contract、windows-layered-window-guide。

### 关键坑（供后来者）
1. `Engine::spawn` 后立即 `set_event_handler` 会三方锁死（主线程持 HOLDER 等 handler 写锁 × 渲染线程持 handler 读锁等 HOLDER）；必须 `Engine::new → set_event_handler → start()`，且回调只准碰独立轻登记表（glass_id→hwnd），禁止锁 HOLDER。
2. `f32::clamp` 对 NaN 透传，配置 normalize 必须用 clamp_finite 语义。
3. cargo 增量指纹可能假通过（改动不重编），验证前用 `cargo clean -p` 或看 warnings 计数变化。
4. `WM_WINDOWPOSCHANGED` 常量若未在 ffi_types 定义，match 分支会退化成通配绑定吞消息。
5. 库坐标系：块 x/y=虚拟屏物理像素，w/h=逻辑像素（×所在屏 DPI 比例=物理）；块中心被钳制在本屏内（分类框最多半出屏）。

### 验证
- cargo fmt/clippy/test：512 通过；warning 51 < 基线 54。
- pnpm check：0 错误；build 通过。
- debug 实例冒烟：双屏 DDA 引擎就绪、玻璃块=2、实测完整缩放会话（ResizeStarted→跟随+参考线→ResizeEnded→save_config）、独占全屏让路/遮挡恢复日志正常、静止 CPU 低。
- 待办：用户实机验收视觉与全交互；验收通过后归档任务（顺带关闭被本重构取代的 09-11-fix-glass-ghost-and-memory-climb）。


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 2: 架构修订：分类框窗口本体改为库窗口（记录库能力差距清单）

**Date**: 2026-10-23
**Task**: 架构修订：分类框窗口本体改为库窗口（记录库能力差距清单）
**Branch**: `master`

### Summary

(Add summary)

### Main Changes

## 会话 2：架构方向修订（推翻双层叠放模型）

用户实机验收判定「库玻璃做底板 + 宿主内容窗口叠放」模型错误（边缘拖拽/框选/图标命中互相冲突、交互大量丧失）。
正确目标：分类框窗口本体 = liquid-verre-rs 的 GlassWindow（库画布块），内容绘制与输入交互全部移植进库窗口。
库作者（同项目作者）将修改 liquid-verre-rs 补齐能力。

- PRD 已记录架构修订 + 库能力差距清单（P0：每块内容层 update_content、输入路由 input_sink；
  P1：拖拽接管、焦点窗口、drop 路由确认；P2：move_filter 吸附）。
- 已实施变更的处置：配置镜像 / glass_engine 骨架 / 事件转日志 / Z 序 / 前端面板 全部保留；
  「命中捕获层 + 透明把手带穿透 + 宿主窗口跟随」交互 hack 待库 API 就绪后删除重写。
- 阻塞项：等库 P0-1 / P0-2 发布版本后继续项目侧移植（窗口退役 → GlassBlockView 事件驱动）。


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 3: sink 首版缺陷修复：消息洪水+模态竞态 → 接管式拖拽（实测验证）

**Date**: 2026-10-26
**Task**: sink 首版缺陷修复：消息洪水+模态竞态 → 接管式拖拽（实测验证）
**Branch**: `master`

### Summary

(Add summary)

### Main Changes

## 会话 3：sink 首版缺陷修复（消息洪水 + 模态竞态 → 接管式拖拽）

用户实机验收报 5 症状（双击/按钮/拖拽卡死/hover 闪烁/无法拖拽）。根因定位：
1. 逐条转发库 120Hz WM_MOUSEMOVE（posted 不合并）压垮宿主队列 → 按钮失灵 + hover 闪烁；
2. 合成 WM_NCLBUTTONDOWN 借系统模态移动，posted up 竞态丢失 → 主线程挂死。
liquid-verre-rs 无缺陷（六规则/FocusHint/clamp 行为全部与文档一致）。

修复：sink 只转发低频事件；标题拖拽改「接管式」（SetCapture + 16ms 光标轮询 +
WM_WINDOWPOSCHANGED 反向同步 + end 回读库权威矩形对齐 clamp）；hover 100ms 轮询
SendMessage 复用既有 handler；定时器三档（hover/会话）。

验证：SendInput+定向 PostMessage 实测锁定切换/双击编辑/拖拽+clamp/resize 状态机/
滚轮全链路日志证据；514 单测绿、51 warnings<54 基线。测试后已恢复用户布局数据并
重启实例。待用户视觉确认 hover/border 反馈/参考线。


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 4: 分类框全灭根因修复：锁定=输入全灭 + 库 debug panic 杀渲染线程

**Date**: 2026-10-27
**Task**: 分类框全灭根因修复：锁定=输入全灭 + 库 debug panic 杀渲染线程
**Branch**: `master`

### Summary

(Add summary)

### Main Changes

## 会话 4：分类框「完全不可用」两条根因定位与修复（锁定=输入全灭 / 库 debug panic 杀死渲染线程）

用户实机验收报 5 症状（标题双击无效、锁定与三点按钮点不动且 hover 闪烁、
标题拖拽后整框卡死、图标 hover 闪烁、拖不动 → 无法验收对齐参考线）。

### 根因 1（P0，宿主接线错误）：锁定被实现成「块不参与点击穿透」
- `attach_box` / `set_box_locked` 用 `set_interactive(!locked)` 表达锁定；库里
  `interactive=false` 的语义是画布对该块**永久保持 `WS_EX_TRANSPARENT`**。
- 引擎架构下分类框窗口是全透明代理窗（画面由库合成），命中链
  「代理窗透明像素 → 库画布 → sink → 代理窗」第二环被切断 → 整块区域零鼠标消息，
  连解锁按钮本身都点不到。用户两个框当时都是 `locked=true` → 表现为整体不可用。
- 修复：新增统一口径 `block_config_for()`：锁定只关 `resize.enabled`，
  `interactive` 恒 true；拖拽放行仍由 sink 的 `BlockGeom::locked` 与内容层
  `WM_LBUTTONDOWN` 判定。补单测钉死（含 movable/resize/interactive 三项断言）。

### 根因 2（P0，库缺陷 + 宿主复活判据失效）：debug 构建约 4.4 分钟必死
- 库 `core/quality.rs:225` `good_streak: u8` 在质量档 0 下每秒自增且永不复位，
  256 拍后 `+= 1` 在 dev 的 overflow-checks 下 panic，渲染线程当场死亡。
- panic 跳过库的 `set_running(false)`，`is_running()` 永远返回 true，
  宿主 `revive_if_dead` 因此永不触发；日志实锤 4 次 panic（09:35/16:16/16:42/16:58，
  均在引擎启动后约 266 s），panic 后 `玻璃引擎[运行中] fps=0` 一直刷。
- 修复：① `Cargo.toml` 加 `[profile.dev.package.liquid-verre-rs] overflow-checks=false`
  （`cargo build -v` 实测该依赖现在带 `-C overflow-checks=off`，主 crate 不变）；
  ② panic hook 按栈地址命中 `liquid-verre` 时 `mark_engine_suspect()`（hook 内不调
  引擎 API，防在垂死线程里自死锁），`revive_if_dead` 增加「嫌疑标志 + 快照 epoch
  连续 2 拍停滞」双条件复活通道，10 s 限流（判定与记账分离，避免永远过不了限流门）。
- 待库侧根治：`good_streak`/`bad_streak` 改 `saturating_add(1)`，发 0.5.2。

### 顺带修掉的工程问题
- 工作树**编译不过**：`poll_glass_hover` 用了 `GridWindowData::hover_out_ticks`，
  结构体却没有该字段（上一轮改动只落了一半）→ 补字段 + 三处初始化；
  hover 离开块需连续 2 拍才补发 `WM_MOUSELEAVE`（单拍越界抖动即闪烁源）。
- 接管式拖拽看门狗：左键已物理抬起却没收到 UP 时强制 `end_box_drag` 收尾
  （否则窗口以 16ms 追着手跑 = 用户看到的「一拖就卡死」）；
  `begin_box_drag` 在库缩放会话期间让位，避免两套状态机抢同一窗口。
- sink 的 `Move` 日志由 info 降为 debug（120Hz 刷日志）。
- `wake_canvas_idle` / `revive_if_dead` 注释口径同步到 0.5.1。

### 测试
- [OK] cargo test 515 全绿（新增 2 条回归钉）、cargo fmt --check 干净、build 54 warnings（基线持平）
- [OK] PostMessage 复现 sink 下发序列：锁定切换 / 标题双击建 edit-box / 三点菜单 / 图标命中 [CLICK] hit=0
- [OK] cargo build -v 确认依赖编译参数 -C overflow-checks=off
- [BLOCK] 端到端真实鼠标无法在 Codex 执行环境验证：Job Object 限制
  （SetCursorPos/SendInput err=5、OpenDesktop 拒绝访问、DDA 0x80070005 → 库回退
  WGC 且 fps=0、块 is_alive()=false）；且分类框窗口带 WDA_EXCLUDEFROMCAPTURE，
  截图看不到框体（核对画面需 IKUN_ALLOW_CAPTURE=1）。已给用户 5 条实机验收清单。


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 5: 真实鼠标复现再挖两条根因：库无 CS_DBLCLKS 致双击不触发 / FocusHint 抢焦点杀菜单

**Date**: 2026-10-28
**Task**: 真实鼠标复现再挖两条根因：库无 CS_DBLCLKS 致双击不触发 / FocusHint 抢焦点杀菜单
**Branch**: `master`

### Summary

(Add summary)

### Main Changes

## 会话 5：真实鼠标复现，再挖两条根因（库无 CS_DBLCLKS 导致双击永不触发 / FocusHint 抢焦点杀掉模态菜单）

用户复验：上一轮两条根因（锁定=输入全灭、库 debug panic）已好，只剩「其他 4 个症状照旧」。
本轮用真实鼠标（纯 ctypes，PowerShell Add-Type 会被火绒当病毒拦）+ 日志逐条复现。

### 根因 3：库画布类没有 CS_DBLCLKS → 双击标题永远不触发
- 库 platform/window.rs 只给 CS_CLASSDC，Windows 不发 WM_LBUTTONDBLCLK，
  PointerPhase::DoubleClick 分支永不执行；日志实锤用户双击 = 两组独立 Down/Up，
  每组还各拉起一次接管式拖拽 + save_config。
- 宿主侧合成双击：RouteState 记上次 Down 的单调 ms 与块内坐标，落在
  GetDoubleClickTime + SM_C?DOUBLECLK/2（按 DPI 折算）内即判双击，统一走
  post_double_click（标题 WM_NCLBUTTONDBLCLK / 主体 WM_LBUTTONDBLCLK）。
  纯函数 is_double_click + 单测。按钮热区排除（锁定/更多连点≠双击标题）。
- 待库 0.5.2 补 CS_DBLCLKS（补上后本合成与真相位同口径，不会双触发）。

### 根因 4：库 FocusHint 抢回焦点 → 模态菜单刚弹出就自杀（三点按钮「点了没反应」）
- 三点按钮在 WM_LBUTTONDOWN 里同步开 show_menu_window（菜单靠 WA_INACTIVE 关闭），
  而块被按压后库发 FocusHint → WM_USER_GLASS_FOCUS → SetFocus(代理窗) → 菜单立刻失焦关闭。
- 修复：菜单打开中 / 就地重命名编辑中，忽略 FocusHint 抢焦点。
- 验证（不碰光标的 PostMessage 复刻 sink 序列，两框都过）：
  Down → DesktopOrgMenu 创建 → 再投 WM_USER_GLASS_FOCUS → 菜单仍存活 → ESC 正常关闭。

### 顺带
- 接管式拖拽参考线懒创建（无位移不建全屏 overlay、不写配置）。
- 新增 debug_log_blocks()：1 s 一条打印每块库侧真值（rect/interactive/occluded/...），
  「一个框能用另一个不能」从此一眼可判。
- 实测光标进块 → 画布摘除 WS_EX_TRANSPARENT 延迟 23–25 ms（锁定框同样在线）。

### 测试
- [OK] cargo test 516 全绿（+is_double_click 回归钉）、fmt 干净、build 54 warnings（基线持平）
- [OK] 真实鼠标：标题双击 → 编辑框弹出（日志实锤）
- [OK] 真实鼠标：锁定切换 / 图标点击 / 框间拖拽 全通（用户手点 + 合成点击）
- [OK] PostMessage：三点菜单在 FocusHint 之后存活，ESC 正常关闭
- [OK] 用户数据核对：误拖拽只把 Docker Desktop 挪回其原始归属框，无损失
- [待办] 拖拽跟手 + 参考线时机 + hover 是否仍闪：需用户目视（窗口带 WDA_EXCLUDEFROMCAPTURE，
  录屏/截图取不到框体画面）；另需注意用户在机器前时不要合成鼠标输入（会抢光标）。


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 6: 缩放会话收尾竞态 + 退出11/11必崩 + 内容层掉帧

**Date**: 2026-10-28
**Task**: 缩放会话收尾竞态 + 退出11/11必崩 + 内容层掉帧
**Branch**: `master`

### Summary

(Add summary)

### Main Changes

## 会话 6：缩放会话收尾竞态 + 退出 11/11 必崩 + 内容层掉帧（三条新缺陷定位与修复）

用户复验「锁定好了、其余照旧」并补 3 条新观察。本轮全部落到根因并修掉两条半：

### 根因 5：库缩放会话「第一拍就收尾」
WM_USER_GLASS_RESIZE(1) 与 WindowSnapshot.resizing 不同通道，空闲档快照 ~4Hz，
会话头几拍仍读到 resizing=false → 旧代码当拍收尾 → glass_session_active 掉回 false
→ WM_WINDOWPOSCHANGED 恢复把旧窗口尺寸反推给块（用户看到「过一会儿自己复原大小」），
且内容层再没收到新尺寸 WM_SIZE（不重排/不重画/不推纹理）。库的内容纹理是
「与块 1:1、锚定左上角、不拉伸」，所以块变大而纹理没跟上 = 按钮图标不跟尺寸。
修复：纯函数 session_next 双条件去抖（必须先见过 true，再连续 3 拍 false；
从未见过 true 时 80 拍宽限兜底）+ 单测。实测 PostMessage 复刻会话：1317ms 正常收尾（旧：当拍）。

### 根因 6：退出路径 11/11 必崩（KERNELBASE 0x87A）
WER 里 10-26~10-28 每次退出都记一次 APPCRASH；app_handle.exit(0) 后约 1s 崩在
Tauri/WebView2 收尾，且 Tauri 事件循环从不返回。旧看门狗用 std::process::exit(0)
强杀 → 跑 CRT atexit/DllMain，与仍在飞 COM 的 WebView2/D2D 线程对撞。
修复：清理全做完后留 400ms 直接 TerminateProcess 主动收尾；看门狗改为轮询
TEARDOWN_DONE + TerminateProcess；main.rs 事件循环返回时记一笔。实测 3/3 退出零崩溃。

### 优化 7：会话期内容层掉帧
内容层软件光栅化 5~11ms/帧，逐拍全量重画追不上 120Hz 玻璃。改为：会话期重画节流
~30Hz（收尾必补一次全量）+ 跟随间隔 16→8ms + 会话期 timeBeginPeriod(1)/timeEndPeriod(1)
（引用计数配平；不抬粒度 SetTimer(8) 实际仍 15.6ms）。

### 症状「hover 闪烁」本轮只布取证点
4 条迁移日志提到 info 级（hover lock/more/item + WM_MOUSELEAVE），等用户停鼠标 3 秒
数来回次数：怀疑是「轮询 inside 判定」与「TrackMouseEvent 真实光标判定」两套口径打架
（代理窗全透明，真实鼠标不会给它发 MOVE）。定位后降回 debug。

### 测试
- [OK] cargo test 517 全绿（+1 回归钉）、fmt 干净、build 54 warnings 持平
- [OK] 会话去抖：PostMessage 实测 1317ms 收尾（旧实现当拍退）
- [OK] 退出：3/3 无 WER 崩溃（此前 11/11 必崩）
- [OK] 交互回归：真实鼠标双击→编辑框、三点菜单存活、锁定切换、图标命中（上一轮已验）
- [待办] 用户复验：拖边缘改大小是否跟手不弹回、拖位置/改尺寸是否还掉帧、hover 是否仍闪


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 7: hover闪烁定案(TrackMouseEvent假LEAVE) + 引擎模式每帧白烧一趟ULW

**Date**: 2026-10-28
**Task**: hover闪烁定案(TrackMouseEvent假LEAVE) + 引擎模式每帧白烧一趟ULW
**Branch**: `master`

### Summary

(Add summary)

### Main Changes

## 会话 7：hover 闪烁定案（TrackMouseEvent 假 LEAVE 风暴）+ 引擎模式每帧白烧一趟 ULW

### 根因 8：hover 闪烁 = TrackMouseEvent 在引擎模式下每次都立刻回假 WM_MOUSELEAVE
代理窗是全透明命中层，真实命中的是库画布，TrackMouseEvent 按真实命中链判定「离开」，
于是轮询刚设上 hover 就被清掉，10Hz 无限抖动。当天日志实锤：WM_MOUSELEAVE 2770 条、
hover 迁移 1345 条，11:36 段是 hover item -1->0 与 LEAVE 的逐拍交替。
修复：glass_mode 下不注册 TrackMouseEvent，离开交给 poll_glass_hover 连续 2 拍判定。
修复后只剩正常 leave（11:47 之后 4 条，均为真离开）。

### 根因 9：引擎模式下代理窗每帧仍跑「D2D 合成 + ULW 提交 3.5MB 全透明位图」
实测光栅合成 ~3.8ms + 提交 ~1.2ms，画面却不会有任何变化 —— 会话跟随的帧预算被这
5ms/帧吃光，就是「内容掉帧、玻璃背景流畅」。修复：尺寸与玻璃状态未变时短路返回
（新增 submitted_w/h/glass 记账），并把会话期内容重画节流 33ms → 16ms。

### 记录两条口径/方法边界
- 库 hit_resize 的 grip = grip_px × (该屏DPI/主屏DPI)，主屏 200% 下只有 10 物理像素，
  宿主边缘判定却是 scale_for_dpi(8,dpi)=16 物理像素 → 两套口径；实测离边 12px 按压
  被判成块内点击而非缩放。待定：宿主抬 grip_px 默认值，或库按 DPI/96 折算。
- 截图不能验证玻璃：画布是 DComp flip-model，BitBlt 取到冻结帧（像素差恒 0）；
  真实鼠标测试前必须扫遮挡（本轮 Chrome 铺满主屏，标题带与手柄都点不到）。
  测试改动的用户布局已从备份还原。

### 测试
- [OK] cargo test 517 全绿、fmt 干净、build 54 warnings 持平
- [OK] hover 假 LEAVE 风暴：2770 条 → 修复后仅存正常 leave
- [OK] 交互回归：光标进块摘除穿透 22ms、块矩形与窗口矩形同步（拖拽后仍一致）
- [待办] 用户目视：hover 是否还闪、拖拽/缩放时内容是否还掉帧
- [受阻] 库缩放手柄与标题拖拽的真实鼠标验证：Chrome 铺满主屏遮挡，需用户让开窗口


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 8: 内容掉帧量化：库快照4Hz发布 + 会话期每帧重交9MB透明ULW帧

**Date**: 2026-10-28
**Task**: 内容掉帧量化：库快照4Hz发布 + 会话期每帧重交9MB透明ULW帧
**Branch**: `master`

### Summary

(Add summary)

### Main Changes

## 会话 8：内容掉帧量化定位 —— 库快照 4Hz 发布 + 会话期每帧重交 9MB 透明 ULW 帧

给「标题/图标/按钮跟着位置大小变化时明显掉帧，玻璃背景却流畅」上了毫秒级 tick 探针
（日志时间戳只到秒，自己打 now_ms），真实鼠标受控缩放一次 1.4s 做基线：
块几何只变 6 次、内容只推 5 次 —— 3.4Hz。

### 根因 10a：库快照发布节流，会话改几何不置 snapshot_dirty
runtime/mod.rs:870 是 `snapshot_dirty || 每 250ms` 才发布，而库自己的缩放/移动会话
改块矩形时不置脏（只有宿主 setter/建块/显示器/质量档置脏）→ 宿主读到的块矩形只有
4Hz → 代理窗尺寸 → 内容 DIB 尺寸 → push_content 全部 4Hz。
宿主绕行（不等发版）：会话每拍调 `Engine::set_default_resize(当前值)` —— 唯一
「置脏但不碰块也不触发重渲染」的公开 setter（commands.rs SetDefaultResize 返回 false）。
实测：块几何 4Hz → 54Hz，内容 push 3.4Hz → 20Hz。

### 根因 10b：会话期每帧重交整窗透明 ULW 帧
成本随面积（1712x1332 ≈ 9MB/帧）把跟随 tick 拖到 30Hz。改为引擎模式下：尺寸没变
或正在会话期就直接收工（代理窗本来全透明，会话收尾必补一帧）。实测 tick 30→54Hz。
踩坑：想用 1x1 透明帧糊弄不行 —— UpdateLayeredWindow 的 size 会同时把窗口缩成 1x1，
分类框直接消失（实测复现）。

### 剩余上限在库侧
内容 push 现在稳定 ~20Hz，瓶颈已是库会话几何更新与快照发布自身节奏；要 60Hz+
需库在会话改几何时置 snapshot_dirty。0.5.2 清单累计三条：quality saturating_add、
画布补 CS_DBLCLKS、会话置 snapshot_dirty（+ 把手带 grip 按 DPI/96 折算的口径问题）。

### 操作教训
用户在机器前时跑真实鼠标脚本 = 抢光标，合成点击会变成拖拽（曾把图标拖进别的框）；
我随后拿 11:47 备份还原配置，差点覆盖掉用户之后的手工整理，已回滚到最新状态。
以后：真实鼠标测试前先确认用户空闲；不要用旧备份盖新配置。

### 测试
- [OK] cargo test 517 全绿、fmt 干净、build 54 warnings 持平；tick 探针已拆除
- [OK] 实测帧率：块几何 4Hz→54Hz、内容 push 3.4Hz→20Hz、跟随 tick 30Hz→54Hz
- [待办] 用户目视：缩放/移动时按钮与图标换行是否还卡
- [待办] 库 0.5.2 三条（可代改 D:/github-project/liquid-verre-rs，待发版授权）


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 9: 按库性能契约重写接管式拖拽：Move 内 set_rect 对齐 120Hz

**Date**: 2026-10-28
**Task**: 按库性能契约重写接管式拖拽：Move 内 set_rect 对齐 120Hz
**Branch**: `master`

### Summary

(Add summary)

### Main Changes

## 会话 9：按库的性能契约重写接管式拖拽（Move 内 set_rect，对齐 120Hz）

用户「还是掉帧」并指明方向：读 liquid-verre-rs 源码的性能要求，对齐它的帧数。

### 对齐依据（库仓库 README / BASELINE.md）
- IDLE_WAIT_MS=8ms≈125Hz，注释明写覆盖 120Hz 一个刷新周期；基线「单块拖拽
  113~116fps / p50 8.32~8.36ms（120Hz 锁相）」，验收线「120Hz 屏拖拽 ≥90fps」。
- 接管拖拽官方写法：sink 返回 Consumed 接管，Down 记锚点，**序列内每次 Move 用
  set_rect 驱动（异步入队 ≤1 帧，与库会话同等延迟）**；sink 线程契约明确允许
  回调内调 setter。

### 改造
旧链路天花板是宿主定时器（SetTimer(8) 实测 ~54Hz）+ GetCursorPos→SetWindowPos→
WM_WINDOWPOSCHANGED→push 四段回环。新链路：RouteState 带 GlassWindow 句柄与
最近下发矩形（sink 内绝不碰 HOLDER，避免三方锁环），Down 命中标题空白（复用
原本死掉的 down_routes_title_drag）开序列，Move 内直接 set_rect，Up 收尾；
代理窗降级为跟随者（WM_USER_GLASS_DRAG_SYNC，按 3px 阈值合并，收尾 wParam=1），
WM_WINDOWPOSCHANGED 在拖拽期不得反推块矩形。删掉 begin_box_drag、定时器拖拽分支、
SetCapture/ReleaseCapture 与四个锚点字段，hover 轮询保留「左键已抬→强制收尾」兜底。

### 踩到并修掉的递推错误（关键教训）
block_local 是相对块**当前**原点的（库里没有屏幕坐标字段）。第一版写
R_k = R_0 + (block_local_k - anchor) 会自我抵消：实测「拖 240px 只走 136px」。
正确递推 R_k = R_(k-1) + (block_local_k - anchor)（基准=上一次请求的矩形）。
修后实测：光标 (+240,+140) → 窗口 (+240,+140) 精确一致，块与窗一致，落盘 1 次。

### 仍未对齐：缩放
库的缩放会话由库驱动（把手带优先级恒高于 sink，宿主拿不到那段事件），宿主只能跟随
快照；上一轮已用 set_default_resize 催办绕过（块几何 4Hz→54Hz，内容 push ~20Hz）。
彻底对齐要库侧「会话改几何置 snapshot_dirty」→ 0.5.2 清单第 3 条。

### 测试
- [OK] cargo test 517 全绿、fmt 干净、warnings 52（比基线少 2：死代码转正式链路）
- [OK] 真实鼠标受控拖拽：驱动频率从定时器 54Hz 变为按 Move 逐帧（脚本 33Hz 光标下
  实测 49Hz，上限=库帧率 125Hz）；落位误差 0；参考线与落盘各 1 次
- [待办] 用户目视确认移动是否已顺滑；缩放仍需库 0.5.2
- [注意] 我的测试把示例框位置/尺寸改成了 (48.5,50.5)/623x354.5，图标整理结果（13 个）
  已保留，位置需用户拖回


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete
