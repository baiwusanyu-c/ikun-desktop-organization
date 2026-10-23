# CLAUDE.md - Desktop Organizer 开发指南

> Windows-only 桌面图标分类管理工具。本文档面向开发者与 AI 编码代理；
> 详细规范见 `.trellis/spec/`（本文与其冲突时以 spec 为准）。

## 项目定位（重要）

本项目是**单一 Windows 产品**，不是平台、不做插件体系、不做跨平台移植：

- **禁止**新增自研插件注册表 / trait 插件热插拔机制（历史 PluginRegistry 已于 2026-10 移除）。
- **禁止**新增 `Box<dyn ...>` 式的"为未来平台预留"抽象；平台层直接写 Windows 具体实现
  （历史上为 macOS 预留的 `DesktopManagerTrait`/`PlatformService` 已移除）。
- **禁止**引入第二套 UI 后端（egui/eframe、GPUI 回退路径均已删除，Tauri 是唯一 UI 运行时）。
- `tauri-plugin-opener`/`tauri-plugin-dialog` 是 Tauri 官方插件，与上述"自研插件"无关，正常使用。
- Cargo `[features]` 表已删除；`cargo build` 即完整应用。

## 技术栈

| 组件 | 版本/说明 |
|------|-----------|
| Rust | `rust-toolchain.toml` 锁定（含 clippy/rustfmt） |
| Tauri | 2.x（WebView2；4 个逻辑窗口，见窗口契约） |
| Svelte | 5 + Vite + TypeScript（frontend/） |
| windows-rs | 0.62（Shell/COM、D2D/DWrite、D3D11/DComp/DXGI） |
| 渲染 | 分类框 = 原生分层窗口 + D2D；液态玻璃 = DComp + D3D11 + DXGI 桌面复制；着色器运行时 D3DCompile（`Win32_Graphics_Direct3D_Fxc` feature 不可删） |
| 配置 | 单一 JSON：`~/.ikun_organizer.json`（AppConfig） |

## 架构与运行时所有权

```text
main.rs
 ├─ 单例信号协议：无实例时正常启动；已有实例时 --create-box/--open-panel/--ai-classify/--quit
 │   仅写 %TEMP% 信号文件后立即退出（运行实例轮询消费）
 └─ run_tauri_app()
      ├─ Builder::manage(SharedTauriRuntime = Arc<Mutex<TauriRuntime>>)   ← 必须先于窗口创建
      ├─ invoke_handler![...]（tauri_commands 全量命令注册处）
      └─ setup: start_platform_runtime(app_handle, runtime)  ← 平台轮询线程
```

- **唯一配置模型**：所有模块经 `AppContext.config`（`src/context.rs`）读写配置；
  Tauri 命令、轮询线程、托盘回调共享同一 `SharedTauriRuntime`。命令层禁止另建配置副本。
- **开机自启**：`sync_auto_start` 在 initialize 最先执行；配置加载失败（`config_loaded=false`）
  时必须跳过，防止按默认值删除用户已开启的条目；写注册表唯一入口 `utils::platform::set_auto_start`。
- **真退出**：设置 `quit_requested` → 轮询线程保存布局 → 主线程销毁分类框窗口 → 清理托盘/注册表 →
  恢复桌面图标 → `app.exit(0)`（附 3s 看门狗强退：僵尸进程会长期占用 DXGI 桌面复制，
  导致后续实例玻璃捕获全部失败，历史上出过事故）。

## 模块地图（src/）

