# 桌面图标智能管理 — macOS 实现计划

## 核心思路

**透明窗口覆盖方式**（类似 Stardock Fences 原理，适配 macOS）：

1. 使用 AppleScript 隐藏 Finder 桌面图标（`tell application "Finder" to set visible of every item of desktop to false`）
2. 创建全屏透明 NSWindow（NSBorderlessWindowMask），覆盖桌面区域
3. 透明窗口使用 Core Graphics/Metal 渲染桌面图标
4. 处理图标交互（双击、右键、拖拽）
5. 分类框窗口 z-order 高于覆盖窗口

```
Z-order（从上到下）:
  应用窗口 (NSWindow.level = .normal)
  分类框窗口 (NSWindow.level = .floating)
  覆盖窗口 (NSWindow.level = .desktopIcon + 1)
  Finder 桌面图标 (被 AppleScript 隐藏)
  桌面壁纸 (NSWindow.level = .desktopWindow)
```

---

## macOS 与 Windows 关键差异

| 特性 | Windows | macOS |
|------|---------|-------|
| 桌面图标管理 | SysListView32 (Explorer) | Finder Desktop View |
| 隐藏原生图标 | `ShowWindow(SW_HIDE)` | AppleScript `set visible to false` |
| 窗口分层 | HWND Z-order | NSWindow.level |
| 渲染技术 | Direct2D / GDI | Core Graphics / Metal |
| 透明窗口 | WS_EX_LAYERED + UpdateLayeredWindow | NSWindow.backgroundColor = .clear |
| 鼠标穿透 | WM_NCHITTEST → HTTRANSPARENT | ignoresMouseEvents / hitTest |
| 右键菜单 | IContextMenu (Shell) | NSMenu + NSWorkspace |
| 文件启动 | ShellExecuteW | NSWorkspace.shared.open |
| 图标提取 | SHGetFileInfo + ExtractIconEx | NSWorkspace.icon(forFile:) |
| 拖拽 | OLE DragDrop / 自定义 | NSDraggingSource / NSPasteboard |

---

## 模块结构

```
src/platform/macos/
├── desktop_overlay/               # 新增模块
│   ├── mod.rs                     # 模块导出
│   ├── overlay_window.rs          # NSWindow 创建/销毁/生命周期
│   ├── overlay_render.rs          # Core Graphics 渲染（图标网格）
│   ├── overlay_event.rs           # NSView 事件处理（鼠标/键盘/拖拽）
│   ├── icon_layout.rs             # 桌面图标布局算法（复用 Windows 逻辑）
│   └── overlay_drag.rs            # 拖拽发起（覆盖窗口 → 分类框）
│
├── icon_hider.rs                  # AppleScript 隐藏/恢复桌面图标
├── grid_window.rs                 # 分类框 NSWindow（复用 egui viewport）
├── icon_extract.rs                # NSWorkspace.icon 提取
├── context_menu.rs                # NSMenu 右键菜单
├── drag_source.rs                 # NSDraggingSource 拖拽源
├── drop_target.rs                 # NSDraggingDestination 拖放目标
├── desktop_manager.rs             # 桌面图标枚举（已存在，需增强）
└── ffi_imports.rs                 # Core Foundation / Core Graphics FFI
```

---

## 阶段 1：覆盖窗口基础

### 目标
创建一个全屏透明 NSWindow，覆盖桌面工作区域（不含 Dock 和菜单栏），放在桌面图标之上。

### 新增文件：`desktop_overlay/overlay_window.rs`

```rust
use cocoa::appkit::{NSWindow, NSWindowStyleMask, NSBackingStoreBuffered};
use cocoa::foundation::{NSRect, NSPoint, NSSize, NSString};
use objc::{class, msg_send, sel, sel_impl};

/// 覆盖窗口创建参数
pub struct OverlayCreateParams {
    pub screen: NSScreen,          // 目标屏幕
    pub frame: NSRect,             // 工作区域（不含 Dock/菜单栏）
    pub scale_factor: f64,         // Retina 缩放因子
}

/// 创建桌面覆盖窗口
pub fn create_overlay_window(params: OverlayCreateParams) -> Result<id>;

/// 销毁覆盖窗口
pub fn destroy_overlay_window(window: id);

/// 获取所有覆盖窗口（多显示器支持）
pub fn get_overlay_windows() -> Vec<id>;
```

