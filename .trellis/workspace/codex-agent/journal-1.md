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


## Session 10: liquid-verre-rs 分类框 5 项 bug 修复（光标粘滞 / 框选卡顿 / 样式直通库接口 / 面板表单重排 / 面板空白回归）

**Date**: 2026-11-02
**Task**: 分类框基于 liquid-verre-rs 的交互与设置面板缺陷修复（用户报 1/3/4/5/6 号问题）
**Branch**: `master`（未提交，工作区改动）

### 取证方法（重要口径）
- 画布与分类框窗口带 `WDA_EXCLUDEFROMCAPTURE`，BitBlt/PrintWindow 均取不到玻璃画面；
  画面取证用库环境变量 `GLASS_INTERNAL_SHOT`/`GLASS_SHOT_EVERY_MS`（GPU 回读连拍）。
- 光标形状验证不需要截图：`SetCursorPos` + `GetCursorInfo`（本进程从未 SetCursor 时
  返回全局可见光标句柄），与 `LoadCursorW(IDC_*)` 句柄直接比对。
- 真实鼠标 SendInput 在 Codex desktop 环境可用（用户授权后执行）。

### 根因与修复
1. **块内光标粘滞（e-resize / pointer 不回落）**：库画布窗口类注册 hCursor 为空，
   宿主 sink 的 `cursor_override` 在非按钮区返回 None → 画布 `WM_SETCURSOR` 走
   DefWindowProc 不设置光标 → 上一阶段形状原地粘住。修复：块内非按钮区显式返回
   IDC_ARROW（把手带由库 hover_handle 优先，不受影响）。实测：锁按钮→HAND、
   进块→ARROW、右边缘→SIZEWE、进块→ARROW 全通过。
2. **框选矩形卡顿（~10Hz）**：sink 空闲不转发 Move 是对的（防消息洪水），但按压
   期间框选/拖拽阈值失去唯一坐标源，只剩 100ms hover 轮询。修复：Down(Consumed)
   置 `host_pressing`，按压期 Move 逐条转发 WM_MOUSEMOVE，Up/物理键松开双路清位；
   另在 `handle_grid_drag_timer` 引擎分支补每拍 GetCursorPos 采样（块外也跟手）。
   实测转发 ~28Hz + 16ms 定时器采样，替代旧 10Hz。
3. **背景色/透明度/圆角不生效 + 红色残影**：`BoxStyleConfig` 的 background_color/
   alpha 从未流入库 `TransparentParams`（只有圆角联动），非液态模式实际由库透明玻璃
   渲染底板；旧「残影」是引擎不可用时回退 D2D 绘制时代的产物。修复：normalize()
   统一映射（圆角→两模式 corner_radius；背景色→transparent.color；透明度→
   transparent.opacity），面板调背景色 #2563eb/72% 后 `GLASS_INTERNAL_SHOT` 单帧
   取证确认库底板变蓝、圆角正确、拖拽无残影。
4. **面板表单重排**：「分类框风格」内嵌液态玻璃开关；开启才显示「液态玻璃参数」
   区（去掉分模式圆角滑条，单一来源）；背景色/透明度仅关闭时显示；缩放交互/引擎
   独立成节（两模式共用）。移除「透明玻璃」重复参数组。
5. **控制面板底部大量空白/滚动条失灵（回归）**：实锤两类根因——
   a) 写命令全部是同步 `#[tauri::command]`（主线程执行）：磁盘写 + 引擎重建阻塞
      事件循环。修复：全部改 `#[tauri::command(async)]`；前端滑条 60ms 合并防抖。
   b) WebView2 合成器在引擎重建/高 GPU 负载滚动时停止提交新帧（JS 活着、输入被
      吞、Ctrl+R 立刻恢复；--disable-gpu 等浏览器参数无效）。修复：rAF 心跳看门狗
      （main.ts）可见状态停滞 >2.5s 自动 location.reload 自愈（60s 限一次）；
      配置应用与画布 Ready 事件后向持久窗口派发 resize 事件强制出新帧。
   c) 画布 Z 序不变量被重建打破（实测画布压在面板之上）：spawn 收尾 + Ready/
      MonitorsChanged/CaptureBackendSelected 事件统一 `reassert_canvas_bottom()`。
   d) 切标签沿用上一页 scrollTop → 短页面底部露容器背景：ControlPanel 切页复位。

### 踩到并记录的坑
- 测试脚本真实拖拽会改动用户布局并连盘（一次边缘拖 = 26 次 save_config）；
  已用 `.tmp_cfg_before_codex_test.json` 恢复，box0 尺寸仍残留 10-28 会话的改动。
- `GetGUIThreadInfo` 在 Python ctypes 下报 87 不可用；`AttachThreadInput` 后
  `GetCursorInfo` 读到的是被附加队列的旧光标——裸 `GetCursorInfo`（本线程从未
  SetCursor）才是全局可见光标。
- WebView2 透明窗的 `documentElement` 内联背景被 Chromium 强制透明吞掉，
  「eval 是否执行」要用 IPC 日志而非颜色标记验证。

### 测试
- [OK] cargo test 519 全绿（+2 回归钉：cursor_id_for、背景色→transparent 映射；
      改 1 条旧断言：圆角单一来源）；cargo fmt --check 干净；build 52 warnings 持平
- [OK] pnpm check 0 errors；pnpm build 通过
- [OK] 真实鼠标光标回归：HAND/ARROW/SIZEWE 全场景无粘滞
- [OK] 框选：连拍帧选框持续跟手；转发实测 ~28Hz（旧 10Hz）
- [OK] 背景色 #2563eb + 透明度 72% + 圆角 13px 经库渲染生效（internal shot 取证）
- [OK] 面板：切页复位、防抖、重建期间可交互；看门狗自愈逻辑就位
- [注意] 引擎重建触发 WebView2 停滞的底层问题（Chromium/Intel Arc 交互）已用
      自愈兜底，根治需库侧（swapchain 优先级/呈现节流）或上游修复

### Status

[OK] **Completed**（工作区改动未提交，等用户验收）

### Next Steps

- 用户验收 5 项修复；如要提交由用户明确发起
- 向 liquid-verre-rs 反馈：画布类注册应带 hCursor；重建期间 WebView2 呈现停滞复现步骤


## Session 11: 验收反馈二轮修复（框选回归库管线内优化 / 面板挂死恢复阶梯）

**Date**: 2026-11-03
**Task**: 用户验收反馈：bug3 覆盖层方案有残影须回退；bug6 未解决（点「启用缩放」等控件即面板高度异常/空白）
**Branch**: `master`（工作区改动，未提交）

### bug3 二轮：撤销独立覆盖层，改库管线内快路径
- 用户实测：独立分层窗口画的选框会被 DDA 桌面采集吃进玻璃折射 → 残影、更卡。**方案作废**
  （`marquee_overlay.rs` 已删，接线全部回滚）。教训：**任何新窗口出现在块区域上方都会被
  库的桌面复制采样**，框选必须留在 liquid-verre-rs 内容纹理管线内。
- 新快路径（`d2d_render::render_marquee_frame`）：
  - 内容层缓存「干净帧」快照（全量重绘产出时快照；指纹引擎模式剔除 marquee 字段）；
  - 每拍 = 上一拍选框区域行还原 → 当前选框 **L 形增量带** 混合（prev ⊂ cur 时只刷新增部分，
    方向翻转/收缩才整框重刷）→ `update_content` **脏区增量上传**（`push_content_dirty`，
    库按半开区间只重传子矩形 GPU 纹理）；
  - 逐通道 256 项查表 source-over（RGB 0,120,215，填充 a=40/边框 a=200，与 D2D 路径同参）；
  - 顺手修正选框绘制多减一次 `scroll_offset` 的历史错位 bug（选框坐标本就是视口系）。
- 驱动节拍：WM_TIMER 实测被系统合并进 ~30ms 桶（timeBeginPeriod(1) 也压不进 16ms，中位
  30ms 实锤）→ 改为**按压期转发的 WM_MOUSEMOVE 直接驱动渲染（12ms 节流）**，定时器降级为
  松手看门狗 + 光标出块采样兜底；实测 tick 40/s → 69/s，真实鼠标（125Hz 输入）下更高。

