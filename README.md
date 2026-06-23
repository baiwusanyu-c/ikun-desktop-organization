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

## 架构演进方案

### 背景

当前每个分类框是独立的 `WS_EX_LAYERED` 分层窗口，删除一个分类框时 `DestroyWindow` 会触发 Windows DWM 重新合成桌面，导致其他分层窗口短暂失效（一帧闪烁）。这是 Windows DWM 对分层窗口的固有行为，无法通过 API 调用顺序绕过。

### 当前架构：N 个分层窗口

```
分类框 A → WS_EX_LAYERED 窗口 → UpdateLayeredWindow
分类框 B → WS_EX_LAYERED 窗口 → UpdateLayeredWindow
分类框 C → WS_EX_LAYERED 窗口 → UpdateLayeredWindow
```

- **优势**：事件由系统分发（各窗口独立 wnd_proc），拖拽/缩放/Z-order 由 Windows 原生管理
- **劣势**：N 套 D2D 资源（factory/target/DIB），删除时闪烁无法消除

### 方案一：单窗口渲染（推荐）

所有分类框渲染到一个分层窗口内，"删除"只是移除内部数据，不销毁窗口。

```
单一分层窗口 → UpdateLayeredWindow
  ├── 分类框 A（内部区域）
  ├── 分类框 B（内部区域）
  └── 分类框 C（内部区域）
```

**渲染管线**（不变）：
```
D2D DC RenderTarget → BindDC(GDI mem_dc) → EndDraw → UpdateLayeredWindow
```

| 维度 | 对比 |
|------|------|
| D2D 资源 | N 套 → 1 套 |
| UpdateLayeredWindow | N 次 → 1 次 |
| 删除闪烁 | ✅ 彻底消除（无窗口销毁） |
| 内存 | N × (位图 + COM) → 1 个大位图 |
| 事件分发 | 需自行 hit-test 路由到对应分类框 |
| 拖拽/缩放 | 需自行实现内部拖拽逻辑 |
| 改造成本 | 中等（渲染代码几乎不变，需重写事件分发） |

### 方案二：DirectComposition

每个分类框作为 DComp visual tree 中的一个节点，DWM 原生增量合成。

```
DCompositionTarget (绑定 HWND)
  ├── IDCompositionVisual (分类框 A)
  ├── IDCompositionVisual (分类框 B)
  └── IDCompositionVisual (分类框 C)
```

**渲染管线**（需重写）：
```
D3D11 Device → DXGI Surface → D2D DXGI RenderTarget → EndDraw
     ↓
DCompositionDevice → DCompositionVisual → SetContent(surface) → Commit
```

| 维度 | 对比 |
|------|------|
| D2D 资源 | N 套 → 共享 D3D11 设备 + 每个 visual 一个 DXGI surface |
| UpdateLayeredWindow | N 次 → 不需要（DComp 原生合成） |
| 删除闪烁 | ✅ 彻底消除（移除 visual，DWM 增量更新） |
| 内存 | 最省（GPU 纹理共享） |
| 动画 | DComp 原生支持（淡入淡出、弹性动画） |
| 事件分发 | 需自行 hit-test |
| 拖拽/缩放 | 需自行实现 |
| 改造成本 | 高（需引入 D3D11 + DXGI，重写整个渲染管线） |

### 性能总结

```
DirectComposition > 单窗口渲染 > 当前架构（N 个分层窗口）
```

### 建议

**单窗口渲染**投入产出比最高 — 渲染代码几乎不变（仍用 `ID2D1DCRenderTarget` + `UpdateLayeredWindow`），只需合并窗口 + 重写事件分发。可作为中期优化目标。

**DirectComposition** 性能最优且支持原生动画，但需要将渲染管线从 GDI DC 模式切换到 DXGI Surface 模式（引入 D3D11 依赖），适合作为长期架构方向。

---

**开发时间**: 2026-06-08
**版本**: v0.1.0 MVP