**关键实现细节**：
- **窗口样式**：`NSBorderlessWindowMask | NSNonactivatingPanel`
- **透明背景**：`window.setBackgroundColor(NSColor.clearColor())`
- **不透明内容**：`window.setOpaque(false)`
- **Z-order**：`window.setLevel(NSWindowLevel::DesktopIcon + 1)`（约等于 20）
- **不抢焦点**：`window.setIgnoresMouseEvents(false)` 但 `canBecomeKeyWindow` 返回 `NO`
- **点击穿透空白区域**：使用 `ignoresMouseEvents` 动态切换，或在 `hitTest` 中返回 `nil`
- **Retina 支持**：`window.backingScaleFactor()` 获取缩放因子
- **屏幕工作区域**：`screen.visibleFrame`（自动排除 Dock 和菜单栏）

### 新增文件：`desktop_overlay/overlay_render.rs`

```rust
use core_graphics::context::CGContext;
use core_graphics::image::CGImage;

/// 渲染覆盖窗口（Core Graphics → NSImage → NSImageView）
pub fn render_overlay(window: id);
```

**渲染管线**：
1. 创建 `CGContext`（位图上下文，32-bit RGBA）
2. 绘制图标网格（见阶段 2）
3. 转换为 `CGImage` → `NSImage`
4. 设置到覆盖窗口的 `NSImageView`
5. `window.display()` 刷新

**替代方案（更高效）**：使用 `CALayer` 或 `Metal` 实现硬件加速渲染，但 Core Graphics 对 MVP 足够。

### 可复用代码
| 来源 | 复用内容 |
|------|---------|
| `grid_window.rs` (Windows) | 窗口创建模式、DPI 适配逻辑 |
| `icon_layout.rs` (Windows) | 布局算法（需转换为 CG 坐标系） |
| `config/app_config.rs` | OverlayConfig 配置结构 |

---

## 阶段 1.5：分类框渲染（对标 Windows 版本）

### 目标
在 macOS 覆盖窗口上渲染分类框，外观和功能对标 Windows 版本 `d2d_render.rs` 的 `render_layered_window` 实现。

### 当前实现问题（与 Windows 版本的差异）

#### 1. 圆角背景
- **Windows**: 使用 `D2D1_ROUNDED_RECT` + `FillRoundedRectangle`，圆角半径 16px，背景色 `RGBA(1,1,1,38/255)`
- **macOS 当前**: 使用 bezier curve 绘制矩形，但圆角实现可能不完整（curve_to_point 参数需要验证）
- **修复**: 使用 `CGPathAddArcToPoint` 或 `NSBezierPath` 的 `bezierPathWithRoundedRect` 绘制正确的圆角

#### 2. 标题栏分隔线
- **Windows**: 在标题栏底部绘制 1px 分隔线，颜色 `RGBA(100,100,100,100/255)`
- **macOS 当前**: 已实现，但需要验证颜色和位置是否正确

#### 3. 标题文字渲染
- **Windows**: 使用 DirectWrite，15pt Microsoft YaHei UI 粗体，白色，4 方向阴影描边（上下左右各 1px 偏移）
- **macOS 当前**: 使用 `NSAttributedString` + `NSShadow`，但 NSShadow 只支持单方向阴影
- **修复**: 需要手动绘制 4 次文字（偏移 1px 上下左右）来模拟描边效果

#### 4. 锁定按钮（🔒/🔓）
- **Windows**: 标题栏右侧显示锁定/解锁按钮，14pt Segoe UI Emoji，hover 时显示半透明白色背景（`RGBA(255,255,255,40/255)`）
- **macOS 当前**: **未实现**
- **实现**:
  - 在标题栏右侧绘制 🔒 或 🔓 emoji（根据 `locked` 状态）
  - 添加 hover 检测（需要鼠标事件跟踪）
  - Hover 时绘制圆角背景高亮

#### 5. 更多按钮（⋮）
- **Windows**: 锁定按钮右侧显示三个竖点按钮（U+22EE），16pt Segoe UI Symbol 粗体，hover 高亮
- **macOS 当前**: **未实现**
- **实现**:
  - 绘制 ⋮ 符号
  - 添加 hover 检测和高亮效果
  - 点击后弹出菜单（需要 NSMenu 实现）

