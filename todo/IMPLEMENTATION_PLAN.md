# 桌面图标智能管理 — 实现计划

## 核心思路

**覆盖窗口方式**（与腾讯 DeskGo / Stardock Fences 相同的底层原理）：

1. `ShowWindow(SW_HIDE)` 隐藏原生 SysListView32（全部桌面图标消失）
2. 创建全屏覆盖窗口，自己渲染桌面图标（D2D/DirectWrite）
3. 覆盖窗口处理所有图标交互（双击、右键、拖拽）
4. 分类框内的图标不在覆盖窗口上渲染（等效"隐藏"）
5. 拖拽 = 覆盖窗口 ⇄ 分类框窗口之间的窗口间操作

```
Z-order（从上到下）:
  应用窗口
  分类框窗口 (WS_POPUP + WS_EX_LAYERED)
  覆盖窗口 (WS_POPUP + WS_EX_LAYERED) ← 新增
  SysListView32 (被 ShowWindow 隐藏)
  桌面壁纸 (Progman / WorkerW)
```

---

## 模块结构

```
src/platform/windows/
├── desktop_overlay/               # 新增模块
│   ├── mod.rs                     # 模块导出
│   ├── overlay_window.rs          # 覆盖窗口创建/销毁/生命周期
│   ├── overlay_render.rs          # D2D 渲染（图标网格 + hover/选中效果）
│   ├── overlay_wnd_proc.rs        # 窗口消息处理（鼠标/键盘/拖拽）
│   ├── icon_layout.rs             # 桌面图标布局算法（网格排列 + 位置持久化）
│   └── overlay_drag.rs            # 拖拽发起（覆盖窗口 → 分类框）
│
├── icon_hider.rs                  # 保留 ShowWindow 隐藏/恢复
├── d2d_render.rs                  # 复用 draw_icon_d2d、render pipeline
├── grid_window.rs                 # 复用窗口创建模式
├── icon_extract.rs                # 复用图标提取
├── shell_menu.rs                  # 复用右键菜单
├── drop_target.rs                 # 复用 OLE Drop Target
├── desktop_manager.rs             # 复用桌面图标枚举
└── ...其他现有模块保持不变
```

---

## 阶段 1：覆盖窗口基础

### 目标
创建一个全屏 WS_EX_LAYERED 覆盖窗口，覆盖桌面区域（不含任务栏），能显示半透明背景。

### 新增文件：`desktop_overlay/overlay_window.rs`

```rust
/// 覆盖窗口创建参数
pub struct OverlayCreateParams {
    pub monitor: HMONITOR,          // 目标显示器
    pub x: i32, y: i32,            // 工作区域起点
    pub width: i32, height: i32,   // 工作区域尺寸
    pub dpi: u32,
}

/// 创建桌面覆盖窗口
pub fn create_overlay_window(params: OverlayCreateParams) -> Result<HWND>;

/// 销毁覆盖窗口
pub fn destroy_overlay_window(hwnd: HWND);

/// 获取所有覆盖窗口（多显示器支持）
pub fn get_overlay_windows() -> Vec<HWND>;
```

**关键实现细节**：
- 窗口类名：`"DesktopOrgOverlay"`
- 窗口样式：`WS_POPUP | WS_EX_LAYERED | WS_EX_TOOLWINDOW`
- **Z-order**：`SetWindowPos(HWND_BOTTOM)` 放在桌面最底层（SysListView32 之上，应用窗口之下）
- **不抢焦点**：`WM_MOUSEACTIVATE` 返回 `MA_NOACTIVATE`
- **点击穿透空白区域**：`WM_NCHITTEST` 在空白处返回 `HTTRANSPARENT`，让鼠标事件穿透到桌面（这样用户可以右键桌面空白处）
- DPI 自适应：`GetDpiForWindow` + `GetMonitorInfoW`

### 新增文件：`desktop_overlay/overlay_render.rs`

```rust
/// 渲染覆盖窗口（D2D/DirectWrite → DIB → UpdateLayeredWindow）
pub fn render_overlay(hwnd: HWND);
```

**渲染管线**（复用 `d2d_render.rs` 的模式）：
1. `CreateCompatibleDC` + `CreateDIBSection(32-bit)` → 内存 DC
2. `ID2D1DCRenderTarget.BindDC` → D2D 绑定
3. `Clear(transparent)` → 全透明背景
4. 渲染图标网格（见阶段 2）
5. `UpdateLayeredWindow(AC_SRC_ALPHA)` → 提交

