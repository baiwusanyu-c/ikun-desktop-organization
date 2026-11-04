# 自检审计报告（20926b54..2bc9dfcf）

> 审计方式：3 路并行子代理深度审计（glass_engine / alignment+wnd_proc+drop_target+残留 / 架构与组织）
> + 主线本地取证、修复与集成。基线：cargo test 524 通过起步，收尾 493 通过
>（差值 = 随死代码删除的独占/镜像测试 31 个）。

## 一、结论摘要

| 维度 | 结论 |
|---|---|
| 过度设计 | 已清除：box_core 817 行无根模块、classification 2 个空文件、desktop_overlay 5500 行孤儿源码树、cfg(not(windows)) 假回退、WM_USER_DELETE_BOX 死分支、只写不读的 anchor/presented/SharedGpu.context 等死状态 |
| 逻辑 bug | 已修复 12 项（详见下表），4 项需实机验证（有日志取证钩子） |
| 重构回归 | 发现并修复 3 处「重构后不可用/劣化」：配置双写源丢数据、合成消息坐标偏移、引擎重建状态不恢复；确认 desktop_overlay 是旧特性移除后的残留而非丢失功能 |
| Rust 规范 | fmt/clippy/test 全绿；allowlist 缩减 5 项（dead_code、unused_variables、unused_assignments、unused_mut、clashing_extern_declarations 清零并移出，再犯即卡红 CI）；乱码注释清零 |
| 架构组织 | 分层单向依赖复核成立（平台层零引用 tauri_commands）；新增 config_writeback 依赖倒置桥符合既有回调注入模式 |

## 二、已修复问题清单（按严重级）

### P0
1. **配置双写源互相覆盖（数据丢失级）**：平台线程（拖拽收尾/覆盖层保存/菜单删框）直接
   `AppConfig::load()+save()` 绕开 `SharedTauriRuntime` 内存真源；Tauri 命令随后用陈旧内存
   整文件回写冲掉平台新写入（布局回滚、被删框复活）。
   修复：新增 `platform/windows/config_writeback.rs` 唯一写入口（运行时锁内「改内存+落盘」，
   钩子由 `start_platform_runtime` 注入；未注入时退回磁盘读改写且加载失败绝不写回默认）。
   涉及：drag_handler::save_config、menu_window 删除、ai_cmd 重建布局回写。

### P1（glass_engine / liquid-verre-rs 集成）
2. 引擎重建后内容纹理不恢复（玻璃块只剩底板）→ 新增 `WM_USER_GLASS_REBUILT` 广播，
   各框在自己线程失效 `content_valid` 并全量重推。
3. 引擎重建后 `RouteState.geom=None`（标题拖拽/手型退化）→ 同一消息内重发布 `update_block_geom`。
4. DPI 重建换算 `old_ratio` 读取恒 miss（200%↔100% 尺寸翻倍/减半）→ 改从重建元组取值。
5. 锁定→解锁把未夹取的 605x1150 下限灌回库（框被钉死）→ 三处夹取收敛 `clamped_resize_options`，
   并按窗口真实 DPI 查询（顺带修掉 `ratio*primary` 在非 96 主屏的算错）。
6. `GetClassNameW` 返回值直接切片可越界 panic（发生在持运行时锁的 tick 内）→ 256 缓冲 + min 钳制。

### P1（输入/拖放）
7. **sink 合成消息坐标偏移**：引擎合成的鼠标消息 lparam 为块局部坐标，但
   hover/图标/锁定/更多/双击/右键 handler 全按 client 坐标二次换算（200% DPI 偏移 12px，
   旧修复只覆盖 marquee 一条路径）→ 统一 `message_content_point`。
8. 参考线 overlay 用虚拟桌面局部坐标 `SetWindowPos`：副屏在左/上方时线画错屏 →
   `guide_window_position` 分离窗口坐标与 DIB 坐标（含负原点单测）。
9. OLE `Drop` 未确认成功即结算 `DROPEFFECT_MOVE`（异步入库失败 → 源文件被 Shell 删除）→
   保守 COPY + allowed 交集；框间移动仍由自有消息驱动。**行为变化**：外部只允许 MOVE 的
   拖放会被拒绝而非静默复制。
10. Canvas 拖放转发 `CANVAS_ROUTED` 裸指针无 AddRef/Release（目标框销毁 → UAF）→
    注册表化 + 配对引用计数 + 注销清理；`DragOver` 逐拍重路由（修 A→B 拖拽落错框）；
    `CanvasDropTarget` 恒 1 假计数改为真计数 + `RevokeDragDrop`，`glass_engine::shutdown` 接线。