#### 6. 编辑模式输入框
- **Windows**: 编辑模式下显示输入框 + 清除按钮（X）+ 保存按钮（✓），使用 DirectWrite 渲染
- **macOS 当前**: **未实现**
- **实现**:
  - 绘制输入框背景（深色半透明）
  - 绘制光标（闪烁的白色竖线）
  - 绘制清除和保存按钮（圆形背景 + 符号）
  - 添加按钮 hover 高亮

#### 7. 图标网格布局
- **Windows**: 使用 `box_core::layout_calc::GridLayout` 计算布局，支持动态列数
- **macOS 当前**: 硬编码 4 列，80x100 单元格
- **修复**: 复用 `box_core::GridLayout`，根据窗口宽度动态计算列数

#### 8. 多视图模式
- **Windows**: 支持 ExtraLarge（128px）、Large（96px）、Medium（48px）、Small（32px）四种视图
- **macOS 当前**: **未实现**，固定 48px 图标
- **实现**:
  - 添加 `ViewMode` 枚举和参数结构
  - 根据视图模式调整图标大小、单元格尺寸、行高
  - 在更多菜单中提供视图切换选项

#### 9. Hover 高亮效果
- **Windows**: 鼠标悬停在图标上时显示半透明白色背景
- **macOS 当前**: **未实现**
- **实现**:
  - 添加鼠标移动事件跟踪
  - 计算当前 hover 的图标索引
  - 绘制 hover 背景高亮（圆角矩形）

#### 10. 选中状态
- **Windows**: 点击图标后显示选中状态（更深的半透明背景）
- **macOS 当前**: **未实现**
- **实现**:
  - 添加鼠标点击事件处理
  - 记录选中图标索引
  - 绘制选中背景高亮

#### 11. 文件名截断和 Tooltip
- **Windows**: 长文件名截断显示（...），鼠标悬停时显示完整名称的 Tooltip 窗口
- **macOS 当前**: 未截断，无 Tooltip
- **实现**:
  - 使用 `NSString.drawInRect` 的截断模式（`NSLineBreakByTruncatingTail`）
  - 创建独立的 Tooltip NSWindow（小尺寸、半透明背景）
  - 鼠标悬停 500ms 后显示 Tooltip

#### 12. 排序和分组
- **Windows**: 支持按名称/类型/大小/修改时间排序，支持升序/降序切换
- **macOS 当前**: **未实现**
- **实现**:
  - 添加 `SortMode` 和 `sort_ascending` 状态
  - 在更多菜单中提供排序选项
  - 实现排序逻辑（可按 `box_core` 中的工具函数复用）

#### 13. 上下文菜单系统
- **Windows**: 完整的右键菜单（查看、排序、刷新、锁定、删除分类框等）
- **macOS 当前**: **未实现**
- **实现**:
  - 使用 `NSMenu` + `NSMenuItem` 创建菜单
  - 实现菜单项点击回调
  - 菜单项包括：查看模式、排序方式、锁定、重命名、删除

#### 14. 边缘检测和拖拽调整
- **Windows**: 鼠标靠近窗口边缘时改变光标为调整大小光标，可拖拽调整窗口大小
- **macOS 当前**: **未实现**
- **实现**:
  - 检测鼠标是否在窗口边缘（上下左右各 4px）
  - 改变光标形状（`NSCursor.resizeLeft` 等）
  - 实现边缘拖拽调整窗口大小

#### 15. 图标绘制方式
- **Windows**: 使用 WIC 解码图标 → D2D Bitmap → DrawBitmap，支持透明度混合
- **macOS 当前**: 使用 `NSImage.drawInRect` 通过 `NSGraphicsContext` 绘制到 CGContext
- **验证**: 需要确认图标透明度是否正确渲染，是否与 Windows 视觉效果一致

#### 16. Retina/DPI 适配
- **Windows**: 使用 `GetDpiForWindow` + `scale_for_dpi` 函数适配所有尺寸
- **macOS 当前**: 使用 `backingScaleFactor`，但需要验证所有元素（文字、图标、按钮）是否正确缩放
- **修复**: 添加统一的 `scale_value(value: f64, scale: f64) -> f64` 工具函数

### 实现优先级

**P0 - 必须立即修复（影响基本可用性）**:
1. 圆角背景渲染正确性
2. 图标绘制透明度验证
3. Retina/DPI 适配

**P1 - 重要功能（对标 Windows 核心体验）**:
4. 锁定按钮 + hover
5. 更多按钮 + hover
6. 标题文字描边效果
7. 图标 hover 高亮
8. 文件名截断

