# 控制面板「检查更新」与在线自动升级（tauri-plugin-updater）

## 目标

把关于页里写死返回 `No updates available` 的假 `check_update` 换成基于 **Tauri 官方 `tauri-plugin-updater`** 的真实链路：检查到新版本 -> 用户点一下 -> 后台下载并 minisign 验签 -> 静默覆盖安装 -> 自动重启到新版本。全程不需要用户手动重装，配置与分类框数据不丢。

## 现状（取证）

| 位置 | 现状 |
| --- | --- |
| `src/tauri_commands/window_cmd.rs:522-527` | `check_update` 是 stub，硬编码 `Ok("No updates available")` |
| `frontend/src/lib/pages/AboutTab.svelte:19-26,37-38` | 按钮 + 检查中态 + 结果行 UI 已完成，只等后端真数据 |
| `frontend/src/lib/api/tauri-api.ts:315-317` | `checkUpdate(): Promise<string>`，无法表达「有新版/下载中/待重启」 |
| `Cargo.toml` / `tauri.conf.json` / `capabilities/default.json` | 无 updater 插件、无 `plugins.updater`、无 `createUpdaterArtifacts` |
| `.github/workflows/release.yml:234-243` | 只上传 `target/dist/*`，无 updater artifact、无 `latest.json` |
| `scripts/build.mjs:276-301` | 产物统一重命名，会丢掉 updater 的 `setup.exe.sig` |

## 插件源码取证（实现前必读，决定了整体架构）

| 事实 | 出处 | 对方案的影响 |
| --- | --- | --- |
| `download()` 返回前已 `verify()`（minisign） | `updater.rs:881-893` | 只用 `download()` 就能拿到「已验签字节」，安全性不打折 |
| Windows `install_inner` 结尾 `std::process::exit(0)` | `updater.rs:1050` | **禁用官方 install 路径**（见 R3） |
| `install_inner` 在 spawn 前只调 `on_before_exit`（默认 `cleanup_before_exit`） | `updater.rs:1017-1020`、`lib.rs:121-123` | 该钩子无法承载本项目清理链，不能指望它 |
| NSIS 参数 `/P` + `/UPDATE` + `/R` + `/ARGS` | `updater.rs:1053-1083`、`config.rs:41-58` | 代理脚本沿用 `/P /UPDATE`，刻意去掉 `/R` |
| v2 原生 updater 不打包 zip：NSIS `setup.exe` 本体直接由 CLI 签名并生成同名 `.sig` | `tauri-bundler/bundle.rs:226-235`、`tauri-cli/bundle.rs:226-313` | 代理直接执行 setup.exe；CI 收集 setup.exe + `.sig` |
| `Builder` 无 endpoint 方法，endpoints 只能来自配置 | `lib.rs:156-268` | `tauri.conf.json` 必须写 `plugins.updater.endpoints` |
| 本项目退出清理链跑在 poller 线程，末尾 `force_finish()` | `platform_runtime.rs:666-742` | 置 `quit_requested` 即触发完整清理，无需另写清理逻辑 |

## 需求

