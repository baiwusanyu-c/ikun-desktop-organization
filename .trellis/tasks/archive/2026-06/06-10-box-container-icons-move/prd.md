# 分类框作为图标容器 - 移动时图标跟随

## Goal

实现类似"腾讯桌面整理"/"Stardock Fences"的核心体验：在桌面上显示半透明分类框，框内装着真实的桌面图标。拖拽分类框时，框内的真实桌面图标跟着一起移动，就像把图标收进了"盒子"里。

## What I already know

### 当前实现的根本问题

| 方面 | 当前实现 | 期望效果 |
|------|---------|---------|
| 窗口 | 800×600 居中窗口 | 全屏透明覆盖层 |
| 分类框 | egui Area 组件（UI 层面） | 桌面上的半透明区域 |
| 图标 | 文字标签显示在 egui 内 | 真实的桌面图标 |
| 移动框 | 移动 egui Area | 移动框 + 真实图标跟随 |
| 桌面交互 | 被 egui 窗口遮挡 | 框外区域正常使用 |

### 腾讯桌面整理的技术原理

通过分析同类软件（腾讯桌面整理、Stardock Fences），核心技术栈为：

1. **透明覆盖窗口** - Win32 `WS_EX_LAYERED` + `UpdateLayeredWindow` 实现逐像素 Alpha 透明
2. **桌面图标控制** - 通过 `SysListView32` 控件的 `LVM_SETITEMPOSITION` 消息移动图标
3. **鼠标事件路由** - `WM_NCHITTEST` 返回 `HTTRANSPARENT` 实现点击穿透
4. **图标限制** - 监控图标拖拽，限制在所属格子范围内

### Windows 桌面图标架构

```
窗口层级:
  Progman (Program Manager)
    └── SHELLDLL_DefView
        └── SysListView32  ← 桌面图标 ListView 控件
```

移动图标的 API 调用链:
1. `FindWindow("Progman")` → 获取 Progman 句柄
2. `FindWindowEx(progman, "SHELLDLL_DefView")` → 获取 DefView
3. `FindWindowEx(defview, "SysListView32")` → 获取 ListView
4. `VirtualAllocEx` → 在 explorer.exe 进程中分配内存
5. `SendMessage(listview, LVM_GETITEMTEXTW, index, remote_ptr)` → 读取图标名
6. `SendMessage(listview, LVM_GETITEMPOSITION, index, remote_ptr)` → 读取位置
7. `SendMessage(listview, LVM_SETITEMPOSITION, index, MAKELPARAM(x,y))` → 设置位置

注意: 跨进程 ListView 操作需要在 explorer.exe 进程空间分配内存（VirtualAllocEx + ReadProcessMemory + WriteProcessMemory）

### Microsoft 推荐的 API

- 官方推荐: `IFolderView::SelectAndPositionItems`（已有 IShellFolder 基础，可扩展）
- 经典方法: `LVM_SETITEMPOSITION`（简单直接，大量第三方软件使用）
- MVP 选择: LVM_SETITEMPOSITION（简单、成熟、文档多）

## Requirements

### MVP 核心功能

* **全屏透明覆盖窗口** - 应用启动后变成全屏透明 overlay，覆盖整个桌面
* **半透明分类框渲染** - 在 overlay 上渲染半透明背景 + 标题栏的分类框
* **真实桌面图标移动** - 拖拽分类框时，框内真实桌面图标跟随移动
* **框外区域点击穿透** - 分类框以外的区域，鼠标点击穿透到桌面（可正常双击图标）
* **一键分类** - 自动分类桌面图标到对应的框内，并移动图标到框的区域
* **布局持久化** - 框和图标的映射关系保存到 JSON

### 交互模型

* **拖拽框的标题栏** → 移动框 + 框内图标跟随
* **框内区域** → 鼠标事件穿透，可双击桌面图标
* **框外区域** → 鼠标事件穿透，正常使用桌面
* **右键标题栏** → 弹出菜单（重命名、删除框、设置样式）

## Acceptance Criteria