**P2 - 增强功能（提升用户体验）**:
9. 编辑模式输入框
10. 多视图模式
11. 上下文菜单
12. Tooltip

**P3 - 高级功能（后续迭代）**:
13. 选中状态
14. 排序和分组
15. 边缘拖拽调整

---

## 阶段 2：图标渲染

### 目标
在覆盖窗口上渲染所有非管理桌面图标，外观与原生桌面图标一致。

### 新增文件：`desktop_overlay/icon_layout.rs`

```rust
/// 桌面图标项（覆盖窗口内使用）
pub struct OverlayIconItem {
    pub file_path: String,         // 文件/应用路径
    pub display_name: String,      // 显示名称
    pub icon_image: CGImage,       // Core Graphics 图标
    pub x: f32,                    // 持久化 X 坐标（逻辑像素）
    pub y: f32,                    // 持久化 Y 坐标（逻辑像素）
}

/// 桌面图标布局（与 Windows 版本结构一致）
pub struct DesktopIconLayout {
    pub icon_size: i32,
    pub label_height: i32,
    pub cell_w: i32,
    pub cell_h: i32,
    pub cols: i32,
    pub padding: i32,
}
```

**渲染细节**：
- 使用 `CGContext::draw_image` 绘制图标
- 使用 `CGContext::show_text_at_point` 或 Core Text 绘制标签
- 选中/hover 效果：绘制半透明蓝色矩形（模拟 macOS Finder 选中色 `#0066CC`）
- 图标尺寸：48x48（Retina 96x96），标签高度 20pt

### 修改：`icon_extract.rs`

```rust
use cocoa::appkit::NSWorkspace;

/// 提取文件图标（NSWorkspace.iconForFile）
pub fn extract_file_icon(file_path: &str) -> Result<CGImage>;
```

**实现**：
```rust
let workspace = NSWorkspace::sharedWorkspace();
let ns_image = workspace.iconForFile(NSString::alloc(nil).init_str(file_path));
// NSImage → CGImage 转换
let cg_image = ns_image.CGImageForProposedRect_context_hints_...();
```

---

## 阶段 3：图标交互

### 目标
实现覆盖窗口图标的双击打开、右键菜单、hover 高亮。

### 新增文件：`desktop_overlay/overlay_event.rs`

```rust
/// 自定义 NSView 子类，处理鼠标事件
pub struct OverlayView {
    // ...
}

impl OverlayView {
    /// 鼠标按下（左键/右键）
    pub fn mouse_down(&self, event: id);
    pub fn right_mouse_down(&self, event: id);
    
    /// 双击打开
    pub fn mouse_down_with_count(&self, event: id);
    
    /// 鼠标移动（hover 高亮）
    pub fn mouse_moved(&self, event: id);
    
    /// 命中测试（空白区域穿透到桌面）
    pub fn hit_test(&self, point: NSPoint) -> id;
}
```

**双击打开**：
```rust
let path = icon.file_path;
NSWorkspace::sharedWorkspace().openFile(NSString::alloc(nil).init_str(&path));
```

**右键菜单**：
```rust
let menu = NSMenu::alloc(nil).init();
menu.addItemWithTitle(..., "打开", ...);
menu.addItemWithTitle(..., "显示简介", ...);
menu.addItemWithTitle(..., "移到废纸篓", ...);
NSMenu::popUpContextMenu(menu, event, view);
```

**点击穿透**：
- 图标区域：返回 self（处理点击）
- 空白区域：返回 nil（穿透到桌面）

---

## 阶段 4：拖拽交互

### 目标
实现覆盖窗口与分类框之间的双向拖拽。

### 新增文件：`desktop_overlay/overlay_drag.rs`

```rust
/// 发起拖拽（覆盖窗口 → 分类框）
pub fn begin_drag_operation(
    window: id,
    icon: &OverlayIconItem,
    event: id,
);

/// 接受拖放（分类框 → 覆盖窗口）
pub fn accept_drop(
    window: id,
    file_path: String,
    drop_point: NSPoint,
);
```

**拖拽实现**（macOS 原生方式）：
1. 调用 `window.dragImage_at_offset_event_pasteboard_source_slideBack()`
2. 分类框窗口实现 `NSDraggingDestination` 协议接受拖放
3. 使用 `NSPasteboard` 传递文件路径