- **R1 真实检查更新**：调 updater `check()`，与本地 `CARGO_PKG_VERSION` 做 semver 比对（`0.1.0-beta.9 > 0.1.0-beta.8` 交给插件，不自研比较）。
- **R2 一键在线升级**：下载已验签的更新包并静默覆盖安装，安装完自动重启为新版本。
- **R3（硬约束）绝不调用插件的 `install()` / `download_and_install()`**：它们在 Windows 上直接 `std::process::exit(0)`，会**完全跳过** `platform_runtime.rs:666-742` 的退出清理——分类框布局落盘、玻璃引擎 shutdown、托盘注销、桌面注册表清理、**隐藏图标还原**全部不执行，后果是升级后用户桌面图标停留在隐藏态；且 `std::process::exit` 会撞上 `platform_runtime.rs:717-720` 记录的 WebView2/D2D COM 对撞崩溃。改为：`update.download()` 拿验签字节 -> 落临时文件 -> spawn detached 安装代理 -> 置 `quit_requested` 走既有完整清理与 `force_finish`。
- **R4 beta 滚动更新通道**：GitHub 的 `releases/latest` 永不包含 prerelease，本项目当前全是 beta，官方文档的 `releases/latest/download/latest.json` 会 404。改用固定滚动 tag `beta`：`.../releases/download/beta/latest.json`，应用端 URL 一次写死不再变。
- **R5 CI 产出更新物料**：签名构建（`TAURI_SIGNING_PRIVATE_KEY`）-> 收集 NSIS `setup.exe` + 同名 `.sig` -> 生成 `latest.json`（version / notes / pub_date / platforms）-> 发布到版本 tag，并把 `latest.json` 覆盖更新到 `beta` tag 的 release。
- **R6 结构化 IPC 契约**：Rust 为协议源，`tauri-api.ts` 同步 TS 类型；字段沿用项目既有 snake_case 直传风格（不加 rename_all）。
- **R7 前端状态机**：`idle / checking / upToDate / available / downloading / installing / error / notConfigured`，下载进度可见，升级前明确提示「应用会退出并重启」。
- **R8 未配置公钥时必须可诊断**：`pubkey` 为占位值时直接返回「更新功能未配置签名公钥」，不得让插件抛 base64 解析错误糊在前端。
- **R9 保留兜底**：失败路径提供「打开 Release 页手动下载」，复用既有 `open_url`（http/https 白名单，`window_cmd.rs:505-511`）。
- **R10 文档**：`RELEASE.md` 增补 updater 发布流程、密钥生成/备份、私钥丢失恢复预案。

## 技术决策

- **D1 更新逻辑全部放 Rust 命令层**：前端不引入 `@tauri-apps/plugin-updater` JS 依赖，`capabilities/default.json` 不放开 `updater:default`。capability 只约束 WebView IPC，Rust 侧 `app.updater()` 不受限制；既符合「IPC 统一走 `tauri-api.ts`」契约，又把权限面压到最小。
- **D2 新增 `src/tauri_commands/update_cmd.rs`**：承载 `check_update` / `install_update`；从 `window_cmd.rs` 移除 stub，`window_cmd` 只留窗口与外链职责。
- **D3 命令返回 `Result<T, String>`**：可恢复错误转中文 `String`（`error-handling.md` Pattern 2），前端 `catch` 展示；禁止 `unwrap/expect`。
- **D4 安装时序：下载 -> 落盘 -> 外部代理 -> 优雅退出**。代理为 detached `powershell -WindowStyle Hidden`（附 `CREATE_NO_WINDOW`），职责：轮询等待本进程 PID 消失 -> 固定等 800ms 让 OS 释放 exe 句柄 -> `Start-Process setup.exe -ArgumentList '/P','/UPDATE' -Wait` -> `finally` 中 **无条件** `Start-Process` 当前 exe（安装失败也要把应用拉回来）。不带 `/R`，重启交给脚本，避免安装器与脚本双重拉起。
- **D5 `pubkey` 先落占位常量** `REPLACE_WITH_MINISIGN_PUBLIC_KEY`，Rust 侧读 `app.config().plugins` 校验占位值并短路返回友好错误（R8）。真实公钥由用户提供后替换。
- **D6 `bundle.createUpdaterArtifacts: true`**，`targets` 保持 `["nsis","msi"]`：updater 载体由 NSIS 产出；MSI 仅 stable 用，不参与更新通道。
- **D7 `latest.json` 的 `url` 指向真实 asset 名**：`build.mjs` 必须成对收集 NSIS `setup.exe` 与其同名 `setup.exe.sig`；url 与 asset 名严格一致，`signature` 取 `.sig` 文本内容。
- **D8 进度回传走 Tauri 事件**（`update-download-progress`），复用既有 `core:event:allow-listen`，不新增权限；事件节流（每 256KB 或 20% 变化才发一次），遵守「禁止高频日志/高频事件」。
- **D9 依赖漂移必须显式化**：实测 HEAD -> 工作区为 tauri 2.11.5 -> 2.12.7、tauri-build/codegen/macros 2.6.3 -> 2.7.6、tauri-plugin 2.6.3 -> 2.7.4、tauri-runtime 2.11.3 -> 2.12.4、tauri-runtime-wry 2.11.4 -> 2.12.5、tauri-utils 2.9.3 -> 2.10.4、tao 0.35.3 -> 0.37.3、wry 0.55.1 -> 0.57.4、window-vibrancy 0.6.0 -> 0.8.1、tray-icon 0.24.2 -> 0.25.1、webview2-com 0.38.2 -> 0.39.1；新增 tauri-plugin-updater 2.14.1、zip 9.8.2、minisign-verify 0.3.0。reqwest 0.12.28 与 0.13.4 在 HEAD 已并存，本次 0.13.4 成为 updater 直接依赖；tauri-plugin-fs 2.5.1 在 HEAD 已存在；`liquid-verre-rs` 仍锁定为 `=0.5.5`（Cargo.toml 未改）。`Cargo.lock` +738/-251。这些包直接关系 WebView/窗口行为，而项目有大量 DPI、多屏、玻璃相关精细修复，**必须单独跑一次安装版回归**，不允许当作免费搭车改动隐瞒。