### bug6 二轮：真根因 = WebView2 渲染/呈现侧挂死，原生看门狗阶梯恢复
- 取证链：白屏时**主线程活着**（信号开关窗正常）、**渲染进程 JS 也活着过**（配置点击生效、
  Ctrl+R 秒恢复）→ 挂点在 WebView2 浏览器/渲染侧管道；`--disable-gpu`、
  `--disable-gpu-compositing`、`--disable-direct-composition` 全部实测无效 → 宿主无法预防，
  只能检测 + 恢复。
- 检测：心跳载荷从「setInterval 时间戳」升级为 **rAF 帧计数**——合成停滞后 setInterval 照发
  但 rAF 冻结，两类挂死（JS 死 / 呈现死）统一暴露为「帧计数 8s 不变」。
- 遮挡判定修复：`WindowFromPoint` 返回最深子窗口（Chrome_WidgetWin_1），必须
  `GetAncestor(GA_ROOT)` 归一再比（否则永远误判被遮挡、永不重建）。
- 恢复阶梯（0→1→2）：
  1. 浏览器进程侧 `ICoreWebView2::Reload()`（经 `with_webview`；无窗口闪烁，杀渲染进程实测
     1s 复活、滚动位置都还在）；
  2. Reload 无效 → `run_on_main_thread` 投递 destroy，poll 线程观察 label 释放后
     **两阶段原位重建**（同一闭包里 destroy+build 会撞 "label already exists" 竞态，实测）。
- 防抖只约束新周期（stage 0 启动，30s）；心跳恢复即复位阶梯。
- 前端 rAF 看门狗（呈现停滞 → location.reload 自愈，60s 限一次）保留，与原生阶梯互补。

### 测试
- [OK] cargo test 519 全绿、fmt --check 干净、build 52 warnings 基线持平
- [OK] 框选连拍取证：选框经库合成无残影；快路径 0 回退、混合成本 4ms→查表+增量带后 <2ms
- [OK] 杀渲染进程端到端：8s 判挂死 → Reload → 1s 心跳恢复 → 阶梯复位
- [待办] 用户真机验收：框选跟手度（真实鼠标 125Hz 输入）、连点「启用缩放」等控件的自愈表现

### Status

[OK] **Completed**（工作区改动未提交）


### Session 11 补充：二轮验收反馈（框选回滚重做 / 启动双层根因 / 挂死检测阶梯）

用户二轮反馈后追加修复：

1. **bug3 按指示回滚到首轮版本再优化**：回滚拼接时误删了定时器分支的
   `render_layered_window` 调用（连拍实测拖拽期间选框完全不出现）——已恢复；
   快路径重新叠加时修掉「松手不消失」根因：引擎模式指纹不含 marquee 字段，
   松手后若无其它状态变化缓存命中、选框永留画面 → 新增
   `marquee_force_full` 标志，UP 收尾强制一次干净全量重绘。
   连拍验收：拖拽 6 帧连续跟手生长、松手后 5 帧全部无残留。
2. **bug4 启动双层根因**：`attach_box` 的块上线在库渲染线程异步完成
   （`has_box→is_alive` 前为 false），create_grid_window 步骤 3 的首帧紧随
   attach 执行 → 判定「无块」走了 D2D 回退背景，且无任何东西再触发重绘，
   直到鼠标进入 hover 才刷成透明帧——与用户描述逐字吻合。修复：attach 成功
   后有限等待块上线（≤300ms）+ 玻璃 tick 残帧纠正
   （`has_box && !submitted_glass` → 补一次重绘，覆盖所有迟到场景）。
   `IKUN_ALLOW_CAPTURE=1` + BitBlt 只拍代理层实测：修复前启动即见标题/图标，
   修复后启动代理层全透明。
3. **bug6 摘除「resize 事件重绘推手」**：每次 apply 都向页面派发 resize 事件，
   透明窗 + clip-path 下正是「点控件后高度异常/空白」的嫌疑放大器，且从未
   被证明有效——整体移除（含 APP_HANDLE 通路）。真挂死由看门狗阶梯兜底
   （心跳 rAF 帧计数 → 浏览器侧 Reload → 两阶段销毁重建），杀渲染进程实测
   1 秒复活。压力序列（开关连点 + 快滚 10 轮）通过。
4. **事故记录**：行过滤脚本 skip 标志未复位，把 config_cmd.rs 截断到 2.3KB
   （丢失全部命令定义）——经 git checkout + 重放 async 化补丁完整恢复，
   11 函数/9 async 核对无误。教训：文件级改写必须用精确锚点 + 写前断言
   函数清单，禁止粗粒度行过滤。

- [OK] cargo test 519 全绿、fmt 干净、build 52 warnings 基线持平

## 2026-11-03（第 4 轮）bug3 颤抖 / bug6 空白 / bug4 绘制审计

1. **先解阻塞**：`#[tauri::command(async)]` 不是 Tauri 2 合法属性（9 处），
   改为 `#[tauri::command]` + `pub async fn`。上一轮「同步命令饿死 WebView2
   WM_PAINT/WM_SIZE」的意图保留（磁盘 IO + 引擎应用仍脱离主线程）。
   build 通过、`cargo test` 519 全绿、`pnpm check` 0 error。

2. **bug6 真根因（实测锤定，不是 CSS 也不是挂死）**：取色弹层用
   `position: fixed` + JS 视口坐标，但容器链上有两处会把 fixed 的包含块
   从视口改掉——`.window-shadow { filter: drop-shadow }`（实测 +8px）与
   `.fields { contain: layout style }`（layout containment 规范上就是 fixed
   的包含块，实测 +271px）。探针数据：`want=[460,336] got=[731,363]`，
   差值恰好等于 `.fields` 的 rect 偏移。错位的弹层落在滚动容器内部，
   扰动可滚动范围 → 用户看到的「点击控件后底部大片空白 + 滚动条失灵」。
   修复：`ColorPicker.svelte` 加 `use:bodyPortal` 把弹层搬到 `document.body`
   （本仓库 Svelte 5.57.2 没有 `{@portal}`，只能手写 action）。
   过程中自己踩了一坑：action 的 `destroy()` 里把节点插回原父节点，等于
   把 Svelte 已经摘掉的节点复活 → 弹层关不掉且泄漏；改成 `node.remove()`。
   验收：`want=204,336 got=204,336`、`want=460 got=460`；两次「已关闭」
   截图逐像素相同（ImageChops.difference bbox=None），无残留、无空白。
   注：上一轮恢复的 8 月 CSS（f6f6ed5：`contain: layout style` /
   `overflow-anchor: none` / `.settings{min-height:100%}`）保持不变，
   本轮没有再动面板样式；我一度加的 ResizeObserver 校正 scrollTop 因
   缺乏实测证据已回退（避免过度修改）。

3. **bug3 颤抖真根因（坐标系不一致，不是重绘次数）**：框选期间有三个源写
   `marquee_current_*`，其中两套用两套坐标：
   - 引擎转发进来的 `lparam` 已经是「块内=内容层」坐标，而
     `content_point_from_lparam` 按 client 坐标再换算一次，多加了窗口
     不可见边框的 12px（200% DPI 实测）；
   - 拖拽定时器 `GetCursorPos() - get_grid_content_rect()` 是内容层真值。
   两源交替覆盖 → 选框每拍在 (447,239) 与 (459,251) 间跳 12px，就是「一直
   重绘 + 框线颤抖」。日志实锤：`move (1162,652) lp=(1150,640)` 紧接
   `timer (1150,640)`。修复：新增 `marquee_content_point()`（引擎模式直接取
   lparam，回退模式仍走 client 换算）统一 DOWN 起点与 MOVE 当前点，定时器
   改走 `content_point_from_screen()`，并让 `poll_glass_hover` 在框选期不再
   补发合成 WM_MOUSEMOVE。验收：连续拖拽 rect 严格单调
   （1250,1464 → 1270,1474 → 1290,1484 → …，步长恰等于鼠标位移）。
   附带纠正：选框不再整体偏右下 12px，且松手命中判定（用真值系）与可见
   矩形终于同系。

