# Desktop Organizer (ikun-desktop-organization)

> 桌面图标智能分类管理工具

![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS-blue)
![Language](https://img.shields.io/badge/language-Rust-orange)
![GUI](https://img.shields.io/badge/GUI-egui-green)

## 项目简介

Desktop Organizer 是一个基于 Rust + egui 的桌面图标智能分类管理应用，帮助你整理混乱的桌面，让图标井井有条。

### 核心功能

- ✅ **一键智能分类** - 自动识别应用类型并分类
- ✅ **分类框管理** - 类似手机文件夹的分类容器
- ✅ **自由布局** - 分类框可拖拽摆放任意位置
- ✅ **尺寸调整** - 拖拽边缘调整分类框大小
- ✅ **样式自定义** - 圆角、背景色、毛玻璃效果
- ✅ **布局记忆** - 自动保存布局，重启后恢复
- ✅ **锁定保护** - 锁定分类框防止误操作

### 支持平台

| 平台 | 状态 | 说明 |
|------|------|------|
| Windows 10/11 | ✅ 核心支持 | 使用 Windows Shell API |
| macOS | ⚠️ 实验性支持 | 需要特殊权限 |

## 项目结构

```
ikun-desktop-organization/
├── Cargo.toml              # 项目配置
├── src/
│   ├── main.rs             # 应用入口
│   ├── app.rs              # egui 应用实现
│   ├── desktop/            # 桌面管理（平台区分）
│   │   ├── mod.rs
│   │   ├── icons.rs        # 图标数据结构
│   │   ├── windows.rs      # Windows 实现
│   │   └── macos.rs        # macOS 实现
│   ├── classification/     # 分类系统
│   │   ├── mod.rs
│   │   ├── database.rs     # 分类数据库
│   │   ├── matcher.rs      # 匹配算法
│   ├── ui/                 # UI 组件
│   │   ├── mod.rs
│   │   ├── box_widget.rs   # 分类框组件
│   │   ├── icon_widget.rs  # 图标组件
│   ├── config/             # 配置管理
│   │   ├── mod.rs
│   │   ├── layout.rs       # 布局管理
│   │   ├── settings.rs     # 应用设置
│   └── utils/              # 工具函数
│       ├── mod.rs
│       └── platform.rs     # 平台工具
├── data/
│   └── categories.yaml     # 分类数据库
└── README.md
```

## 技术栈

- **Rust 1.82+** - 编程语言
- **egui + eframe** - 即时模式 GUI 框架
- **serde** - 数据序列化
- **windows-rs** - Windows API 绑定
- **cocoa/objc** - macOS API 绑定

## 快速开始

### 安装依赖

```bash
# 确保 Rust 已安装
rustc --version

# 克隆项目
cd /Users/baiwusanyu/RustroverProjects/ikun-desktop-organization

# 构建项目
cargo build
```

### 运行应用

```bash
# 开发模式
cargo run

# 发布模式
cargo run --release
```

## 开发进度

### MVP 阶段（当前）

- [x] 项目结构搭建
- [x] 基础 egui 窗口
- [x] 分类数据库设计
- [x] 分类匹配算法
- [ ] Windows Shell API 集成
- [ ] macOS 桌面集成
- [ ] 分类框 UI 完善
- [ ] 拖拽功能实现
- [ ] 布局保存/恢复

### 后续计划

- [ ] 开机自启功能
- [ ] 多显示器支持
- [ ] 高级样式系统
- [ ] MCP + Agent 智能分类
- [ ] 安装包制作

## 贡献指南

欢迎提交 Issue 和 Pull Request！

## 许可证

MIT License

---

**开发时间**: 2026-06-08
**版本**: v0.1.0 MVP