**第一步验证**：覆盖窗口显示一个半透明色块，确认 Z-order 正确。

### 可复用代码
| 来源 | 复用内容 |
|------|---------|
| `grid_window.rs` | 窗口类注册、WS_EX_LAYERED 创建、GWLP_USERDATA 模式 |
| `d2d_render.rs` | D2D 渲染管线、`init_d2d_devices`、`draw_icon_d2d` |
| `ffi_types.rs` | 所有类型定义和常量 |
| `ffi_imports.rs` | 所有 Win32 API 声明 |

---

## 阶段 2：图标渲染

### 目标
在覆盖窗口上渲染所有非管理桌面图标，外观与原生桌面图标一致。

### 新增文件：`desktop_overlay/icon_layout.rs`

```rust
/// 桌面图标项（覆盖窗口内使用）
pub struct OverlayIconItem {
    pub file_path: String,         // .lnk 或文件路径
    pub display_name: String,      // 显示名称
    pub icon_handle: HICON,        // 系统图标句柄
    pub is_managed: bool,          // 是否在分类框中（true = 不渲染）
}

/// 桌面图标布局
pub struct DesktopIconLayout {
    pub icon_size: i32,            // 图标尺寸（DPI 自适应）
    pub label_height: i32,         // 标签高度
    pub cell_w: i32,               // 单元格宽度
    pub cell_h: i32,               // 单元格高度
    pub cols: i32,                 // 列数
    pub padding: i32,              // 边距
}

/// 计算桌面图标布局
pub fn calculate_desktop_layout(area_width: i32, area_height: i32, dpi: u32) -> DesktopIconLayout;

/// 命中测试
pub fn hit_test(layout: &DesktopIconLayout, items: &[OverlayIconItem], x: i32, y: i32) -> Option<usize>;
```

**布局算法**：
- 图标从右上角开始排列（与 Windows 桌面默认一致）
- 网格对齐（snap to grid）
- 排除分类框窗口占据的区域（可选，后续优化）
- DPI 自适应：基准 48px 图标 @ 96 DPI

### 修改：`desktop_overlay/overlay_render.rs`

渲染每个非管理图标：
1. 图标图像：`draw_icon_d2d(target, wic, hicon, x, y, size)` — **直接复用**
2. 图标标签：DirectWrite 渲染白色文字 + 阴影（模拟桌面效果）
3. 选中效果：半透明蓝色背景矩形
4. Hover 效果：半透明白色边框

### 数据来源
- 桌面图标枚举：`DesktopManager::get_desktop_icons()` — **直接复用**
- 图标提取：`extract_file_icon(path)` — **直接复用**
- 分类框内图标列表：从 `AppConfig.layout.boxes[].icon_names` 读取

### 过滤逻辑
```rust
// 从配置获取所有管理图标路径
let managed_paths: HashSet<String> = config.layout.boxes
    .iter()
    .flat_map(|b| b.icon_names.iter().cloned())
    .collect();

// 只渲染非管理图标
for icon in desktop_icons {
    let is_managed = managed_paths.contains(&icon.path.to_string_lossy().to_string());
    // is_managed = true → 跳过渲染
}
```

---

## 阶段 3：图标交互

### 目标
覆盖窗口上的图标具有与原生桌面图标一致的交互行为。

### 新增文件：`desktop_overlay/overlay_wnd_proc.rs`

```rust
pub unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM
) -> LRESULT;
```

### 消息处理

| 消息 | 处理 |
|------|------|
| `WM_MOUSEMOVE` | 检测 hover 图标 → 重绘 hover 效果 + `TrackMouseEvent` |
| `WM_MOUSELEAVE` | 清除 hover 状态 |
| `WM_LBUTTONDOWN` | 选中图标（单选 / Ctrl+多选） |
| `WM_LBUTTONDBLCLK` | 双击启动：`ShellExecuteW("open", file_path)` |
| `WM_RBUTTONUP` | 右键菜单：`show_shell_context_menu(hwnd, file_path)` — **直接复用** |
| `WM_NCHITTEST` | 图标区域 → `HTCLIENT`；空白区域 → `HTTRANSPARENT`（点击穿透） |
| `WM_MOUSEACTIVATE` | 返回 `MA_NOACTIVATE`（不抢焦点） |
| `WM_SETCURSOR` | 图标上 → 手型/箭头；空白 → 默认 |
| `WM_DPICHANGED` | 重新计算布局 + 重绘 |
| `WM_SIZE` | 重新计算布局 + 重绘 |
| `WM_TIMER` | 拖拽阈值检测（见阶段 4） |