* [ ] 应用启动后显示全屏透明覆盖窗口
* [ ] 分类框以半透明背景渲染在桌面上
* [ ] 拖拽分类框标题栏时，框内真实桌面图标跟随移动（延迟 < 100ms）
* [ ] 松开鼠标后图标停留在新位置
* [ ] 框内区域双击桌面图标可以正常打开
* [ ] 框外区域可以正常使用桌面
* [ ] 一键分类后图标移动到对应框的区域
* [ ] 重启应用后框和图标的位置恢复

## Out of Scope

* macOS 支持
* 多显示器支持（MVP 仅支持主显示器）
* 框的边缘拖拽调整尺寸（MVP 使用预设尺寸）
* 框内滚动（MVP 限制每框图标数量）
* 毛玻璃/高级样式
* 开机自启动
* 图标拖拽限制在框内（MVP 不做）
* 右键菜单（MVP 不做）

## Technical Approach

### 整体架构

```
┌──────────────────────────────────────────────┐
│           全屏透明 Overlay 窗口               │
│  (WS_EX_LAYERED + WS_EX_TRANSPARENT)         │
│                                              │
│  ╔══════════╗          ╔══════════╗          │
│  ║ 🎮 游戏   ║          ║ 💻 开发   ║          │
│  ║ 标题栏   ║ ← 可拖拽  ║ 标题栏   ║          │
│  ║ (透明区域)║          ║ (透明区域)║          │
│  ╚══════════╝          ╚══════════╝          │
│    ↑ 真实图标             ↑ 真实图标          │
│    透过透明区域可见        透过透明区域可见     │
│                                              │
│  其他区域: 鼠标点击穿透 → 正常使用桌面         │
└──────────────────────────────────────────────┘
```

### 模块改动

#### 1. `src/desktop/icon_mover.rs` (新建)

核心: SysListView32 控制模块

```rust
pub struct DesktopIconMover {
    listview_hwnd: HWND,
    explorer_process: HANDLE,
}

impl DesktopIconMover {
    /// 查找桌面 SysListView32 句柄
    pub fn find_desktop_listview() -> Result<Self>;
    
    /// 获取所有桌面图标（名称 + 位置 + ListView index）
    pub fn get_icons_with_positions(&self) -> Result<Vec<DesktopIconInfo>>;
    
    /// 通过名称匹配图标 index
    pub fn find_icon_index(&self, name: &str) -> Result<i32>;
    
    /// 移动指定图标到新位置
    pub fn move_icon(&self, index: i32, x: i32, y: i32) -> Result<()>;
    
    /// 批量移动图标（偏移量）
    pub fn move_icons_by_offset(&self, indices: &[i32], dx: i32, dy: i32) -> Result<()>;
}

pub struct DesktopIconInfo {
    pub index: i32,
    pub name: String,
    pub position: (i32, i32),
}
```

关键技术点:
- `VirtualAllocEx` 在 explorer.exe 分配 LVITEM 结构体
- `WriteProcessMemory` 写入 LVITEM
- `SendMessage(LVM_GETITEMTEXTW)` 读取图标名称
- `SendMessage(LVM_GETITEMPOSITION)` 读取位置
- `SendMessage(LVM_SETITEMPOSITION)` 设置位置
- `ReadProcessMemory` 读取结果
- `VirtualFreeEx` 释放内存

#### 2. `src/desktop/mod.rs` (修改)

扩展 `DesktopManagerTrait`:

```rust
pub trait DesktopManagerTrait {
    // ... 现有方法 ...
    
    /// 获取图标信息（带 ListView index）
    fn get_icons_with_indices(&self) -> Result<Vec<DesktopIconInfo>>;
    
    /// 移动图标到新位置
    fn move_icon(&self, index: i32, x: i32, y: i32) -> Result<()>;
    
    /// 批量移动图标（偏移量）
    fn move_icons_by_offset(&self, indices: &[i32], dx: i32, dy: i32) -> Result<()>;
}
```

#### 3. `src/ui/box_widget.rs` (重构)

从 egui Area 改为全屏 overlay 上的绘制:

```rust
pub struct ClassificationBoxWidget {
    // ... 现有字段 ...
    
    /// 框内图标的 ListView indices
    pub icon_indices: Vec<i32>,
    
    /// 框的屏幕矩形（用于命中测试）
    pub screen_rect: egui::Rect,
    
    /// 是否正在拖拽
    pub dragging: bool,
    
    /// 拖拽起始位置
    pub drag_start: Option<egui::Pos2>,
}

impl ClassificationBoxWidget {
    /// 在全屏 overlay 上渲染
    pub fn show_on_overlay(&mut self, ui: &mut egui::Ui);
    
    /// 检查鼠标是否在标题栏上
    pub fn is_mouse_on_title_bar(&self, mouse_pos: egui::Pos2) -> bool;
    
    /// 开始拖拽
    pub fn start_drag(&mut self, mouse_pos: egui::Pos2);
    
    /// 拖拽移动（返回位移 delta）
    pub fn update_drag(&mut self, mouse_pos: egui::Pos2) -> Option<egui::Vec2>;
    
    /// 结束拖拽
    pub fn end_drag(&mut self);
}
```

#### 4. `src/app.rs` (重构)

```rust
pub struct MyApp {
    desktop_manager: DesktopManager,
    classifier: AppClassifier,
    classification_boxes: Vec<ClassificationBoxWidget>,
    layout: DesktopLayout,
    
    /// 桌面图标移动器
    icon_mover: Option<DesktopIconMover>,
    
    /// 当前正在拖拽的框 index
    active_drag_box: Option<usize>,
    
    /// 屏幕尺寸
    screen_size: egui::Vec2,
}

impl MyApp {
    /// 处理图标跟随移动
    fn handle_box_move(&self, box_index: usize, delta: egui::Vec2);
    
    /// 一键分类（包含图标移动）
    fn perform_auto_classification(&mut self);
    
    /// 更新 overlay 的命中测试
    fn update_hit_test(&self, ctx: &egui::Context);
}
```

#### 5. `src/main.rs` (修改)

```rust
let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
        .with_title("Desktop Organizer")
        .with_fullscreen(true)              // 全屏
        .with_decorations(false)             // 无边框
        .with_transparent(true)             // 透明
        .with_always_on_top()               // 置顶
        .with_mouse_passthrough(|_, _, _| { // 条件穿透
            // 仅在非框区域穿透
            // 需要根据当前鼠标位置判断
        }),
    ..Default::default()
};
```

#### 6. `Cargo.toml` (修改)

新增 Windows API features:
```toml
windows = { version = "0.58", features = [
    # ... 现有 ...
    "Win32_System_Memory",       # VirtualAllocEx, ReadProcessMemory
    "Win32_System_Threading",    # OpenProcess, GetWindowThreadProcessId
    "Win32_System_Diagnostics_Debug", # PROCESS_VM_* 权限
]}
```

### 渲染流程

```
每帧:
  1. 透明 overlay 覆盖全屏
  2. 遍历分类框:
     a. 渲染半透明背景矩形
     b. 渲染标题栏（分类名 + 图标）
     c. 标题栏区域可拖拽
     d. 框内区域透明（真实图标透过可见）
  3. 非框区域: 全透明 + 鼠标穿透

拖拽时:
  1. 标题栏捕获鼠标
  2. 计算位移 delta
  3. 更新框位置
  4. 调用 icon_mover.move_icons_by_offset() 移动真实图标
  5. 图标在桌面上实时跟随
```

### 命中测试策略

**MVP 方案: 条件穿透**

利用 egui/eframe 的 `with_mouse_passthrough` 回调:
- 鼠标在某个框的**标题栏**区域 → 不穿透（可以拖拽）
- 鼠标在其他区域 → 穿透（正常使用桌面）

这样：
- 框内区域：穿透，可以双击图标
- 框外区域：穿透，正常使用桌面
- 标题栏：不穿透，可以拖拽框

**限制**: 用户只能通过标题栏拖拽框，不能通过框的其他部分。对于 MVP 足够。