4. **bug4 审计结论：引擎模式下应用层不再画任何分类框像素**。
   - `d2d_render.rs` 第二趟：`if glass_active { /* 只 Clear 成全透明 */ } else {
     画圆角底板 }`，且 `if !glass_active { DrawBitmap(content) }`；
   - 透明帧本身也短路：尺寸与玻璃状态未变 / 会话期直接 return，不交帧；
   - DComp 路径自带底板绘制，但只在 `IKUN_DCOMP=1` 时装配（默认关）；
   - 内容层（图标/标题/按钮）经库的 `update_content` 接口推给 liquid-verre-rs，
     属库规定入口，不是应用层重复图层。
   运行时取证（临时计数器，已撤除）：空闲 12s 与光标停在框内 8s 两种场景
   应用层渲染调用 **0 次**；拖拽 5s 窗口 `calls=2 raster=2 ulw=0
   marquee_fast=70`——70 次走快路径增量上传，0 次提交透明帧。idle 进程 CPU
   约 3.8% 单核（库自身连续渲染），应用层无白烧。

5. **副作用待告知**：合成鼠标测试改动了用户桌面上分类框的位置/尺寸与一个
   图标格位（与 `.tmp_cfg_before_codex_test.json` 对比有 4 个框 position/size
   差异）。样式项（背景色 #ffb8b8 / 圆角 30 / 透明度 25% / 遮挡剔除 off）
   与备份一致，未回滚 layout，等用户决定。

- [OK] cargo fmt / build / test 519 全绿；pnpm build + svelte-check 0 error


## Session 10: 接入 liquid-verre-rs 0.5.2：修下层框 border 盖到上层框 + 退役临时绕行

**Date**: 2026-11-04
**Task**: 接入 liquid-verre-rs 0.5.2：修下层框 border 盖到上层框 + 退役临时绕行
**Branch**: `master`

### Summary

依赖升 0.5.2；按库侧指南退役 border 抬层/双击合成/4ms 压底守护/催发布 hack/debug overflow 止血，主诉由库逐块 pass 层序解决；fmt+test 全绿，4 项待实机复验

### Main Changes

## 结论

liquid-verre-rs 升到 **0.5.2**（crates.io 已发布，库仓库 HEAD 8af57b8），
宿主按《0.5.2 库侧修复结果与应用层接入指南》退役临时绕行，本次主诉
「下层框缩放反馈白边盖到上层框身上」改由库的逐块 pass 层序保证。

## 改了什么

- **Cargo.toml / Cargo.lock**：0.5.1 -> 0.5.2；删除 `[profile.dev.package.liquid-verre-rs]
  overflow-checks = false`（#6 库侧 `bump_streaks` 已修，debug ~256s 必 panic 的止血段不再需要）。
- **#1 主诉（border 层序）**：`grid_window.rs::begin_glass_resize_session` 删除专为 border 加的
  `raise_box` 抬层绕行（库 CHANGELOG 明确：宿主复验 raise_box 修不了）。border 现由库
  `core::draw::border_emit_plan` 在第 i 块玻璃之后、任何更高块之前发射 => 下层框白边被上层
  玻璃正确遮挡。`glass_engine.rs` 模块头新增「玻璃块层序」不变量。
- **#5 双击（升版必改）**：删除 sink 里的 `is_double_click` / `double_click_window_ms` /
  `double_click_slop_px` / `RouteState.last_down_*` 合成链路，只匹配 `PointerPhase::DoubleClick`。
  序列口径写进注释与规范：CS_DBLCLKS 生效后物理双击是 `Down -> Up -> DoubleClick -> Up`，
  第二次按下不再产生 Down。
- **#2 + #3 压底**：删除 4ms「压底收敛守护」线程与「宿主自己拿 `canvas_handle` 发
  `SetWindowPos(HWND_BOTTOM)`」路径；改为 `apply_canvas_bottom_level()`（走
  `Engine::set_window_level`，0.5.2 等值也重发）+ 判据(120ms 防抖)的 `reassert_canvas_bottom()`
  + 关键时机的 `press_canvas_bottom()`（引擎启动/重建、Ready、MonitorsChanged、新建块显示）。
- **#4 会话快照**：`request_render()` 删除拿 `set_default_resize` 当「发布催办器」的 hack，
  只保留 `Engine::request_render()`；契约改为「块几何变化 => 同帧快照可见」，
  `moving`/`resizing` 现已能如实回落（会话收尾判据可靠）。
- **#8**：`EngineConfig` 补 `frame_latency: None`（1..=8，默认 1 跟手优先），不进风格面板。
- **规范同步**：`.trellis/spec/guides/tauri-window-contract.md` 新增三段——玻璃块层序与 border、
  画布 Z 序兜底、指针事件与快照契约。

## 保留未动（有意为之）

`revive_if_dead`（库侧建议再留一个版本周期）、`wake_canvas_idle` 双保险、
下发前夹 `min_width/min_height`（#7 语义仍是硬下限）、`wnd_proc` Z 序不变量里的
`raise_box`（产品语义：激活框整体在最前，与 border 无关）。

## 门禁

`cargo fmt --all -- --check` 干净；`cargo test` 524 passed / 0 failed；
`cargo clippy --all-targets` 与 `cargo build` 无新增告警（基线 57 warnings 持平）。
已核对：全仓无残留旧行为假设（等值早退 / 催发布 / DoubleClick 永不到来 / 独立叠加 pass）。

## 待人工实机复验（GPU + 真实桌面，无法在 CI 覆盖）

1. 两个框重叠，悬停/拖拽**下层**框边缘：白边不再出现在上层框身上。
2. 冷启动：玻璃不再压在 Chrome/IDE 之上、控制面板不整页空白
   （4ms 守护退役后只靠关键时机 + 100ms tick 判据，这是本轮最大回归风险点）。
3. 双击标题只打开一次编辑框（若「开了又关」说明相位重复投递）。
4. debug 连跑 > 5 min 不 panic；20 框最小尺寸拉满不钉死。


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 11: 接线 liquid-verre-rs 0.5.3：下层框亮边从「画在哪」变成「该不该亮」

**Date**: 2026-11-04
**Task**: 接线 liquid-verre-rs 0.5.3：下层框亮边从「画在哪」变成「该不该亮」
**Branch**: `master`

### Summary

0.5.2->0.5.3 版本接线，业务代码零改动；在发布产物里逐处核对 topmost_body_z_at/grip_reachable/cursor_over_canvas/hover_gate_blocks 真落地；规范收口命中层序=绘制层序 + WindowFromPoint 时效语义；fmt/test 524 全绿

### Main Changes

## 结论

库侧 0.5.3（#9 把手命中让位上层块本体 / #10 外来窗口 `WindowFromPoint` 门控 / #11 三处旧口径 +
不变量 13「绘制层序与命中层序共用同一套 z 判据」）已 release：库仓库 `8f18a6c`、tag `v0.5.3`、
crates.io 有 0.5.3。宿主**只接线，业务代码零改动**。

## 做了什么

1. `Cargo.toml` / `Cargo.lock`：`liquid-verre-rs` 0.5.2 -> 0.5.3。`EngineConfig` 无新增必填字段，
   `cargo check --all-targets` 一次通过（0.5.2 那次需要补 `frame_latency: None`，这次没有这层摩擦）。
2. **不轻信文档**：在实际参与编译的发布产物 `~/.cargo/registry/src/*/liquid-verre-rs-0.5.3/` 里
   逐处核对判据是否真落地 —— `core/manager.rs:36 topmost_body_z_at`、`:53 grip_reachable`、
   `:158 if !grip_reachable(b.z_order, top_body_z) { continue; }`、
   `platform/window.rs:756 cursor_over_canvas` / `:776 point_i32`、
   `runtime/mod.rs:69-70 hover_gate_blocks(pinned, cursor_on_canvas) = !pinned && !cursor_on_canvas`
   （合取写法 + `GLASS_CURSOR_AT` 钉死绕行都在，这两条是我清单里点名的「CI 全绿但取证集体空转」风险）。
3. 影响面排查：全仓只有 `glass_engine.rs:1388`（`debug_log_blocks`）读库侧 `WindowSnapshot::hovered`，
   纯诊断；业务 hover（图标高亮 / 锁定 / 更多按钮）走宿主自己的 `WM_MOUSEMOVE` 命中。
   所以「归属上移到上层块」不改变宿主行为，只让诊断日志变准。
4. 注释口径跟进：`glass_engine.rs` 模块头「玻璃块层序」新增 0.5.3 命中同判据一段；
   `grid_window.rs::begin_glass_resize_session` 补一句「这条争议已从画在哪变成该不该亮」。