### 双击启动
```rust
WM_LBUTTONDBLCLK => {
    if let Some(idx) = hit_test(layout, items, x, y) {
        let path = &items[idx].file_path;
        ShellExecuteW(hwnd, "open", path_wide, ...);
    }
}
```

### 右键菜单
```rust
WM_RBUTTONUP => {
    if let Some(idx) = hit_test(layout, items, x, y) {
        show_shell_context_menu(hwnd, &items[idx].file_path);
    } else {
        show_desktop_context_menu(hwnd); // 桌面空白右键
    }
}
```

### 可复用代码
| 来源 | 复用内容 |
|------|---------|
| `shell_menu.rs` | `show_shell_context_menu`、`show_desktop_context_menu` |
| `wnd_proc.rs` | 消息处理模式（hover、选中、hit test） |

---

## 阶段 4：拖拽交互

### 目标
- 覆盖窗口上的图标可以拖到分类框
- 分类框内的图标可以拖到覆盖窗口（桌面）

### 4A：覆盖窗口 → 分类框（拖出）

**新增文件：`desktop_overlay/overlay_drag.rs`**

使用自定义拖拽（与分类框内拖拽相同的模式）：
1. `WM_LBUTTONDOWN` → 记录 `drag_pending = true, drag_start_x/y`
2. `WM_MOUSEMOVE` → 移动超过 `DRAG_THRESHOLD(5px)` → 激活拖拽
3. 创建拖拽覆盖窗口（`drag_image.rs` — **直接复用**）
4. `SetCapture(hwnd)` → 鼠标跟随
5. `WM_MOUSEMOVE` → 更新覆盖窗口位置 + 检测目标分类框高亮
6. `WM_LBUTTONUP` → `ReleaseCapture` → 检测释放目标

释放检测：
```rust
let target_hwnd = WindowFromPoint(cursor_pos);
if is_grid_window(target_hwnd) {
    // 拖入分类框：PostMessage(target, WM_USER_DRAG_ITEM_ADDED)
    // 从覆盖窗口移除该图标 + 标记为 managed + 保存配置
} else {
    // 取消拖拽（拖回原位）
}
```

### 4B：分类框 → 覆盖窗口（拖入）

**修改：`drag_handler.rs` 的 `handle_box_to_desktop`**

当前逻辑已处理"框→桌面"，需要适配覆盖窗口：
1. 分类框拖出 → 检测到释放目标是覆盖窗口（类名 `"DesktopOrgOverlay"`）
2. 从分类框移除该项
3. 标记为非管理图标（从 config 的 `icon_names` 中移除）
4. 覆盖窗口重绘（该图标出现）
5. Vault export（如果是用户桌面图标）

### 4C：覆盖窗口内拖拽（图标重新排列）

后续实现：用户在覆盖窗口内拖拽图标改变位置。
- 拖拽释放到覆盖窗口空白区域 → 更新图标位置 → 保存

### 可复用代码
| 来源 | 复用内容 |
|------|---------|
| `drag_handler.rs` | 自定义拖拽三阶段模式（阈值检测 → 覆盖窗口跟随 → 释放检测） |
| `drag_image.rs` | 拖拽覆盖窗口（半透明图标 + 标签） |
| `drop_target.rs` | OLE IDropTarget（如果选择 OLE 拖拽方式） |

---

## 阶段 5：位置持久化

### 目标
图标在覆盖窗口上的位置在重启后保持不变。

### 修改：`config/app_config.rs`

新增配置段：
```rust
pub struct OverlayConfig {
    /// 非管理图标位置映射：file_path → [x, y]（逻辑像素）
    pub icon_positions: HashMap<String, [f32; 2]>,
    /// 覆盖窗口图标尺寸
    pub icon_size: i32,
    /// 排列方向（right-to-left / left-to-right）
    pub layout_direction: String,
}

// 在 AppConfig 中添加
pub struct AppConfig {
    // ... 现有字段 ...
    pub overlay: OverlayConfig,  // 新增
}
```