### 关键风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| LVM_SETITEMPOSITION 在 Win11 不工作 | 高 | 启动时检测，失败则降级为仅视觉框 |
| VirtualAllocEx 权限被拒绝 | 高 | 以管理员权限运行，或请求 UAC 提升 |
| egui 透明窗口性能 | 中 | 降低帧率，仅在交互时刷新 |
| 图标名匹配失败 | 中 | 同时使用 PIDL + 名称双重匹配 |
| 自动排列图标冲突 | 中 | 检测并提示用户关闭"自动排列" |

## Implementation Plan

### Step 1: 桌面图标移动能力 (icon_mover.rs)
- 实现 SysListView32 查找
- 实现 VirtualAllocEx 跨进程内存
- 实现 LVM_GETITEMTEXTW / LVM_GETITEMPOSITION / LVM_SETITEMPOSITION
- 单元测试: 能否成功移动一个图标

### Step 2: 全屏透明 overlay
- 修改 main.rs 窗口配置为全屏透明
- 实现点击穿透逻辑
- 验证: overlay 透明且不影响桌面操作

### Step 3: 分类框渲染 + 拖拽
- 重构 box_widget.rs 为 overlay 上的渲染
- 实现标题栏拖拽
- 实现拖拽时的位移计算

### Step 4: 移动联动
- 拖拽框 → 计算 delta → 调用 icon_mover 移动图标
- 验证: 拖框时图标跟随

### Step 5: 一键分类 + 图标移动
- 分类后自动将图标移动到框的区域
- 计算框内图标的布局（网格排列）
- 保存布局

### Step 6: 布局持久化 + 恢复
- 保存框-图标映射关系
- 启动时恢复布局和图标位置

## Decision (ADR-lite)

**Context**: 需要实现"移动框 → 图标跟随"的核心体验，类似腾讯桌面整理/Fences
**Decision**: 
- 全屏透明 overlay（egui viewport）+ SysListView32 LVM_SETITEMPOSITION 移动真实图标
- 条件鼠标穿透: 标题栏可拖拽，其他区域穿透
- MVP 先不做图标拖入/拖出框（仅做框移动带图标）

**Consequences**:
- 优点: 核心体验与腾讯桌面整理一致，技术成熟
- 限制: 需要管理员权限（或 UAC），跨进程内存操作有一定复杂度
- 风险: LVM_SETITEMPOSITION 在未来 Windows 版本中可能受限

## Technical Notes

### 文件清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `src/desktop/icon_mover.rs` | 新建 | SysListView32 图标移动 |
| `src/desktop/mod.rs` | 修改 | 扩展 trait，导出 icon_mover |
| `src/desktop/windows.rs` | 修改 | 集成 icon_mover，新增 move_icon 方法 |
| `src/ui/box_widget.rs` | 重构 | overlay 渲染 + 拖拽 |
| `src/ui/icon_widget.rs` | 简化/删除 | MVP 不需要（图标在桌面上直接可见） |
| `src/app.rs` | 重构 | 全屏 overlay 逻辑 |
| `src/main.rs` | 修改 | 窗口配置 |
| `src/config/layout.rs` | 修改 | 新增图标映射数据 |
| `Cargo.toml` | 修改 | 新增 Windows API features |

### Windows API 参考

- [Old New Thing: IFolderView::SelectAndPositionItems](https://devblogs.microsoft.com/oldnewthing/20211122-00/?p=105948)
- [MJT Net: Set Desktop Icon Positions](https://www.mjtnet.com/blog/2006/01/19/set-desktop-icon-positions/)
- [Stack Overflow: Moving desktop icons using winapi](https://stackoverflow.com/questions/76020228)
- [DesktopFramesPlus (GitHub) - 开源 Fences 替代品](https://github.com/limbo666/DesktopFences)
- [Windows 半透明窗口的三个方案（知乎）](https://zhuanlan.zhihu.com/p/17896885874)
- [Vortice 使用 DirectComposition 显示透明窗口](https://blog.lindexi.com/post/Vortice-%E4%BD%BF%E7%94%A8-DirectComposition-%E6%98%BE%E7%A4%BA%E9%80%8F%E6%98%8E%E7%AA%97%E5%8F%A3.html)