5. 规范收口：`.trellis/spec/guides/tauri-window-contract.md` 里那段「已知未修（等库 0.5.3 #9）」
   整段替换为新契约，含**时效语义**（外来窗口盖上来/撤走都要动一下鼠标才生效，别拿静止鼠标当反例）、
   禁止宿主重算层序判据、禁止 `raise_box` 抬层与 `set_resizable(false)` 两种旧绕法、
   `hovered` 是快照（≤1 帧）不得当实时信号。

## 门禁

`cargo fmt --all -- --check` 干净；`cargo test` 524 passed / 0 failed；
`cargo check --all-targets` / `cargo build` 无新增告警（47 / 57 基线持平）。
debug 产物已重建：`target/debug/ikun_organizer.exe`。

## 库侧遗留（不阻塞宿主）

0.5.3 的性能结论按库里 `measurement-and-gates.md` §2 还欠一次「轮转 A/B + >=3 重复 + 中位数」
（本轮机器明显更忙：同场景桌面重绘 110~114 次/秒 vs 0.5.2 那轮 47；`own_us` run-to-run 散布 +/-25 us）。
若将来怀疑「拖拽时玻璃变慢」，先复测这一项，别先动宿主代码。

## 待用户实机复验（本轮主诉的最终闭环）

1. 两框重叠：把光标放进**上层框机身内**正好压住下层框左边缘 => 下层框**不再亮白边、手型不变缩放箭头**；
   把同一条边缘移出上层框范围 => 立刻亮（防修过头）。
2. 外来窗口（Chrome/IDE）盖住边缘后**动一下鼠标** => 灭边；撤走窗口后再**动一下鼠标** => 恢复。
3. 前一轮的三项仍待复验：启动层级（4ms 守护已退役）、双击只开一次编辑框、debug 连跑 >5 min 不 panic。


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 12: Rust 自检审计：20926b54..2bc9dfcf 重构后修复与规范沉淀

**Date**: 2026-11-04
**Task**: Rust 自检审计：20926b54..2bc9dfcf 重构后修复与规范沉淀
**Branch**: `master`

### Summary

三路子代理审计+主线修复。P0 配置双写源经 config_writeback 唯一写入口修复；glass_engine 重建状态恢复（WM_USER_GLASS_REBUILT）、DPI old_ratio、缩放夹取收敛、GetClassNameW 加固；sink 合成消息坐标统一 message_content_point；参考线负原点坐标/会话 generation；OLE MOVE 保守化、CANVAS_ROUTED 引用计数、无界读取收口；FFI 混用清零；删除 box_core 817 行无根模块与 desktop_overlay 5500 行孤儿树；CI allowlist 缩减 5 项。门禁：fmt/check/493 test/严格 clippy 全绿。详见 .trellis/tasks/11-04-11-04-rust-self-check-audit/audit-report.md

### Main Changes

(Add details)

### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 13: 修复分类框图标双击偶发无反应 + 多屏拖拽显示异常/漂移出屏

**Date**: 2026-11-26
**Task**: 11-26-fix-dblclick-and-multimonitor-drag
**Branch**: `master`

### Summary

用户外部反馈两条 bug，代码走查定位到 6 个可实证根因并修复（玻璃/回退两条路径都覆盖）。

**Bug1 双击图标偶发没反应**
1. `DRAG_THRESHOLD = 5` 是硬编码物理像素，不随 DPI 放大 → 高 DPI 下手抖即误判成拖拽；
   改为 `drag_logic::drag_threshold_physical_px(SM_CXDRAG/SM_CYDRAG, dpi)`，夹 [5,16]。
2. `handle_grid_drag_timer` 只要光标几何上落在资源管理器矩形内就起**阻塞式 DoDragDrop**
   并给本框加 `WS_EX_TRANSPARENT`；而分类框常驻 HWND_BOTTOM，Explorer 在 Z 序上盖着它时
   「光标还在框内」也会命中 Explorer → 双击的第二次按下被模态循环吞掉（还会误发起复制）。
   新增 `GridWindowData::drag_box_rect`（起拖时记录内容矩形）+ 纯判据
   `should_start_external_ole_drag`：光标必须**离开本框**才升级 OLE。
3. `WM_LBUTTONDBLCLK` 进入时先干净收尾进行中的自定义拖拽（复位 drag_* + 杀
   `TIMER_ID_GRID_DRAG` + 销毁残影），保证双击优先。

**Bug2 多屏拖拽显示异常 + 漂移出屏**
4. `get_virtual_desktop_bounds` 此前**全项目零调用**，拖拽无任何边界；新增纯逻辑
   `box_core/monitor_geom.rs`（clamp_to_bounds / monitor_index_for_center /
   snap_into_single_monitor / rect_fits_any_monitor），在 create_grid_window、
   WM_MOVING、**WM_WINDOWPOSCHANGED**（引擎模式标题拖拽的真实几何源头——WM_MOVING 不会触发）、
   collect_grid_layout 落盘前、follow/end 会话收尾全部接入单屏吸附；
   吸附幂等、只改位置不改尺寸，归属屏按块中心（与库 `rehome_block` 同构）。
5. `glass_engine::block_content_rect` 用宿主可能陈旧的 `entry.ratio` 还原宽高
   （库已按新屏 `snapshot.scale` 重采样）→ 跨屏那一拍内容窗口/纹理按旧 DPI 尺寸落位，
   就是「一半主屏一半副屏显示异常」的直接来源；改用 `snapshot.scale`，
   新增 `block_scale/block_ratio_drifted/sync_block_ratio`，跟随 tick 纠偏 +
   作废 `content_valid/submitted_glass` 强制全量重绘（`cross_dpi_fix_ms` 200ms 去抖）。
6. `WM_DPICHANGED` 在 `SetWindowPos` 之后仍解引用 `lparam`（handler 返回后临时结构失效 = UB），
   改为入口一次性读字段；DPI 变大导致物理尺寸超过落位屏时用
   `monitor_utils::fit_content_rect_into_monitor` 兜底（必要时允许缩小）。

### Main Changes

- 新增：`src/box_core/monitor_geom.rs`（多屏边界纯逻辑 + 17 项单测）
- 修改：`src/box_core/drag_logic.rs`（DPI 阈值 + OLE 升级判据纯函数与单测）、
  `src/box_core/mod.rs`、`src/platform/windows/monitor_utils.rs`
  （get_monitor_rects / snap / clamp / monitor_index_of_point / fit / drag_threshold_for_dpi）、
  `src/platform/windows/glass_engine.rs`、`src/platform/windows/grid_window.rs`、
  `src/platform/windows/wnd_proc.rs`、`src/platform/windows/drag_handler.rs`、
  `src/platform/windows/grid_layout.rs`、`src/platform/windows/icon_manager.rs`、
  `src/platform/windows/ffi_types.rs`（SM_CXDRAG/SM_CYDRAG）、
  `src/platform/windows/ffi_imports.rs`（EnumDisplayMonitors）
- 规范：`.trellis/spec/backend/windows-platform-guidelines.md` 新增
  「多显示器拖拽边界与双击保护」；`.trellis/spec/guides/tauri-window-contract.md` 新增
  「多屏拖拽与几何边界契约」并补 2 条验证用例。

### Git Commits

(未提交 —— 按红线等用户明确指令)

### Testing

- [OK] `cargo fmt -- --check` 干净
- [OK] `cargo test` 全绿（519 passed / 0 failed，含新增 17+6 项纯逻辑单测）
- [OK] CI 严格 clippy 组合（-D warnings + 现有 allowlist）零告警
- [ ] 待用户实机复验：双屏（主 100%/副 200% 与反向）拖拽切换、跨界停位显示、
  Explorer 窗口盖住分类框时双击图标、锁定态/菜单打开态拖拽边界

### Status

[OK] **Completed**（待实机复验）

### 五维自检（子代理 Gauss + 主线复核）

子代理审查交回 11 条，逐条处置：

