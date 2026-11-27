# Changelog

本文件是发布报表（release notes）的**唯一来源**：`.github/workflows/release.yml`
会抓取与本次 tag 对应的 `## [vX.Y.Z]` 段落，拼进 GitHub Release 正文。
发版操作见 `RELEASE.md`，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [v0.1.0-beta.9] - 2026-11-27

Windows-only 版本，本轮新增**控制面板「检查更新」与在线自动升级**。

### Added
- **检查更新真实可用**：关于页的「检查更新」此前是写死返回 `No updates available` 的
  空壳，现接入 Tauri 官方 `tauri-plugin-updater`，与 GitHub Releases 的 `beta` 更新通道
  做 semver 比对。断网 / 清单缺失 / 公钥未配置 / 验签失败四类错误返回可区分的中文提示，
  并保留「前往下载页」兜底。
- **一键在线升级，无需手动重装**：点「立即升级」→ 后台下载更新包并完成 minisign 验签 →
  拉起独立安装代理 → 应用自行退出 → NSIS 静默覆盖安装 → 自动重启为新版本。配置
  （`~/.ikun_organizer.json`）与分类框布局完整保留；exe 名与安装路径跨版本不变，因此
  开机自启注册表条目升级后继续有效，无需重新注册。
- **下载进度可见**：新增 `update-download-progress` 事件，关于页显示进度条与百分比
  （按 256KB 间隔节流，不刷爆 IPC）。

### Changed
- 更新源采用固定滚动 tag `beta` 的 `latest.json`，而**不是**官方文档给的
  `releases/latest/download/latest.json`——GitHub 的 "latest" 永不包含 prerelease，
  本项目当前全是 beta，照抄必然 404。