### 保存/恢复流程

```
保存: overlay icon positions → config.overlay.icon_positions → save()
恢复: load() → config.overlay.icon_positions → 定位图标
```

---

## 阶段 6：生命周期集成

### 目标
覆盖窗口的创建/销毁与应用生命周期绑定。

### 修改：`app.rs`

```rust
fn restore_layout(&mut self) {
    // 1. 隐藏原生桌面图标
    hide_native_icons();  // ShowWindow(SW_HIDE)

    // 2. 创建覆盖窗口（渲染非管理图标）
    create_desktop_overlay();

    // 3. 恢复分类框
    for box_data in &boxes {
        create_grid_window(params);
    }
}

fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    if !self.initialized {
        self.initialized = true;
        self.restore_layout();
        // ...
    }
}
```

### 修改：`MyApp::drop()`

```rust
fn drop(&mut self) {
    // 1. 保存布局
    // 2. 销毁分类框
    destroy_all_grid_windows();
    // 3. 销毁覆盖窗口
    destroy_desktop_overlay();
    // 4. 恢复原生桌面图标
    restore_native_icons();  // ShowWindow(SW_SHOW)
    // 5. 清理注册表
    cleanup_desktop_registry();
}
```

### 多显示器支持
- 为每个显示器创建一个覆盖窗口
- `MonitorFromPoint` + `GetMonitorInfoW` 获取工作区域（**复用** `grid_window.rs`）
- 每个覆盖窗口独立渲染和交互

### Explorer 重启恢复
- 检测 SysListView32 是否失效（`IsWindow` 检查）
- 如果失效：重新 `ShowWindow(SW_HIDE)` 隐藏新建的 SysListView32
- 覆盖窗口不受影响（独立窗口）

---

## 阶段 7：桌面文件监听（可选增强）

### 目标
桌面新增/删除文件时，覆盖窗口自动更新。

### 实现
- 使用 `notify` crate（已在 Cargo.toml 中）监听桌面目录
- 新增 .lnk → 覆盖窗口添加图标
- 删除 .lnk → 覆盖窗口移除图标
- 节流：debounce 500ms，避免快速连续事件

---

## 实现顺序与依赖关系

```
阶段 1（覆盖窗口基础）
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

**MVP（最小可用版本）= 阶段 1 + 2 + 3 + 6**
- 启动后：原生图标隐藏 → 覆盖窗口显示非管理图标 → 双击/右键正常
- 退出后：覆盖窗口销毁 → 原生图标恢复

---

## 关键风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| Z-order 不对（覆盖窗口挡住应用） | 严重 | 使用 `HWND_BOTTOM` + 不抢焦点 + 空白区域点击穿透 |
| 覆盖窗口右键菜单异常 | 中 | 复用已验证的 `shell_menu.rs` |
| 多显示器 DPI 缩放 | 中 | 复用 `get_cursor_monitor_work_area_phys` 模式 |
| 图标数量大时渲染性能 | 低 | D2D 硬件加速 + 仅渲染可见区域 |
| 覆盖窗口和分类框拖拽冲突 | 中 | 分类框 Z-order 高于覆盖窗口，拖拽目标检测按 Z-order 优先 |
| Explorer 重启后原生图标重新显示 | 中 | 定时器检测 + 重新 ShowWindow(SW_HIDE) |
| WM_NCHITTEST 穿透逻辑复杂 | 中 | 图标区域返回 HTCLIENT，其余返回 HTTRANSPARENT |

---

## 验收标准

- [ ] 启动后：原生桌面图标隐藏，覆盖窗口显示非管理图标
- [ ] 覆盖窗口图标双击正常启动程序
- [ ] 覆盖窗口图标右键显示 Shell 上下文菜单
- [ ] 覆盖窗口空白区域右键显示桌面背景菜单
- [ ] 分类框内的图标不在覆盖窗口上显示（选择性隐藏）
- [ ] 拖桌面图标到分类框 → 图标从覆盖窗口消失，出现在框内
- [ ] 拖分类框图标到覆盖窗口 → 图标出现在释放位置，从框消失
- [ ] 退出应用 → 所有图标恢复原生桌面
- [ ] 多显示器正确显示
- [ ] 不需要管理员权限
- [ ] `cargo fmt` + `cargo clippy` + `cargo test` green