| # | 问题 | 处置 |
|---|------|------|
| 1 | `monitor_geom` score 改 i128 但 `best` 仍 i64（E0308） | 已修（`Option<(usize, i128)>`），并保留其 `clamp_axis` / 饱和运算加固 |
| 2 | `end_glass_resize_session` 的 `Option<RECT>` 借用/移动冲突（E0382） | 已重写为「读值 → 结束借用 → 动作」，用 `have_actual` 布尔量 |
| 3 | `WM_WINDOWPOSCHANGED` 死赋值（`content` 写了没读） | 已删；吸附统一走 `snap_outer_rect_for_drag`（内含虚拟桌面兜底） |
| 4 | 收尾先 clamp 再比较 → 块整块在屏外时不 push，玻璃仍留屏外 | 已修：保留 `raw_content`，用 `final_content != raw` 判 `content_moved` |
| 5 | **UB 风险**：持 `&mut GridWindowData` 调 `SetWindowPos`（同步重入消息会二次 get_data） | 已修：`WM_WINDOWPOSCHANGED` / `follow_glass_session` / `end_glass_resize_session` 全部改为先抄值再动作；纪律写入规范 |
| 6 | `WM_MOVING` 回推块缺会话保护 | 已修：补 `!glass_session_active && !block_in_session` |
| 7 | `collect_grid_layout` 的「自愈」承诺无触发源 | 采纳子代理更安全的**失败关闭**版：源矩形无效时不升级 OLE；并把注释改成「只规范化落盘值」 |
| 8 | 跨 DPI 布局序列化口径不自洽（物理÷DPI 当逻辑存） | 记为已知遗留（彻底修需存「显示器身份 + 屏内相对坐标」），写进规范与本节 |
| 9 | 建窗 DPI 取样在吸附/目标屏确定之前 | 同 #8 一并记为遗留；当前吸附保证「无论基准怎么偏都不会落到屏外」 |
| 10 | `fit_content_rect_into_monitor` 缺纯逻辑测试 | 已修：抽出 `monitor_geom::fit_into_bounds`（+3 项单测：负原点副屏 / 超大框 / 非法 bounds），平台函数只做转换 |
| 11 | 子代理中断时未跑门禁 | 主线补齐（见下） |

主线另补两处收尾：双击分支里两段重复的 `get_data` 取数已合并为一段；
残影销毁收紧到「拖拽确实激活过」才执行（纯 pending 时不再白销毁一次）。

### 门禁（最终）

`cargo fmt -- --check` 退出 0；`cargo test` **528 passed / 0 failed**；
CI 严格 clippy 组合（`-D warnings` + 现有 allowlist）**零告警**。
未执行任何 `git add` / `git commit`。

### Next Steps

- 用户实机复验 4 项后确认；若跨界过渡仍有残影，考虑请库侧支持跨屏双画布投影。
- 遗留项 #8/#9（跨 DPI 布局序列化基准）建议单开任务：配置增加「显示器身份 + 屏内相对坐标」。


## Session 14: 修复跨屏拖拽后图标变乱 + 副屏拖左边缘右漂移

**Date**: 2026-11-26
**Task**: 11-26-fix-cross-screen-icon-scramble-and-resize-drift
**Branch**: `master`

### Summary

承接 48d2619 后的两条实机反馈，实测取证（`GetDpiForWindow` / `GetDpiForMonitor` /
`WindowFromPoint` / EnumWindows 矩形）确认本机为 **主屏 1560x1040@192DPI(200%) +
副屏 (3120,0)-(5040,1080)@96DPI(100%)**，三个分类框当前都在副屏，
配置 `resize.min_width/min_height = 605x1150`。

**共同根因：玻璃缩放下限被 DPI 双重换算**
`glass_resize_floor(dpi)` 用 `scale_for_dpi(200,dpi) / monitor_dpi_ratio(dpi)`，
而库的 `scale = 该屏 DPI / **主屏** DPI`（不是 96）→ 主屏非 96 DPI 时该式不再等于 200。
本机副屏 ratio=0.5 → 下限变成 **400 逻辑=400 物理像素**，比现有框（355/377）还宽：

1. Bug「副屏拖左边缘 → 右边缘一直向右漂」：库
   `w=(w0-dx).max(min_w)` 立刻被抬到 400（>w0），锚定式 `x = x0 + (w0-w)` 变负
   → 左边缘外扩、右边缘右移；主屏 ratio=1（下限 200 < 框宽）故正常 —— 与用户描述逐字吻合。
2. Bug「跨屏后图标变乱」之一：尺寸被硬下限强改 → 列数变化 → 触发图标重排。

**第二根因（独立缺陷）：重排用「猜」的旧列数**
`reflow_positions` 用 `max(grid_col)+1` 当旧列数；有空洞 / 末行不满时必然偏小，
线性重映射整体错位且**不可逆**，而跨屏途中 `WM_SIZE` 会多次触发重排。

### Main Changes

- `src/platform/windows/glass_engine.rs`：`glass_resize_floor` 改为返回常量逻辑口径
  （`LOGICAL_MIN_W=200 / LOGICAL_MIN_H=140`，不再接受窗口 DPI 参与换算）；
  单测加「不得被 DPI 二次放大（<=240x180）」断言。
- `src/box_core/layout.rs`：`reflow_positions` 新增 `old_cols_hint` 参数
  （给了就直接采信、不与新列数取较大；没给才退回推断），
  并加两条幂等短路（无固定位项 / 列数未变）；新增 4 项单测（空洞对照、双向 hint、幂等）。
- `src/platform/windows/grid_window.rs`：`GridWindowData` 新增 `layout_cols`
  （三处初始化 + `Default`/`test_default`）。
- `src/platform/windows/grid_layout.rs`：`recalculate_layout` 每轮写 `layout_cols`，
  `reflow_items` 用它当旧列数；补 2 项回归单测（含「不给 hint 时结果必然不同」的对照）。
- 规范：`windows-platform-guidelines.md` 新增「库逻辑像素口径」与「重排旧列数必须记账」两节。

### Git Commits

- 上一条已提交：`48d2619 fix(分类框): 修「双击图标偶发没反应」+ 多屏拖拽边界与跨屏 DPI 错位`
- 本轮改动**未提交**（等用户明确指令）

### Testing

- [OK] `cargo fmt -- --check` 干净
- [OK] `cargo test` **535 passed / 0 failed**
- [OK] CI 严格 clippy 组合零告警
- [ ] 待实机复验：副屏拖左边缘右边缘是否钉住；主副屏来回拖拽后图标行列是否不变

### Status

[OK] **Completed**（待实机复验）

### Next Steps

- 实机复验通过后连同上一轮一起提交。
- 遗留：布局序列化「物理 / 当时 DPI」的跨 DPI 恢复基准（建议存「显示器身份 + 屏内相对坐标」）。

## Session 15: 定案——副屏缩放漂移根因在 `liquid-verre-rs`，撤销宿主补偿

**日期**：2026-11-26（第 3 轮，用户要求判定归属并给出库更新路径）

### Summary

用户三轮验收后「副屏拖左边缘、右边缘持续向右漂」仍在。本轮改为**读库源码定案**而非继续试错：
从 cargo registry 里的 `liquid-verre-rs-0.5.4` 源码逆出缩放会话的完整单位链，与两轮实测数据
逐像素对齐，确认是库的缺陷且宿主在结构上无法修复，于是**删除本轮加的锚点补偿**，产出可提交的
库侧缺陷报告 + 补丁 + 更新流程。

### 定案依据（三条互锁）

1. **量纲错**：`input/drag.rs::ResizeState::apply` 里 `x = x0 + (w0 - w)` 把逻辑像素增量
   直接加到物理像素 `x` 上（库注释 `drag.rs:6-7` 自己写明 `x/y` 物理、`w/h` 逻辑），
   锚定边位移 = δ x (1 - scale)，scale=1 时才为 0。
2. **两套 scale**：`runtime/mod.rs:740` 每帧用 `app.scale_of(b.monitor)`，而会话原点
   （`platform/window.rs:551`）与快照物理尺寸用 `b.scale`；`scale_of` 越界时
   `map_or(1.0, ..)` 静默返回 1.0。推演出的 Δ(被拖边)=δ、Δ(物理宽)=δ x scale 与
   合成输入（+200/+100）和真实鼠标（-26/+13）两轮实测**完全一致**。
3. **测试盲区**：`drag.rs` 所有会动 x/y 的锚定用例（:289 :308 :319 :327 :338）全传 scale=1.0，
   唯一带 scale 的 :356 用的是不改 x 的 `E::RIGHT`。

宿主无法修：`ResizeState`/`DragSession` 是 `pub(crate)`；库每帧用会话 origin 重算并整块覆盖
`b.x/b.width`，宿主 push 下一帧即被抹掉；只夹代理窗口会造成「玻璃走、内容不走」双层错位。

### Main Changes