**自定义拖拽方式**（更可控）：
1. 创建拖拽覆盖窗口（半透明图标 + 标签）
2. 跟随鼠标移动
3. 检测释放位置：分类框区域 / 覆盖窗口空白区域

---

## 阶段 5：位置持久化

### 目标
图标在覆盖窗口上的位置在重启后保持不变。

### 修改：`config/app_config.rs`

**复用 Windows 版本的 OverlayConfig 结构**（已存在）：
```rust
pub struct OverlayConfig {
    /// 非管理图标位置映射：file_path → [x, y]（逻辑像素）
    pub icon_positions: HashMap<String, [f32; 2]>,
    /// 覆盖窗口图标尺寸
    pub icon_size: i32,
    /// 排列方向
    pub layout_direction: String,
}
```

**坐标系转换**：
- macOS 坐标系：原点在左下角，Y 轴向上
- 持久化使用逻辑像素（Point，非 Pixel），Retina 无关

---

## 阶段 6：生命周期集成

### 目标
覆盖窗口的创建/销毁与应用生命周期绑定。

### 新增文件：`icon_hider.rs`

```rust
/// 隐藏原生桌面图标（AppleScript）
pub fn hide_native_icons() -> Result<()> {
    let script = r#"
        tell application "Finder"
            set visible of every item of desktop to false
        end tell
    "#;
    run_applescript(script)
}

/// 恢复原生桌面图标
pub fn restore_native_icons() -> Result<()> {
    let script = r#"
        tell application "Finder"
            set visible of every item of desktop to true
        end tell
    "#;
    run_applescript(script)
}

fn run_applescript(script: &str) -> Result<()> {
    std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()?;
    Ok(())
}
```

**替代方案**：使用 `NSAppleScript` API 直接调用，避免进程创建开销。

### 修改：`app.rs`

```rust
fn restore_layout(&mut self) {
    // 1. 隐藏原生桌面图标
    hide_native_icons();

    // 2. 创建覆盖窗口（渲染非管理图标）
    create_desktop_overlay();

    // 3. 恢复分类框
    for box_data in &boxes {
        create_grid_window(params);
    }
}

fn drop(&mut self) {
    // 1. 保存布局
    // 2. 销毁分类框
    destroy_all_grid_windows();
    // 3. 销毁覆盖窗口
    destroy_desktop_overlay();
    // 4. 恢复原生桌面图标
    restore_native_icons();
}
```

### 多显示器支持
- 使用 `NSScreen.screens()` 枚举所有屏幕
- 每个屏幕创建一个覆盖窗口
- `screen.visibleFrame` 获取工作区域（自动排除 Dock）

### Finder 重启恢复
- macOS Finder 重启后桌面图标会重新显示
- 使用 `NSWorkspace.shared.notificationCenter` 监听 `NSWorkspaceActiveSpaceDidChangeNotification`
- 检测到桌面变化时重新执行 `hide_native_icons()`

---

## 阶段 7：桌面文件监听（可选增强）

### 目标
桌面新增/删除文件时，覆盖窗口自动更新。

### 实现
- 复用 `notify` crate（已在 Cargo.toml 中）
- 监听 `~/Desktop` 目录变化
- 新增文件 → 覆盖窗口添加图标
- 删除文件 → 覆盖窗口移除图标
- 节流：debounce 500ms

---

## 实现顺序与依赖关系

```
阶段 1（覆盖窗口基础）
  ↓
阶段 1.5（分类框渲染 - 对标 Windows）← 当前阶段
  ↓
阶段 2（图标渲染）
  ↓
阶段 3（图标交互：双击/右键）
  ↓
阶段 6（生命周期集成）← 到这里就有可用的 MVP
  ↓
阶段 4（拖拽交互）
  ↓
阶段 5（位置持久化）
  ↓
阶段 7（文件监听）
```

**MVP（最小可用版本）= 阶段 1 + 1.5 + 2 + 3 + 6**
- 启动后：原生图标隐藏 → 分类框渲染（对标 Windows）→ 覆盖窗口显示非管理图标 → 双击/右键正常
- 退出后：覆盖窗口销毁 → 原生图标恢复

---

