# Beta 发布：构建脚本评估与 GitHub CI/CD 发布流程

## 目标

项目 Windows 版本初步可用，发布首个 beta 版本。评估并完善现有构建脚本与构建流程，
实现"在 GitHub 上跑 CI/CD、自动打包、产物直接挂到 GitHub Release 供下载"，
本地构建脚本支持一键打包并通过参数指定目标平台（win / mac，mac 预留）。

## 现状评估（调研结论）

现有资产：
- `scripts/build.mjs`：一键构建脚本，含环境检查、前端安装、tauri build、产物收集到 `dist/`，已支持 Win/macOS/Linux 产物命名。
- `.github/workflows/build.yml`：push/PR 触发，Windows + macOS 构建、cargo test、lint，产物仅上传为 workflow artifact。

发现的问题：
1. **无发布流程**：只有 CI，没有 CD。产物只能从 Actions 页面手动下载（保留 90 天），无法在 GitHub Release 页面稳定下载。
2. **构建脚本无显式平台参数**：靠宿主 OS 自动推断，用户无法 `--platform win|mac` 显式指定；mac 未实现却与 win 同等暴露，缺少"预留"语义。
3. **Tauri CLI 版本未固定**：CI 用 `npm install -g @tauri-apps/cli`、本地用 `npx --yes`，版本漂移可能导致构建行为不一致。
4. **CI 无 npm 缓存**：每次全量安装前端依赖。
5. **macOS CI job 阻塞主干**：mac 功能未实现，该 job 失败会让 CI 变红，应降级为非阻塞（预留）。
6. **根目录无 package.json**：无法用 `npm run pack:win` 这类短命令一键打包。
7. **版本号 0.1.0 未标记 beta**：首个对外发布应使用预发布语义版本（0.1.0-beta.1）。
8. **无校验和**：分发产物无 SHA256，beta 分发不规范。

## 需求

- R1: `scripts/build.mjs` 增加 `--platform <win|mac>` 参数（默认当前宿主平台）：
  - `win` 仅允许在 Windows 宿主执行，否则报错（不做跨平台交叉编译）；
  - `mac` 标记为**预留**：macOS 宿主上按现有 mac 路径构建（未验证），非 macOS 宿主给出"暂不支持"的明确错误。
- R2: 根目录新增最小 `package.json`，提供一键命令：`npm run pack:win`、`npm run pack:mac`（预留）、`npm run pack:debug`。
- R3: 新增 `.github/workflows/release.yml`：
  - 触发：推送 `v*` tag 或 workflow_dispatch 手动输入版本号；
  - Windows 构建 → 生成 SHA256 校验和 → 创建 GitHub Release（prerelease）并上传安装包（setup.exe / msi / zip / 校验和文件）；
  - macOS job 以注释/禁用形式预留。
- R4: 完善 `build.yml`：固定 Tauri CLI 主版本、增加 npm 缓存、macOS job 改为 `continue-on-error`（预留非阻塞）。
- R5: 版本号升级到 `0.1.0-beta.1`（Cargo.toml + tauri.conf.json 同步）。
- R6: 更新 `scripts/README.md`；新增 `docs/RELEASE.md` 说明发布流程（如何发新 beta）。

## 验收标准

- [x] `node scripts/build.mjs --platform win --help` 等参数解析正确，非法组合给出清晰中文错误
- [x] 根目录 `npm run pack:win` 等价于调用 build.mjs --platform win
- [x] release.yml 语法正确（actionlint/yaml 校验），tag 推送后能在 GitHub Release 页面下载安装包
- [x] build.yml 保持 CI 能力，mac job 失败不阻塞
- [x] Cargo.toml 与 tauri.conf.json 版本一致为 0.1.0-beta.1
- [x] 文档（scripts/README.md、docs/RELEASE.md）与新流程一致
- [x] 不涉及 Rust 业务代码改动，cargo fmt/clippy/test 保持通过

## 技术笔记

- 产物命名沿用现有约定：`ikun_organizer-v{version}-windows-x64{,-setup}.{exe,msi,zip}`
- Release 使用 `softprops/action-gh-release@v2`，权限 `contents: write`
- 不做代码签名（beta 阶段），Release notes 中说明"未签名，可能被 SmartScreen 提示"
- webviewInstallMode 已是 embedBootstrapper，安装包自带 WebView2 引导，适合分发
- 发布操作：改版本号 → commit → `git tag v0.1.0-beta.1 && git push origin v0.1.0-beta.1`