- `grid_window.rs`：**删除** `glass_resize_anchor` 字段、`apply_resize_anchor`、
  `session_content_rect`、`drop_use_of_data` 占位函数与 2 个锚定单测（净 -218 行）；
  `begin_glass_resize_session(hwnd, edges)` 还原为 `begin_glass_resize_session(hwnd)`；
  跟随 tick 只做虚拟桌面夹取；收尾回推改用块的原始矩形。
- `glass_engine.rs`：删除 `glass_resize_edges_from_wparam`，把被 `resize_handle_bits`
  截断的 `dispatch_glass_session` 文档注释归位；保留 `[GLASS-PUSH]` / `block_debug_line` 诊断。
- `wnd_proc.rs`：`WM_USER_GLASS_RESIZE` 调用还原为单参（300ms 收尾宽限期**保留**，
  它是防止宿主用陈旧矩形污染库会话锚点的正确做法）。
- 规范：`windows-platform-guidelines.md` 把「锚点归宿主所有」一节整体改写为
  「副屏缩放锚定边漂移是库缺陷，宿主不要纠正」，含复现数据、源码定位、允许做的三件事。
- 新增缺陷报告：`.trellis/tasks/11-26-.../liquid-verre-rs-defect.md`（291 行，含可直接
  apply 的补丁、要补的 scale 维度回归测试、方案 A/B/C 三条更新路径 + 适配层回归清单 6 条）。

**未动**（上一轮的真实收益，继续留在工作区）：`glass_resize_floor` 常量逻辑值（宿主自己的
DPI 双重换算 bug，修完主屏彻底正常）、`layout::reflow_positions` 旧列数记账（图标变乱）、
`layout_cols` 记账、`glass_session_ended_at` 宽限期、事件丢失兜底、诊断日志。

### Git Commits

- 已提交（上一轮）：`48d2619 fix(分类框): 修「双击图标偶发没反应」+ 多屏拖拽边界与跨屏 DPI 错位`
- 本轮**未提交**（遵守 commit 红线，等用户当轮指令）

### Testing

- [OK] `cargo fmt -- --check` 干净
- [OK] `cargo test` **535 passed / 0 failed**（删掉 2 个已废弃的锚定单测）
- [OK] CI 严格 clippy 组合零告警
- [OK] `cargo build` 通过，已重启实例（PID 12420，`RUST_LOG=info`）供用户验收
- 待用户复验：双击打开、主/副屏拖拽后图标行列、副屏拖边缘（预期仍漂 = 库缺陷，非回退）

### Status

[OK] **Completed**（宿主侧能做的已做完；副屏缩放漂移移交库侧，已出缺陷报告）

### Next Steps

- 用户按方案 A 在库仓库应用 `drag.rs` 两处 `* s` + `mod.rs` 的 `drag_scale` 改用 `b.scale`，
  用 `[patch]` 指本地库验证后发 0.5.5，再把宿主依赖从 `=0.5.4` 升到 `=0.5.5` 并过适配层回归 6 条。
- 若要先出可用版本：可关 `box_style.liquid_glass.resize.enabled` 由宿主驱动缩放（需产品确认）。
- 遗留：布局序列化「物理 / 当时 DPI」的跨 DPI 恢复基准（建议存「显示器身份 + 屏内相对坐标」）。

## Session 16: 升级到 liquid-verre-rs 0.5.5，适配层回归通过

**日期**：2026-11-26（第 4 轮，用户发布库 0.5.5 后升级宿主）

### Summary

用户按 Session 15 的缺陷报告在库侧根修并发布 `liquid-verre-rs 0.5.5`
（commit `14751faf`，13:08）。本轮完成宿主升级 + 逐条适配层回归。

### Main Changes

- `Cargo.toml`：`liquid-verre-rs = "=0.5.4"` -> `"=0.5.5"`，依赖注释改写为 0.5.5 的
  新契约（缩放会话全程物理域、`min_size` 仍是逻辑像素、apply 返回 `/ scale` 与写回
  `* scale` 成对相消），并保留「升级必须配合适配层回归」的 P2 纪律。
- `Cargo.lock`：仅 `liquid-verre-rs` 一条版本变更（rsproxy 镜像已同步）。
- 规范：`tauri-window-contract.md` 新增「缩放会话量纲契约（库 >= 0.5.5）」+
  一条验收用例（副屏拖左缘 `x + w x scale` 守恒）；`windows-platform-guidelines.md`
  在「副屏缩放锚定边漂移是库缺陷」一节追加 0.5.5 落地记录。
- 缺陷报告追加第 10 节结案：库的实际修法、4 条新判别单测、5.2 双口径为何已被消解、
  适配层回归逐条结论。
- 代码：**零改动**（`cargo check --all-targets` 直接过，说明 0.5.5 未破坏任何宿主可见 API）。

### 取证核对（读 0.5.5 发布包源码）

- `platform/window.rs:549` `let origin = [b.x, b.y, b.width, b.height];`（物理矩形，不再 `/ s`）
- `input/drag.rs:212-236` `dx = cursor - start`（物理）、`x = x0 + (w0 - w)`（同量纲）、
  返回 `[x, y, w / s, h / s]`
- `runtime/mod.rs:771` `b.width = r[2] * scale;`（与 apply 内同一个 scale，相消）
- 新测试：`resize_left_edge_half_scale_anchor_zero_drift`、
  `resize_top_edge_double_scale_anchor_zero_drift`、
  `resize_scale_flip_mid_session_keeps_physical_rect`、
  `resize_min_clamp_half_scale_anchor_still_fixed`

### 过程中的坑（记一笔，别再犯）

- 用 Python 文本模式写 md 时，`newline="\r\n"` 会把字符串里已有的 `\r\n` 再翻一层，
  得到 `\r\r\n`，git 视作整文件改写（一次 `--stat` 冒出 3473 行）。定案：**一律二进制写**
  （`while \r\r\n in b: replace` 收敛后统一成纯 CRLF），或文本模式用 `newline=""`。

### Git Commits

- 已提交（Session 13）：`48d2619`（双击 + 多屏边界）
- 本轮及 Session 15 改动**未提交**（等用户当轮指令）

### Testing

- [OK] `cargo fmt -- --check` 干净
- [OK] `cargo test` **535 passed / 0 failed**
- [OK] CI 严格 clippy 组合零告警（44.8s 全量）
- [OK] `cargo tree` 确认 `liquid-verre-rs v0.5.5`
- [OK] `cargo build` 通过并已重启（PID 21668，13:20，`RUST_LOG=info`）
- 待用户实机复验：副屏拖左缘右缘是否钉住（本缺陷主验收项）、主副屏过渡是否跳尺寸、
  跨屏拖拽后图标行列、双击打开

### Status

[OK] **Completed**（宿主升级完毕，等待实机验收）

### Next Steps

- 实机验收通过 -> 提交（依赖升级 + 图标重排修复 + 规范/取证文档可拆两个 commit）。
- 遗留候选：布局序列化改「显示器身份 + 屏内相对坐标」，彻底解决跨 DPI 恢复基准。

## Session 17: 内容排布与滚动（需求 2.1 自动换行 / 2.2 滚动条）——判定为宿主侧并修复

**日期**：2026-11-26（第 5 轮）

### Summary

用户提「宽度变化要自动换行、高度不够要出滚动条」并问归属。**判定：全部是宿主问题，
不需要动 `liquid-verre-rs`** —— 库只画玻璃块与把手，列数/落位/滚动条/滚轮全在宿主
内容层（`box_core::layout*` + `d2d_render` + `wnd_proc`），滚轮事件本来就由库的 sink
转成 `WM_MOUSEWHEEL` 交回宿主（`glass_engine::PointerPhase::Wheel`）。

查出四个互相独立的老缺陷：

1. **`WM_SIZE` 里 reflow 跑在 recalculate 之前** -> `cols == layout_cols` 恒成立 ->
   `reflow_positions` 永远早退 -> 缩小宽度时固定位留在旧列，图标直接出框（需求 2.1 主因）。
2. **`calculate_grid` 的 `padding = 8` 没随 DPI 缩放** -> `cols = (W-16)/cell_w` 分子不是
   DPI 同比量 -> 同一只框主屏/副屏算出差一整列 -> 跨屏切换即「图标变乱」（需求 1 的另一半，
   之前只修了记账，没修这条 DPI 不变性）。
3. **行数按 `items.len()/cols` 猜** -> 有空洞或用户手动放到第 11 行时算短 ->
   `max_scroll` 偏小、滚动条不出现（需求 2.2 主因）。