## 验收标准

- [ ] `check_update` 不再返回硬编码字符串；无新版返回 `upToDate`，有新版返回新版本号 + notes。
- [ ] 断网 / 清单 404 / 验签失败三类情况各自返回可区分的中文错误，前端不白屏、不卡死。
- [ ] `pubkey` 为占位值时点击检查更新得到「未配置签名公钥」提示，而不是插件内部错误。
- [ ] 点「下载并安装」：进度推进 -> 日志出现完整清理链（`Cleanup complete`）且无 `TerminateProcess` 之前的崩溃 -> 安装器静默覆盖 -> 自动重启为新版本。
- [ ] **升级后桌面图标全部可见且位置正确**（R3 的核心验证点）。
- [ ] 升级后 `~/.ikun_organizer.json` 保留、分类框位置与尺寸保留；HKCU `Run` 自启键仍指向有效 exe。
- [ ] 代理脚本在解压/安装失败时依然把应用拉回前台，不出现「升级后应用消失」。
- [ ] CI 在 beta 版本下产出 updater `setup.exe` + `.sig` + `latest.json`，且 `.../releases/download/beta/latest.json` 可匿名 GET。
- [ ] `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings`（allowlist 内）/ `cargo test` 通过。
- [ ] `pnpm -C frontend run check` 通过；页面不出现裸 `invoke()`。
- [ ] `capabilities/default.json` 未新增 updater/process 权限。

## 风险与对策

| 风险 | 影响 | 对策 |
| --- | --- | --- |
| 官方 `install()` 用 `std::process::exit(0)` 跳过退出清理 | 桌面图标残留隐藏态 + COM 对撞崩溃 | D4 彻底绕开官方 install，只用 `download()` 拿验签字节 |
| 代理脚本自身失败（落盘/权限/路径） | 用户觉得「升级把应用弄没了」 | 脚本 `-ErrorAction SilentlyContinue` + 安装后无条件拉起 exe；临时目录保留便于取证 |
| 旧进程刚退出、exe 句柄未释放 | 安装器报文件占用 | 代理在 PID 消失后固定等 800ms 再动手 |
| NSIS 杀不掉旧进程（tauri#16277/#6984） | 安装器弹「进程仍在运行」 | 旧进程先干净退出，安装器根本不需要 kill |
| 未配公钥即发布 | 老用户收到更新但验签失败 | R8 占位短路 + CI 增加「pubkey 仍为占位则 release job 失败」守卫 |
| 私钥丢失 | 无法再给已装机用户推更新 | 只能发一版带新公钥的安装包让用户手动重装；写入 `RELEASE.md` 并要求离线备份 |
| GitHub 匿名 API 限流 60/h | 检查更新偶发失败 | 走静态 `latest.json` asset（非 API），不受限流影响 |
| tauri 依赖栈小版本漂移 | WebView/窗口/DPI 行为回归 | D9 显式记录 + 要求安装版回归，不隐瞒 |
| beta 通道 URL 写死 | 将来上 stable 需改配置 | 预留 `beta` / `stable` 双通道命名约定，本次只实装 `beta` |

## 不在本次范围

- Authenticode 代码签名（SmartScreen 弹窗照旧，`RELEASE.md` 已声明 beta 未签名）。
- 便携版（zip）就地替换自身 exe 的真·免安装器升级。
- 强制更新 / 后台静默自动检查（本次仅手动点击触发）。

## 需要用户提供

1. `tauri signer generate` 生成的**公钥**（写入 `tauri.conf.json`）。
2. GitHub 仓库 Secrets：`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。