- 发布链路要求 minisign 签名密钥（`TAURI_SIGNING_PRIVATE_KEY` /
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`），CI 自动产出 `*-setup.exe.sig` 与
  `latest.json`，并把清单覆盖更新到 `beta` 通道。公钥仍为占位值时 release job 直接失败，
  避免发布一个老用户验签不过的版本。
- 更新逻辑全部落在 Rust 命令层：前端不引入 updater JS 插件，`capabilities` 也未放开
  `updater:*` / `process:*` 权限（capability 只约束 WebView IPC）。

### Fixed
- **规避「升级后桌面图标停留在隐藏态」**：插件在 Windows 上的 `install()` /
  `download_and_install()` 以 `std::process::exit(0)` 收尾，会跳过本项目退出清理链
  （分类框布局落盘、玻璃引擎 shutdown、托盘注销、桌面注册表清理、**隐藏图标还原**）。
  改为只借用其 `download()`（返回前已完成验签），安装交给独立代理脚本，本进程走既有
  优雅退出路径。顺带绕开上游 `install()` 不返回与 NSIS `Failed to kill <App>` 两个已知缺陷。
- `.gitignore` 补上签名密钥防护：`signer generate -w ~/.tauri/...` 里的 `~` 在 Windows
  不展开，会在仓库根建出字面量 `~` 目录，把私钥落进工作区。

## [v0.1.0-beta.8] - 2026-11-26

Windows-only 版本，本轮集中在**多显示器（主 200% / 副 100% 混 DPI）**与**分类框交互**。

### Fixed
- **分类框图标双击偶发无反应**：手抖几像素即被升级为阻塞式 OLE 拖拽并吞掉双击。现在
  `WM_LBUTTONDBLCLK` 会先收尾进行中的拖拽，拖拽启动阈值按 DPI 折算（SM_CXDRAG），
  源矩形无效时失败关闭。
- **多屏拖拽无边界 / 可整块拖出屏幕 / 半主半副显示异常**：新增单屏吸附 + 虚拟桌面夹取，
  跨屏切换保留列数，布局落盘前同样吸附，重启后不会再出现在屏幕外。
- **主副屏之间拖拽后图标位置变乱**：`glass_resize_floor` 对窗口 DPI 做了二次换算，
  主屏 192 / 副屏 96 的机器上副屏最小尺寸被算成比框还宽；改为常量逻辑值 200x140，
  并为重排补上「上一轮真实列数」记账（有空洞时靠 `max(col)+1` 猜必然错位）。
- **副屏拖左边缘时右缘持续向右漂**：根因在依赖库 `liquid-verre-rs`（缩放会话把逻辑像素
  增量直接加到物理像素坐标，仅 scale=1 时侥幸自洽；且每帧 scale 取自 `scale_of(monitor)`
  与会话原点的 `b.scale` 不同源），已随库 **0.5.5** 根修。宿主同步撤销了无效的锚点回推
  补偿，避免与库逐帧互相拉扯；取证与补丁见 `.trellis/tasks/11-26-*/liquid-verre-rs-defect.md`。
- **尺寸变化不自动换行 / 内容超高没有滚动条**：`WM_SIZE` 里重排跑在列数重算之前导致
  永远早退；网格留白未按 DPI 缩放导致跨屏列数跳变；行数按 `items.len()/cols` 猜导致
  深行图标被算短。现在列数对 DPI 恒定、缩小自动换行、放大自动换回上一行，
  滚动条 / 滚轮 / 收敛三处共用同一口径并按行滚动。
- **对齐参考线变黑、且必须两组同时命中才显示**：旧实现按「桌面亮度」二选色，读的是
  合成后的整屏（含当时打开的所有窗口）且进程级终身缓存，开着白色页面时被选中
  `rgb(30,40,60)` ≈ 纯黑。改为固定「暗外描边 5px + 亮内芯 2px」双色叠加（两遍绘制，
  交叉点无缺口）；overlay 由单个跨屏大窗改为**每屏一个窗口 + 线段按屏裁剪**
  （Per-Monitor-V2 下单窗跨两种 DPI 会被 DWM 拉伸/错位）；包围盒退化判定移到加边距之后，
  单组 A-B 命中也能出线。

### Changed
- 对齐阈值按被拖窗口 DPI 折算（逻辑 5px → 物理，夹 [3,12]）：主屏不再「吸不住」、
  副屏不再「吸太松」。
- 缩放/移动会话期间宿主不再回推块几何（几何真源在库），只做虚拟桌面边界夹取与收尾
  一次性单屏归位；本地会话结束后保留 300ms 反向下发宽限期。

### Dependencies
- `liquid-verre-rs` 0.5.3 → 0.5.4 → **0.5.5**：0.5.4 修 Win10 + Wallpaper Engine 兼容；
  0.5.5 缩放会话全程物理域计算，根修多屏过渡跳尺寸与锚边漂移。升级已过适配层回归 6 条
  （同帧快照可见 / 画布与层级同一次 SetWindowPos / 100ms 唤醒定时器 / 事件形状 /
  逻辑像素口径 / 多屏三场景），宿主零代码破坏。

### Testing
- `cargo test` **550 passed / 0 failed**；`cargo fmt --check` 与 CI 严格 clippy 组合全绿。
- 新增回归单测覆盖：网格列数 DPI 不变性、宽度吸附容差、缩小换行与幂等、跨屏保列数、
  滚动深行上限、跨屏线段裁剪与负原点、多组同时命中、OLE 双击阈值、单屏吸附纯逻辑。
- 规范同步：`.trellis/spec/backend/windows-platform-guidelines.md`（多屏边界、库量纲契约、
  内容排布与滚动、对齐参考线）、`.trellis/spec/guides/tauri-window-contract.md`。

## [v0.1.0-beta.7] - 2026-11-04

### Fixed
- 清理 clippy 1.98 新增的三处 error，恢复 CI 绿灯。

## [v0.1.0-beta.6] - 2026-11-04

### Added
- 桌面宿主 P0/P1：内容层渲染、输入路由、焦点路由（FocusHint）与三套行为取证脚本。

### Fixed
- 接入 `liquid-verre-rs` 0.5.2/0.5.3，修复「下层框白边盖到上层框」的层序问题。
- 对齐参考线：恢复原始检测逻辑、修单线包围盒退化、修双 static 宽限期。
- 分类框：启动层级、hover 闪烁（TrackMouseEvent 假 LEAVE 风暴）、引擎模式每帧白烧一趟
  UpdateLayeredWindow、缩放会话收尾竞态、退出 100% 崩溃、会话期内容层掉帧、
  锁定导致输入全灭、库 debug panic、双击不触发、菜单被抢焦点。

### Performance
- 拖拽改由库逐帧驱动，并附 60Hz 天花板的实测证据。

> 更早版本（v0.0.1-beta1 ~ v0.1.0-beta.5）的说明见
> <https://github.com/baiwusanyu-c/ikun-desktop-organization/releases>。