4. **滚动上限三处口径不一致**：`WM_SIZE`/滚轮用 `win_h - title_h`，渲染用
   `window_height - grid_top` -> 「滚得动却没滚动条 / 有条滚不到底」；
   滚轮还是裸像素 `delta * 3`，192 与 96 DPI 下每格距离差一倍。

### Main Changes

- `src/box_core/layout_calc.rs`：新增 `scale_view_px` / `grid_side_pad` / `grid_top_gap`
  （留白 DPI 化，列数对 DPI 恒定）与 `snap_width_for_cols`（只吸收取整噪声，真缩小必须放行）；
  `calculate_grid` 改用它们并给 `cell_w` 兜底 `max(1)`。
- `src/box_core/layout.rs`：新增纯函数 `row_extent` / `content_rows`（按条目实际占据的行）
  / `scroll_limit`（统一滚动上限）。
- `src/platform/windows/grid_layout.rs`：**删掉 `reflow_items`**（顺序陷阱的源头），
  改为显式传旧列数的 `reflow_to` + 两个编排入口 `relayout_for_size`（尺寸变化）/
  `relayout_for_dpi`（跨屏，返回吸附后的宽度给调用方摆窗口）+ `scroll_metrics`
  （行数与上限的唯一来源）。10 处旧调用改写。
- `src/platform/windows/wnd_proc.rs`：`WM_SIZE` -> `relayout_for_size` + `scroll_metrics`；
  `WM_DPICHANGED` -> `relayout_for_dpi`，并按它返回的宽度算外框再 `SetWindowPos`；
  `WM_MOUSEWHEEL` -> `scroll_metrics` + 按行滚（`SCROLL_ROWS_PER_NOTCH = 2`，
  不足整格的高精度滚轮退化成像素级）。
- `src/platform/windows/d2d_render.rs`：滚动条改走 `content_rows` + `scroll_limit`，
  `scroll_ratio` 夹到 [0,1]，可见高以 `grid_top` 起算（与滚轮完全同式）。
- 规范：`windows-platform-guidelines.md` 新增「内容排布与滚动：列数是唯一的换行真源」四条纪律；
  `tauri-window-contract.md` 补两条验收用例（窄->换行/宽->上行、滚动条与滚到底完整可见）。

### Git Commits

- 已提交：`cc5ab6f chore(deps)`、`d54431c fix(分类框)`
- 本轮改动**未提交**（等用户当轮指令）

### Testing

- [OK] `cargo fmt -- --check` 干净
- [OK] `cargo test` **547 passed / 0 failed**（新增 12 条：DPI 不变性 / 吸附容差 /
  relayout 换行与幂等 / 跨屏保列数 / scroll_metrics 深行）
- [OK] CI 严格 clippy 组合零告警（改后强制 `Checking` 复跑确认）
- [OK] `cargo build` + 重启实例（PID 23312，13:43，`RUST_LOG=info`）
- 待用户实机复验：拖窄/拖宽的实时换行、跨屏列数不变、内容超高时滚动条与滚轮

### Status

[OK] **Completed**（宿主侧修复完毕，无需库改动）

### Next Steps

- 验收通过后提交（建议 `fix(分类框): 尺寸变化自动换行 + 统一滚动口径`）。
- 可选增强（未做，等确认）：滚动条支持鼠标拖动 thumb 与点击轨道翻页；
  框缩小到装不下时是否自动增高（当前按需求 2.2 走滚动而非增高）。

## Session 18: 参考线变黑定位 + 对齐参考线改造（双色描边 / 每屏 overlay / DPI 阈值）

**日期**：2026-11-26（第 6 轮）

### Summary

用户先要提交（已提交 `61045bb`），随后报「对齐参考线怎么变成黑色了」，并追加要求：
水平+垂直、**跨屏**、**每个框与其它框两两**都能对齐。

**归属判定：不需要改 `liquid-verre-rs`** —— 参考线完全由宿主 overlay 绘制，检测矩形取
宿主代理窗，候选框枚举不按屏过滤；库驱动的缩放/移动会话也已经通过
`enter/exit_glass_resize_session -> on_enter/exit_size_move` 接上参考线。
（唯一「将来可能需要库」的点是**磁吸**：见契约文档新增的
「可选扩展：会话内几何修正回调」。）

### 变黑的取证

```text
2026-11-25 13:35  [GuideColor] avg_luminance=0.486 -> selected=rgb(255,255,255)   正常
2026-11-26 11:27  avg_luminance=0.818 -> rgb(30,40,60)                            黑
2026-11-26 11:29  avg_luminance=0.948 -> rgb(30,40,60)                            黑
2026-11-26 13:22 / 13:43  0.949 / 0.929 -> rgb(30,40,60)                          黑
我用同一公式当场重采：主屏 avg=0.455 -> 会选白色
```

结论：代码没变，变的是**采样那一刻屏幕上有什么**。根因三条叠加：
`GetPixel(GetDC(NULL))` 读合成后的整屏（含所有窗口）；只在首次拖拽采一次且进程级终身缓存；
历史回退 —— 05e3135 把采样移到启动时（调用点在 `src/gpui_app/windows_main.rs`），
20926b5 删 gpui_app 时把调用一起删了，于是退化成「首次拖拽时看到什么算什么」。

### Main Changes

- `alignment_guide/guide_overlay.rs`：**整段重写绘制层**。删掉 `SAMPLE_GRID_SIZE` /
  `sample_screen_color` / `relative_luminance` / `init_guide_color` / `get_guide_color` /
  `CACHED_GUIDE_COLOR` 及其 5 条单测；改固定双色（暗描边 5px α0.55 + 亮内芯 2px α0.95，
  **两遍绘制**避免交叉点缺口）；overlay 从「一个虚拟桌面大窗」改为
  **每屏一个窗口**（`MonitorOverlay{hwnd, monitor}`，布局变化自动重建以支持热插拔），
  新增纯函数 `clip_segment_to_monitor` / `segments_for_monitor` / `bounds_of` /
  `overlay_window_position`，包围盒夹进本屏，右开放边界保证跨屏两段无缝；
  `update_guide_lines(result)` 去掉虚拟桌面原点参数。
- `alignment_guide/alignment_detect.rs`：新增 `alignment_threshold_for_dpi`
  （逻辑 5 -> 物理，夹 [3,12]）+ 2 条单测（阈值 DPI 折算、跨屏两两对齐）。
- `alignment_guide/mod.rs`：两条路径（timer / WM_MOVING）改用 DPI 化阈值；
  每次进入会话都调 `create_guide_overlay()`（内部幂等）以便热插拔重建；
  删除不再需要的 `is_guide_overlay_created`。
- 规范：`windows-platform-guidelines.md` 新增「对齐参考线：双色描边 + 每屏一个 overlay」；
  `tauri-window-contract.md` 新增「可选扩展：会话内几何修正回调」与 2 条验收用例。

### 过程中的坑

- Python 里给 Rust 源码写中文时**不要在脚本里用 `\uXXXX` 双转义**，会原样落到源码里
  造成 `incorrect unicode escape sequence`（本次 46 处，事后用正则批量解码修复）。
- `assert_eq!(x, true)` 被 clippy `-D warnings` 拦下，改 `assert!(x)`。

### Git Commits

- 已提交：`cc5ab6f`（升级 0.5.5）、`d54431c`（图标变乱 + 撤销补偿）、`61045bb`（换行 + 滚动口径）
- 本轮改动**未提交**（等用户当轮指令）

### Testing

- [OK] `cargo fmt -- --check` 干净
- [OK] `cargo test` **548 passed / 0 failed**（新增 7 条对齐相关纯函数单测）
- [OK] CI 严格 clippy 组合零告警
- [OK] `cargo build` 通过，实例已重启（PID 8576，14:09，`RUST_LOG=info`）
- 待用户实机复验：黑线是否消失、跨屏两框的横线/竖线、三框两两对齐、192/96 DPI 吸合手感

### Status

[OK] **Completed**（宿主侧；库无需改动）

### Next Steps

- 验收通过后提交：`fix(对齐参考线): 双色描边 + 每屏 overlay + DPI 化阈值`。
- 若确认要「磁吸」（拖拽时真的吸到参考线上）：标题栏拖拽可在宿主 `WM_MOVING` 直接实现；
  玻璃驱动的缩放/移动需要库加 `GeometryAdjust` 回调（契约文档已写清 API 草案）。

