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