| 模块 | 职责 | 边界 |
|------|------|------|
| `main.rs` | 入口、信号文件协议、Per-Monitor DPI V2/OLE 初始化、panic hook、Tauri builder | 不放业务逻辑 |
| `context.rs` | AppContext：AppConfig + config_loaded | 无 trait 对象；状态字段仅保留实际被读取的 |
| `tauri_commands/` | IPC 适配层：`config_cmd/box_cmd/window_cmd/app_cmd/ai_cmd/log_cmd` 按域拆分；`platform_runtime.rs` = 轮询线程 + 原生编排 | 只做参数校验 + 锁运行时 + 调业务/平台 |
| `box_core/` | 平台无关纯逻辑：box_model/box_manager/layout_calc/view_engine/sort_engine/group_engine/interaction；自原 shared/ 并入：drag_logic、drag_state、geometry、layout、selection（全局多选）、types | 禁止出现 windows/HWND；由 platform 层调用 |
| `classification/` | 规则匹配（matcher）与 AiClassifier（OpenAI 兼容 API） | 与渲染无关 |
| `config/` | AppConfig 结构与 load/save/export/import。字段即外部契约：删除/改名需同步前端 `tauri-api.ts` 类型 | |
| `platform/windows/` | 全部原生实现（约 47 文件）。分区：窗口（grid_window/menu_window/control_panel_window/wnd_proc/zorder/occlusion）、渲染（d2d_render/render_devices/liquid_glass/icon_bitmap）、图标（icon_manager/icon_hider/icon_extract/app_icon）、拖拽（drag_drop/drag_handler/drag_visual/drag_image/ole_dnd/drop_target/multi_select→re-export box_core::selection）、Shell 与系统集成（shell_menu/desktop_manager/desktop_utils/rename_editor/global_hotkey/sys_tray/alignment_guide/desktop_overlay/*）、监控（monitor_utils/mem_probe）、ffi_imports/ffi_types（手写 FFI；新代码优先用 windows crate）、types（共享数据结构） | 不存在 trait 抽象层；调用方直接用 `crate::platform::xxx`（薄 re-export） |
| `utils/` | logger（文件轮转，中文日志）、dpi、platform（注册表自启，Windows-only）、string_utils、alloc_probe（全局分配计数：把 Rust 堆从 D2D/D3D11/驱动私有分配中切出来做内存归因） | |

`frontend/src`：`App.svelte`（hash 路由 4 页）+ `lib/pages/`（ControlPanel、CreateBox、EditBox、AiClassify、AiSettings、AboutTab、StyleSettings）+ `lib/api/tauri-api.ts`（invoke 封装 + 配置类型定义）+ `lib/stores/` + `lib/styles/glass.css`（毛玻璃视觉基线）。

## 窗口契约（详见 `.trellis/spec/guides/tauri-window-contract.md`）

| label | 用途 | 策略 |
|-------|------|------|
| control-panel | 控制面板（主题/样式/关于/AI 设置） | 持久窗口，CloseRequested → hide |
| create-box / edit-box | 创建/编辑分类框弹窗 | 持久窗口，alwaysOnTop，CloseRequested → hide |
| ai-classify | AI 分类进度弹窗 | 同上 |

- 窗口全部 `decorations: false, transparent: true, shadow: false`，标题栏由 Svelte 自绘
  （`TitleBar.svelte` + `lib/window-lifecycle.ts`）。
- `tauri.conf.json` 中窗口 `create: false`：先声明规格，命令层按需 `WebviewWindowBuilder` 创建/复用；
  show 前按目标显示器物理像素居中（多屏）。

## 渲染要点（详见 `.trellis/spec/backend/windows-platform-guidelines.md`）

1. 分类框 = `WS_EX_LAYERED` + `UpdateLayeredWindow`（per-pixel alpha）。alpha=0 像素会
   **透传鼠标**并触发 WM_MOUSELEAVE——背景 alpha 动态管理是历史 bug 高发区，改动前必读分层窗口指南。
2. 液态玻璃：DComp 视觉 + D3D11 + DXGI desktop duplication 抓真实桌面帧，D2D 合成折射；
   单帧捕获失败沿用上一成功帧（禁止黑色/白色回退）；锁屏与显示器切换要处理 ACCESS_LOST，
   避免重连循环白烧 CPU。
3. 图标来自 Shell 提取（icon_extract/icon_manager）；icon_hider 把原生桌面图标藏入
   managed_icons"仓库"，**退出必须恢复**，注册表菜单项残留必须清理。
4. 中文名：DWrite 渲染 + rename_editor 独立线程 EDIT（支持 IME）；日志消息一律中文。

## 构建与质量门禁

```bash
cargo check --all-targets        # 快速验证
cargo test                       # 全量单测（门禁）
cargo fmt -- --check             # 门禁
# clippy 门禁 = cargo clippy --all-targets -- -D warnings + CI allowlist（见 build.yml）
pnpm -C frontend run check    # svelte-check + tsc（门禁）
pnpm run pack:win               # 发布构建（预发布仅 NSIS；stable 加 MSI）→ target/dist/
```

- 版本号存在于 **4 处**（Cargo.toml / tauri.conf.json / package.json / Cargo.lock），
  由 `pnpm run release:beta:push`（scripts/release.mjs）统一驱动，勿手改。
- 预发布版本（含 `-beta.N`）只产 NSIS：MSI/WiX 要求纯数字版本，build.mjs 自动降级。
- build.rs：tauri-build 嵌入应用图标（唯一来源 `tauri/icons/icon.ico`，根目录不再有 logo.ico 副本）+ embed-resource 嵌入 DPI manifest（MSVC 走 rc.exe、GNU 走 windres）。
- **并行测试须知**：多模块共享进程级全局状态（selection 全局注册表、注册表 Run 键等）。
  涉及全局态的测试必须自带串行锁——参考 `box_core/selection.rs` 的 `serial()` 与
  `utils::platform` 的 `test_support::registry_test_lock`。

## 硬性规范（摘要，全文见 spec）

- 错误处理：`anyhow::Result` + `.context()`；启动路径优雅降级；UI/消息线程禁止 panic。
- 依赖方向：Svelte → tauri_commands → TauriRuntime/AppContext → box_core/classification/config → platform/windows → utils；**禁止反向依赖**。
- 公开 API 与模块必须有（中文）文档注释；数据结构实现 `Default`。
- 单文件 ≤ 1500 行（渲染引擎类可适度放宽）；可独立测试的纯逻辑优先拆小模块并配单测。
- 高频路径（每帧/每条消息）禁止打日志。
- 所有中文内容 UTF-8（无 BOM）；PowerShell 写文件遵循 AGENTS.md 编码规则（优先 Python/Node 写入）。

## 文档索引

- `.trellis/spec/index.md` — 规范总入口（架构/目录/常用命令）
- `.trellis/spec/backend/` — directory-structure / error-handling / quality-guidelines /
  logging-guidelines / windows-platform-guidelines / icon-specification / database-guidelines
- `.trellis/spec/frontend/` — tauri-svelte-architecture
- `.trellis/spec/guides/` — tauri-window-contract / windows-layered-window-guide /
  windows-shell-api-guide / code-reuse-thinking-guide / cross-layer-thinking-guide
- `RELEASE.md` — 发布流水线；`scripts/README.md` — 打包脚本用法