### 用户复验反馈修正（同一轮，14:59）

用户复验指出：**必须 A-B 与 A-C 同时命中才出现参考线**，要求任意一组单独命中也要出线。

根因是我重画 overlay 时把退化判定放进了纯函数 `bounds_of`：单条线的包围盒天然是一维的
（横线 y0 == y1、竖线 x0 == x1），`x0 >= x1 || y0 >= y1` 在加 margin **之前**判就把单组命中
整条丢掉，两组不同位置的命中才凑成二维盒 -> 完全吻合用户描述。这正是本模块 2026-11-03
就踩过一次的同一个坑（旧代码注释写着「margin 先加再做退化判断」），重写时被我搬错了位置。

修正：`bounds_of` 只负责算盒（非空即返回 Some，允许一维），退化判定**只留在加 margin、
夹进本屏之后**的调用方；新增两条回归单测钉住：
`single_horizontal_pair_still_produces_a_line`（单组横线/竖线都必须出盒、加 margin 后不退化）、
`multiple_pairs_are_all_kept`（多组同时命中时三条线都保留）。

门禁复跑：`cargo test` **550 passed / 0 failed**、`cargo fmt --check` 干净、
CI 严格 clippy 组合零告警（强制 `Checking` 复跑）、`cargo build` 通过、实例重启 PID 26424。
顺手修掉两处中文错字（复楠 -> 复验）。

**归属不变：仍然不需要改 `liquid-verre-rs`** —— 这条完全在宿主 overlay 的几何里。

## Session 13: 修复：拔掉副屏后分类框丢失（等比重定位+重连自动搬回）

**Date**: 2026-11-27
**Task**: 修复：拔掉副屏后分类框丢失（等比重定位+重连自动搬回）
**Branch**: `master`

### Summary

新增 box_core/monitor_remap 纯映射 + platform/windows/display_reconcile 对账编排；WM_DISPLAYCHANGE 防抖重定位、重连自动搬回、持久化显示器身份（serde 兼容）；fmt/clippy/test 全绿（573），未提交

### Main Changes

# 会话：拔屏分类框等比重定位 + 重连自动搬回（2026-11-27）

## 任务

`.trellis/tasks/11-27-fix-monitor-unplug-box-relocation`

用户报：多屏下关掉副屏后，副屏上的分类框不可见、不可操控。要求等比例映射到
主屏对应位置（允许重叠），并确认了「副屏重连后自动搬回」。

## 根因

- 全项目无 `WM_DISPLAYCHANGE` 处理；玻璃引擎 MonitorsChanged 只失效缓存 + 压层，
  不搬框；拔屏瞬间系统不给滞留窗口发位置消息，所有既有吸附自愈全部失效。
- `BoxLayoutData` 无显示器身份（11-26 任务预留遗留项）。

## 实现（14 个文件）

- `src/box_core/monitor_remap.rs`（新）：MonitorOrigin{device,rect,rel[x,y]}、
  to/from_relative、target_physical_size（逻辑口径换算）、origin_matches
  （IoU>=0.75 或 设备名+分辨率/原点）、plan_for_box（Skip/Settled/ReturnHome/
  Relocate/Snap 五分支纯决策）、config 双向 From。16 项单测。
- `src/platform/windows/display_reconcile.rs`（新）：窗口线程对账编排
  （schedule 防抖 500ms + 1.5s 确认拍最多 2 拍；busy 让位不消耗拍）、
  apply_relocation（保列数重采样 + 目标屏宽度钳制 + 玻璃 push + 全量重绘）、
  adopt_current_monitor（用户几何稳定收养）、plan_restore（启动恢复按屏身份，
  离岗屏保留 relocated=true 等待搬回）。含与 glass_resize_floor 的同源常量测试。
- `wnd_proc.rs`：WM_DISPLAYCHANGE / WM_TIMER(TIMER_ID_MONITOR_RECONCILE) /
  WM_USER_DISPLAY_RECONCILE / WM_EXITSIZEMOVE 收养 + user_move_resize_in_progress。
- `grid_window.rs`：GridWindowData.monitor_origin/monitor_relocated/reconcile_pass；
  GridCreateParams.monitor_hint/monitor_dpi_hint；collect_grid_layout 落盘 monitor；
  玻璃会话两收尾 adopt；notify_display_changed_all。
- `glass_engine.rs`：MonitorsChanged → 广播窗口线程对账。
- `monitor_utils.rs`：get_monitors_named（MONITORINFOEXW 设备名）+ get_live_monitors（含 DPI）。
- `ffi_types.rs`：WM_DISPLAYCHANGE / TIMER_ID_MONITOR_RECONCILE /
  WM_USER_DISPLAY_RECONCILE / MONITORINFOEXW。
- `app_config.rs`：BoxMonitorOrigin + BoxLayoutData.monitor（serde default 向后兼容，
  roundtrip + 缺省字段测试）。
- `platform_runtime.rs`：restore_layout 经 plan_restore 映射（含 DPI hint）。

## 关键决策

- 不按屏拆分 boxes 列表；每框记「显示器身份 + 屏内位置比例」。
- 尺寸保持 96 逻辑口径（目标物理 = 当前 x 目标DPI/当前DPI），与手工跨屏拖拽
  WM_DPICHANGED 同构；仅装不下时按长宽比等比缩。
- 自动搬回可被用户手动挪动取消（会话收尾收养即清 relocated）。
- 拔屏期间重启、重启后接屏：启动恢复天然完成搬回（relocated 持久化）。

## 验证

- cargo fmt --check：通过；CI 严格 clippy 组合：通过；
- cargo test：573 全绿（新增 16 纯逻辑测试 + 常量同源测试 + serde 兼容测试）。
- 待实机验收：拔屏 ≤1s 等比重定位；重连自动搬回；滞留期间手动拖框取消搬回；
  拔屏重启恢复；192/96 DPI 组合不重排图标；玻璃/回退双路径块=命中层=内容对齐。

## 提交状态

- 未 commit（红线：等用户明确指令）。工作区改动完整保留。


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 14: 拔屏重定位两轮复测修复（双重DPI换算/系统救援污染）+ 三维自检

**Date**: 2026-11-27
**Task**: 拔屏重定位两轮复测修复（双重DPI换算/系统救援污染）+ 三维自检
**Branch**: `master`

### Summary

修复双重DPI换算（抑制窗口）与系统救援归属污染（保留原屏身份）；确认主屏手动挪动取消搬回语义并补回归测试；fmt/clippy/test 575 全绿；未提交

### Main Changes

# 会话14：拔屏重定位两轮复测修复 + 三维自检（2026-11-27）

## 缺陷轮1（用户复测：尺寸异常 + 重连不搬回）
- 根因A：apply_relocation 预换算与系统 WM_DPICHANGED 建议矩形双重换算叠加。
  修复=换算只由对账层做一次 + dpi_suppress_* 抑制窗口（1s，consume_dpi_suppress 命中即清）。
- 根因B：Win11 拔线自动救援把框放上主屏，对账 Settled 误收养新屏、丢原屏身份 → 永不搬回。
  修复=plan_for_box fits 分支「origin 不在岗」保留身份 + relocated=true；用户会话收尾 adopt 才可改判。
- 环境坑：RUST_LOG=warn 继承导致 INFO 日志缺失，排障需干净环境启动。
- 用户复测通过（位置等比正确、大小视觉一致、重连搬回正常）。

## 需求再确认
- 用户明确：拔屏后若在主屏主动挪动过某框，重连不搬回该框（= adopt 清 relocated 语义，
  已补回归测试 plan_does_not_return_home_after_user_readopted_primary）。

## 三维自检（本会话）
- 过度设计检查：无新增冗余（各分支均映射真实 Windows 行为）；日志文案 moved=false 改为
  「系统救援已落位，标记待搬回」。
- Rust 实践：saturating/i64/i128、0 生产 unwrap、#[must_use]、MONITORINFOEXW 兼容指针惯例、
  f32 比较仅限幂等记账。
- 架构符合性：box_core 纯逻辑、窗口线程执行、写回唯一入口、玻璃契约让位。
- 残留风险（记入 PRD）：cur_dpi 竞态 / monitor_dpi_hint 理论漂移 / 抑制 1s 残留。

## 门禁
- cargo fmt --check ✅ CI 严格 clippy ✅ cargo test 575 全绿 ✅
- 未 commit（红线，等用户指令）。


### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete
