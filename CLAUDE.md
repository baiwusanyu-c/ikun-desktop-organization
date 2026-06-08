# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 构建和运行命令

```bash
# 开发模式构建和运行
cargo run

# 发布模式构建（优化编译）
cargo run --release

# 检查代码（快速编译检查）
cargo check

# 运行测试
cargo test

# 运行特定测试
cargo test <test_name>

# 格式化代码
cargo fmt

# 代码检查（linter）
cargo clippy
```

## 架构概述

本项目是一个桌面图标智能分类管理工具，使用 Rust + egui 构建跨平台 GUI 应用。

### 核心模块结构

```
src/
├── app.rs              # egui 应用主入口，实现 eframe::App trait
├── desktop/            # 桌面系统集成（跨平台）
│   ├── mod.rs          # DesktopManagerTrait 接口定义
│   ├── icons.rs        # DesktopIcon 数据结构
│   ├── windows.rs      # Windows 平台实现（cfg(windows)）
│   └── macos.rs        # macOS 平台实现（cfg(target_os = "macos")）
├── classification/     # 智能分类系统
│   ├── mod.rs          # AppClassifier 入口
│   ├── database.rs     # AppCategoryDB 分类数据库（从 YAML 加载）
│   └── matcher.rs      # AppMatcher 匹配算法
├── ui/                 # egui UI 组件
│   ├── box_widget.rs   # ClassificationBoxWidget 分类框组件
│   └── icon_widget.rs  # IconWidget 图标组件
├── config/             # 配置管理
│   ├── layout.rs       # DesktopLayout 布局持久化（JSON 格式）
│   └── settings.rs     # AppSettings 应用设置
└── utils/              # 工具函数
```

### 关键架构设计

**跨平台桌面管理**：通过 `DesktopManagerTrait` trait 定义统一接口，Windows 和 macOS 各自实现。平台特定代码通过 `#[cfg(...)]` 条件编译隔离。

**分类系统**：
- `AppCategoryDB` 从 `data/categories.yaml` 加载分类规则，支持 exe_patterns（可执行文件名匹配）、path_patterns（路径匹配）、keywords（关键词匹配）
- `AppMatcher` 实现匹配算法，支持简单通配符（如 `idea*.exe`）

**UI 层**：使用 egui 即时模式 GUI。`ClassificationBoxWidget` 是可拖拽、可调整尺寸的分类框容器，支持锁定保护。

**配置持久化**：布局配置保存在用户配置目录 `~/.config/ikun-desktop-organization/layout.json`（macOS/Linux）或 `%APPDATA%`（Windows）。

### 数据文件

- `data/categories.yaml`：分类数据库，定义分类规则（游戏、开发工具、办公、媒体等）

### 平台差异

- Windows：使用 `windows-rs` crate 调用 Shell API 读取桌面图标
- macOS：使用 `cocoa`/`objc` crate，目前为实验性支持，返回模拟数据用于 MVP 测试