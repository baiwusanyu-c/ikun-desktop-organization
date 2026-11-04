# Directory Structure

> 项目代码组织方式（v3.0 架构：Windows-only，单一 Tauri 运行时）

---

## Overview

模块化架构：每个功能领域一个独立模块；平台实现全部收敛在 `platform/windows/`。

**核心设计原则**：
- **单一运行路径**：`cargo build` 即完整应用；无 Cargo features、无插件注册表、无 legacy UI 回退
- **共享运行时注入**：Tauri Commands、平台轮询线程共享唯一 `AppContext`（`SharedTauriRuntime`）
- **具体实现优先**：禁止为假设性跨平台引入 trait 抽象；调用方直接使用 `crate::platform::xxx`
- **业务解耦**：`box_core/` 为平台无关纯逻辑（含自原 shared/ 并入的拖拽/多选/几何），由 platform 层调用
- **单向依赖**：Svelte → tauri_commands → AppContext → box_core/classification/config → platform/windows → utils

---

## Directory Layout

```text
src/
├── main.rs                 # 入口：信号文件单例协议、DPI V2/OLE 初始化、panic hook、Tauri builder
├── context.rs              # AppContext（AppConfig + config_loaded；不持有平台对象）
│
├── tauri_commands/         # IPC 适配层 + 原生编排
│   ├── mod.rs              #   命令聚合导出
│   ├── platform_runtime.rs #   TauriRuntime、平台轮询线程、托盘/热键/信号分发、退出清理序列
│   ├── config_cmd.rs       #   配置读写/主题/样式/AI 配置命令 + apply_runtime_config
│   ├── box_cmd.rs          #   分类框 CRUD 与格子窗口创建参数
│   ├── window_cmd.rs       #   持久 WebView 窗口生命周期（show/hide/toggle/复用）
│   ├── app_cmd.rs          #   应用信息/退出/屏幕查询
│   ├── ai_cmd.rs           #   AI 分类命令（异步执行 + 结果落框）
│   └── log_cmd.rs          #   前端日志转发
│
├── box_core/               # 平台无关业务逻辑（禁止 use windows / HWND）
│   ├── layout_calc.rs      #   网格布局计算（calculate_grid/hit_test）
│   ├── view_engine.rs      #   视图参数（get_view_params）
│   ├── drag_logic.rs       #   拖拽纯逻辑（compute_drag_over_state/apply_shift_insert）
│   ├── layout.rs           #   布局重排（reflow_positions）
│   └── selection.rs        #   全局多选注册表（跨容器选择；测试带 serial() 串行锁）
│
├── classification/         # 分类系统
│   └── ai_classifier.rs    #   OpenAI 兼容 AI 分类器
│
├── config/
│   └── app_config.rs       # 统一 JSON 配置（~/.ikun_organizer.json）；字段为外部契约
│
├── platform/
│   ├── mod.rs              # 薄壳：SystemTrayAction 枚举 + pub use windows::*
│   └── windows/            # 全部 Windows 原生实现（分区如下）
│       ├── types.rs        #   共享数据结构（DesktopIcon/IconData/...）
│       ├── 窗口            #   grid_window / menu_window / control_panel_window / wnd_proc /
│       │                   #   zorder / occlusion / grid_layout
│       ├── 渲染            #   d2d_render / render_devices / glass_engine（liquid-verre-rs 适配层）/ icon_bitmap
│       ├── 图标与桌面      #   desktop_manager / desktop_utils / icon_manager / icon_hider /
│       │                   #   icon_extract / app_icon
│       ├── 拖拽            #   drag_drop / drag_handler / drag_visual / drag_image / ole_dnd /
│       │                   #   drop_target / multi_select（re-export box_core::selection）
│       ├── Shell/系统      #   shell_menu / global_hotkey / sys_tray / rename_editor /
│       │                   #   ffi_imports / ffi_types
│       ├── 对齐辅助线      #   alignment_guide/
│       └── 监控            #   monitor_utils / mem_probe
│
└── utils/
    ├── logger.rs           # 文件轮转日志（中文日志规范见 logging-guidelines）
    ├── dpi.rs              # DPI 换算
    ├── platform.rs         # 开机自启注册表（唯一写入入口 + test_support 串行锁）
    ├── alloc_probe.rs      # 全局分配计数探针（Rust 堆归因）
    └── string_utils.rs     # 字符串工具

frontend/                   # Svelte 5 + Vite（详见 spec/frontend/）
build/                      # build.rs（tauri-build + embed-resource 清单嵌入）+ app.manifest
scripts/                    # build.mjs（Windows 打包）/ release.mjs（发版）
tauri.conf.json             # 4 个 WebView 窗口 + NSIS/MSI bundle
```

已删除模块（2026-10 重构）：`src/plugin/`（自研插件体系）、`src/gpui_app/` + `src/ui/` + `src/app.rs`
（legacy UI 回退）、`src/ipc/`（空占位）、`src/shared/`（并入 box_core）、`platform` trait 抽象层、
macOS 依赖与构建面。**不得复活**。
2026-11-04 自检审计追加删除：`box_core/{box_model,box_manager,sort_engine,group_engine,interaction,
geometry,drag_state,types}.rs`（817 行「无根模块」：零生产调用或与平台侧实现重复）、
`classification/{matcher,database}.rs`（0 字节残留）、`liquid_glass.rs`/`occlusion.rs`（被
liquid-verre-rs 取代）、**整个 `platform/windows/desktop_overlay/` 孤儿源码树**（9 文件约 5500 行，
b040c6c「移除覆盖区域窗口，使用原生桌面渲染未分类图标」后未删净：无任何 `mod` 声明、不在编译图内，
仅靠 `WM_USER_OVERLAY_*`/`TIMER_ID_OVERLAY_*` 常量与 `selection::ContainerType::Overlay` 留了幻觉引用）。
`Config.overlay`（icon_positions 等）作为磁盘兼容字段保留，仅读侧（icon_hider 恢复逻辑）仍在使用。**新增模块必须有生产调用方；抽取到 box_core 的算法必须同步迁移调用方，
禁止平台侧留双份实现**（用 `cargo clippy --all-targets` 无 allowlist 临时跑一遍即可暴露死代码）。

---

## Module Rules

### 新代码放哪里

| 代码类型 | 位置 | 理由 |
|----------|------|------|
| 可单测的纯计算（布局/命中/重排/选择） | `box_core/` | 平台无关，保持可测试 |
| Win32 句柄、消息循环、COM、渲染 | `platform/windows/` | 平台实现唯一归属地 |
| 前端可调用的新能力 | `tauri_commands/` 加命令 + mod.rs 注册 | IPC 适配层职责 |
| 配置字段 | `config/app_config.rs` + 前端 tauri-api.ts 类型同步 | 外部契约 |

### 命名与结构规范

- 文件/函数 snake_case；模块目录即领域名；一个领域一个 mod.rs 导出公共 API
- 平台实现文件内按"窗口/渲染/图标/拖拽/Shell/覆盖层/监控"分区，新增文件归入对应前缀
  （grid_* / overlay_* / icon_* / drag_* / ffi_*）
- 手写 FFI 只允许在 ffi_imports.rs / ffi_types.rs；新代码优先 `windows` crate
- 单文件 ≤ 1500 行（渲染引擎可放宽）；超过则按职责拆子模块

### 测试布局

- 单元测试与被测模块同文件 `#[cfg(test)]`（项目惯例）
- 共享进程级全局状态（selection 注册表、注册表 Run 键）的测试**必须**获取串行锁后再断言