## 关键风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| AppleScript 权限问题 | 严重 | 首次运行提示用户授予辅助功能权限（System Preferences → Security & Privacy） |
| Z-order 不对（覆盖窗口挡住 Dock） | 严重 | 使用 `visibleFrame` 自动排除 Dock，`setLevel` 精确控制 |
| Retina 显示模糊 | 中 | 使用 `backingScaleFactor` 适配，CGContext 使用 2x 尺寸 |
| 右键菜单与原生 Finder 菜单冲突 | 中 | 使用 `NSWorkspace` 打开文件，保持行为一致性 |
| 拖拽与分类框窗口交互 | 中 | 分类框 z-order 高于覆盖窗口，使用 `NSDraggingDestination` 协议 |
| Finder 重启后图标重新显示 | 中 | 监听空间切换通知 + 定时器检测 |
| macOS 沙盒限制 | 低 | 应用需要签名 + 用户授权，或使用非沙盒分发方式 |

---

## macOS 特有考量

### 权限管理

macOS 需要用户授权才能：
1. **辅助功能权限**：控制其他应用（Finder 桌面图标）
   - 检测：`AXIsProcessTrusted()`
   - 提示：`AXIsProcessTrustedWithOptions()` 弹出系统对话框
2. **桌面文件访问**：读取 `~/Desktop` 需要用户授权（macOS 10.15+）
   - 通过 `NSOpenPanel` 或 Info.plist 声明用途

### Info.plist 配置

```xml
<key>LSUIElement</key>
<true/>  <!-- 隐藏 Dock 图标 -->

<key>NSDesktopFolderUsageDescription</key>
<string>需要访问桌面文件夹以管理图标</string>

<key>NSAppleEventsUsageDescription</key>
<string>需要控制 Finder 以隐藏/显示桌面图标</string>
```

### 签名与公证

- 开发阶段：可使用 `codesign --force --deep --sign -` 自签名
- 正式发布：需要 Apple Developer 账号 + Notarization

---

## 验收标准

- [ ] 启动后：原生桌面图标隐藏，覆盖窗口显示非管理图标
- [ ] 覆盖窗口图标双击正常启动程序
- [ ] 覆盖窗口图标右键显示上下文菜单
- [ ] 覆盖窗口空白区域右键显示桌面背景菜单（或穿透到 Finder）
- [ ] 分类框内的图标不在覆盖窗口上显示（选择性隐藏）
- [ ] 拖桌面图标到分类框 → 图标从覆盖窗口消失，出现在框内
- [ ] 拖分类框图标到覆盖窗口 → 图标出现在释放位置，从框消失
- [ ] 退出应用 → 所有图标恢复原生桌面
- [ ] 多显示器正确显示（包括 Dock 位置适配）
- [ ] Retina 显示清晰
- [ ] 首次运行正确提示权限授权
- [ ] 不需要 root 权限
- [ ] `cargo fmt` + `cargo clippy` + `cargo test` green

---

## 与 Windows 版本的代码复用策略

### 可直接复用（平台无关）

- `config/app_config.rs`：OverlayConfig 配置结构
- `box_core/`：分类框业务逻辑
- `classification/`：分类规则和 AI 分类
- `utils/`：日志、DPI 计算、字符串工具
- `notify` crate 的文件监听逻辑

### 需要适配（平台特定）

| Windows | macOS | 说明 |
|---------|-------|------|
| `ffi_types.rs` | `ffi_imports.rs` | Core Foundation / Cocoa FFI |
| `d2d_render.rs` | `overlay_render.rs` | Core Graphics 替代 Direct2D |
| `shell_menu.rs` | `context_menu.rs` | NSMenu 替代 IContextMenu |
| `icon_extract.rs` | `icon_extract.rs` | NSWorkspace 替代 SHGetFileInfo |
| `drag_handler.rs` | `overlay_drag.rs` | NSDraggingSource 替代 OLE |
| `icon_hider.rs` | `icon_hider.rs` | AppleScript 替代 ShowWindow |

### 新增（macOS 特有）

- `ffi_imports.rs`：Core Foundation / Core Graphics / Cocoa FFI 绑定
- 权限检测与提示逻辑
- Info.plist 配置

---

## 依赖项（Cargo.toml）

```toml
[target.'cfg(target_os = "macos")'.dependencies]
cocoa = "0.25"
objc = "0.2"
objc-foundation = "0.1"
core-foundation = "0.9"
core-graphics = "0.23"
core-text = "20"
```

---

**文档版本**：v1.0  
**最后更新**：2026-07-08