11. 框间拖拽把 `%TEMP%` 副本路径持久化进目标框（重启后文件失效）→ 优先消费
    `IkunBoxDragData.items` 原始逻辑路径。

### P1（其他）
12. AI 分类重建分类框后不回写布局 → 同锁回写内存 + 落盘。
13. 覆盖层保存失败时用默认配置整份覆盖用户配置 → 唯一写入口天然修复（失败即跳过）。
14. `clashing_extern_declarations`：icon_extract/icon_manager 手写 FFI 与 windows crate
    混用（含 SHGetFileInfoW 返回类型互相矛盾）→ 全部改 windows crate 类型化绑定。
15. `STGMEDIUM` 释放不规范 + `DROPFILES`/`CIDA`/自定义载荷无界读取（恶意数据对象可越界读）
    → `ReleaseStgMedium` RAII + `GlobalSize` 边界校验 + ANSI 显式拒绝。

### P2（修复节选）
- 参考线 bbox margin 二次不对称膨胀；`LAST_HIT`/`LAST_RENDER` 跨会话竞态（generation 化，
  `WM_USER_DESTROY_GUIDE` wparam 携带会话号）；`WM_USER_DRAG_OVER_UPDATE` 负坐标符号扩展；
- `end_box_drag`/`box_drag_active` 整链不可达死状态机、`DRAG_HWND`/`CUSTOM_MOVE`/`GRAB_DX/DY`、
  错误 `KillTimer(TIMER_ID_OVERLAY_HOVER)`、`WM_USER_DELETE_BOX` 死分支、`presented_frames`、
  `SharedGpu.context`、`RenameState` 5 个只写字段、`set_anchor` 只写死状态、
  多选源移除单槽覆盖（改 FIFO 队列 + 逐项降序移除）、`desktop_manager::new`、
  box_core/geometry 双份实现、alignment_detect 旧检测残留、`drag_handler.rs:68` 历史乱码；
- 引擎健壮性：重建单飞 `REBUILDING`、死亡双拍确认（防误杀刚启动引擎）、`attach_box`
  渲染线程未运行时快速失败、FocusHint 同步回调直发（焦点反馈不再拖 100ms）；
- 依赖：liquid-verre-rs 精确钉 `=0.5.3`（适配层依赖库具体行为约定）。

## 三、需实机验证（已在代码留 debug 取证日志）
1. Canvas 回调线程与代理窗 UI 线程套间语义（Lorentz 已把跨线程状态访问改 PostMessage 回 UI 线程；
   `canvas_target_*` 有线程 ID 取证日志）。
2. 引擎重建/复活在真实热插拔下的完整表现（内容重推、Z 序、块位置回读权威性 —— Darwin P2-7/P2-8
   的 snapshot 权威化未动，触发时序需实测）。
3. 外部 Explorer→分类框改为 COPY 后的用户感知（是否需要在 UI 明示「拖入即复制」）。
4. 排序/分组菜单项仍是「只存状态不生效」的历史半成品（sort/group 引擎删除前后行为一致）。

## 四、有意未处理（记录取舍）
- `persist_locked`/`config_cmd` 长临界区（跨磁盘 I/O + 引擎调用）：无死锁环证据，改动风险大于收益，
  已在 spec 记录为已知约束。
- `wnd_proc.rs` ~1900 行、`d2d_render.rs`/`grid_window.rs` >1500 行：渲染/消息巨型文件按规范
  「引擎可放宽」保留，标注下次功能改动前拆分。
- `window_cmd` 的 `check_update` 占位、`close/toggle_window` 无前端调用：属产品契约决策，未擅删。
- glass_engine 适配层 bool 返回（吞 anyhow 细节）：调用方语义就是「是否回退」，全面改 Result
  收益低，边界函数日志已带完整原因。

## 五、spec 沉淀
- `quality-guidelines.md` 新增「重构完成判据」8 条（无根模块清零、双份实现清零、状态恢复清单、
  配置单写入口、坐标空间标注、FFI 一致性、OLE 契约、镜像测试负资产）。
- `tauri-window-contract.md`：配置唯一写入口契约 + 「引擎重建/复活最低恢复清单」+
  sink 合成消息坐标空间契约。
- `directory-structure.md`：更新模块树与「不得复活」清单（含 desktop_overlay 孤儿树处置）。
- CI：build.yml/release.yml allowlist 缩减 5 项并注明「只减不增」。

## 六、门禁终态
- cargo fmt --all：通过
- cargo check --all-targets：0 error
- cargo test：493 passed / 0 failed
- cargo clippy --all-targets（缩减后 allowlist，-D warnings）：通过
- pnpm -C frontend run check：0 errors（前端本轮无改动）
