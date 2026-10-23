# Desktop Organizer（ikun 桌面整理）

![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-blue)
![Language](https://img.shields.io/badge/language-Rust-orange)
![UI](https://img.shields.io/badge/UI-Tauri%202%20%2B%20Svelte%205-green)

> 桌面图标智能分类管理工具：**分类框直接渲染在桌面之上**，不是窗口式文件夹。

## 产品形态

Desktop Organizer 是一个 **Windows 原生**桌面整理工具。分类框是贴在桌面图标层的
原生 Win32 分层窗口，使用 Direct2D/DirectWrite 渲染，支持（Win11 风格）液态玻璃效果；
控制面板与创建/编辑弹窗使用 Tauri 2 + Svelte 5（WebView2）。

### 核心功能

- ✅ **一键 AI 智能分类** - 兼容 OpenAI 格式 API，按文件类型/名称自动归类
- ✅ **分类框管理** - 类似手机文件夹的桌面分类容器，创建/编辑/删除/锁定
- ✅ **自由拖拽** - 文件从资源管理器拖入框、框内排序、框间移动、Shift 插入
- ✅ **对齐辅助线** - 多框移动时的智能对齐引导
- ✅ **液态玻璃样式** - 折射/模糊/饱和度/弹性形变可调（DXGI 桌面捕获 + D3D11）
- ✅ **桌面接管** - 隐藏原生桌面图标入"仓库"，右击菜单注入（创建框/一键分类/面板/退出）
- ✅ **多显示器 & DPI** - Per-Monitor V2 感知，物理像素布局
- ✅ **系统托盘 + 全局热键 + 开机自启**
- ✅ **布局记忆** - 自动保存分类框布局，重启后恢复

## 技术栈

| 层 | 技术 |
|----|------|
| 控制面板 / 弹窗 UI | Tauri 2 + Svelte 5 + TypeScript |
| 分类框 / 桌面覆盖层 | Win32 分层窗口 + Direct2D/DirectWrite + DWM |
| 液态玻璃 | DirectComposition + D3D11 + DXGI 桌面复制，着色器运行时 D3DCompile |
| Shell 集成 | windows-rs（Shell/COM：托盘、右键菜单、OLE 拖放、图标提取） |
| 业务核心 | Rust（box_core 纯逻辑 + classification AI 分类 + config JSON） |

## 快速开始

### 环境要求

- Windows 10/11（本项目**仅支持 Windows**，不做跨平台承诺）
- Rust（rustup 按 `rust-toolchain.toml` 自动锁定版本；需 MSVC 工具链）
- Node.js ≥ 22.13 + pnpm 11（pnpm 11 依赖新版 Node 的 `node:sqlite`；推荐与 CI 一致的 24 LTS，pnpm 可用 `corepack enable` 启用）
- WebView2 Runtime（安装包 embedBootstrapper 自动处理）

### 构建与运行

```bash
# 1. 构建前端（首次必须）
# tauri.conf.json 的 frontendDist 指向 frontend/dist，
# 该目录不存在时 tauri::generate_context!() 会在编译期直接 panic。
cd frontend
pnpm install
pnpm run build
cd ..

# 2. 开发运行（Tauri 是唯一 UI 路径）
cargo run

# 调试日志
RUST_LOG=debug cargo run
```

> 也可以直接用 `pnpm run pack:fast`（debug 模式、跳过安装包）一键完成前端 + Rust 构建。
> 若报错 `The \`frontendDist\` configuration is set to "frontend/dist" but this path doesn't exist`，
> 说明前端还没构建，回到第 1 步即可。

### 常用命令

| 命令 | 说明 |
|------|------|
| `cargo run` | 开发运行 |
| `cargo test` | 全量单元测试（590+） |
| `cargo fmt` / `cargo clippy` | 格式化 / Lint |
| `pnpm -C frontend run check` | 前端类型检查（svelte-check + tsc） |
| `pnpm run pack:win` | 发布构建，产出 NSIS/MSI 安装包到 `target/dist/` |
| `pnpm run pack:fast` | 快速打包（debug、无安装包） |
| `pnpm run release:beta:push` | 版本号 +1、打 tag、推送触发发布流水线 |

## 项目结构

```text
.
├── src/
│   ├── main.rs            # 入口：单例信号文件协议 + DPI/OLE 初始化 + Tauri 启动
│   ├── context.rs         # AppContext（唯一配置模型，共享运行时注入）
│   ├── tauri_commands/    # IPC 适配层：config/box/window/app/ai/log 命令 + 平台轮询运行时
│   ├── box_core/          # 平台无关业务逻辑：布局/排序/视图/分组引擎 + 拖拽/多选/几何纯逻辑
│   ├── classification/    # 分类规则匹配 + AI 分类器
│   ├── config/            # 统一 JSON 配置（~/.ikun_organizer.json）
│   ├── platform/
│   │   └── windows/       # Windows 实现主体（窗口/渲染/Shell/托盘/拖拽/覆盖层，47 文件）
│   └── utils/             # 日志、DPI、注册表自启、分配探针
├── frontend/              # Svelte 5 + Vite（控制面板/创建/编辑/AI 分类窗口）
├── scripts/               # build.mjs 打包、release.mjs 发版
├── build/                 # build.rs（清单嵌入 + tauri-build）
├── tauri/icons/           # 应用图标唯一来源（icon.ico：exe 嵌入与安装包共用）
├── capabilities/          # Tauri 窗口权限清单（tauri-build 约定扫描路径，不可移动）
├── tauri.conf.json        # Tauri 配置（4 个 WebView 窗口 + NSIS/MSI 打包）
├── .trellis/spec/         # 开发规范（backend/frontend/guides）
└── .github/workflows/     # Windows CI + Release 流水线
```

## 发布

见 [RELEASE.md](RELEASE.md)。打 tag（`v*`）触发 Release 流水线：质量门禁（fmt/clippy/test）
通过后构建 NSIS（预发布版）或 NSIS + MSI（正式版），发布 GitHub Release 并附 SHA256SUMS。

## 已知边界

- 仅支持 Windows 10/11；没有 macOS/Linux 移植计划。
- Beta 安装包未做代码签名，首次运行会有 SmartScreen 提示。
- 管理员权限（高完整性级别）运行时，UIPI 会阻止从资源管理器拖入。
