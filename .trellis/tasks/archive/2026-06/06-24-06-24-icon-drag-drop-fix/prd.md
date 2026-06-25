# 修复图标拖拽体验（拖拽图像 + 桌面恢复）

## Goal

修复分类框图标拖拽的两个关键缺陷：
1. 拖拽图像渲染异常（图标缩小、不完整、黑色背景）— 需要达到原生桌面一样的拖拽体验
2. 拖拽到桌面空白区域后图标消失 — 需要让图标实际出现在桌面释放位置

## What I already know

### 代码分析结果

| 文件 | 关键发现 |
|------|---------|
| `drag_image.rs` (266行) | 自定义 WS_EX_LAYERED 覆盖窗口 + `icon_to_bitmap()` 将 HICON 转 48×48 DIB |
| `drag_image.rs::icon_to_bitmap()` | 使用 `DrawIconEx(DI_NORMAL)` 渲染 + mask 修复 alpha，**这是黑底的根因** |
| `d2d_render.rs::draw_icon_d2d()` | 正确方案：`GetIconInfo` → `hbm_color` → `GetDIBits` 读 32-bit ARGB → premultiplied alpha |
| `drag_handler.rs::handle_box_to_desktop()` | 只做了 `data.items.remove()`，**没有调用 DesktopIconManager 恢复图标位置** |
| `icon_manager.rs` | DesktopIconManager 已完整实现 `show_icon_at()`，但**从未在拖拽流程中使用** |
| `ole_dnd.rs` | OLE IDataObject 不暴露 CF_HDROP（注释说避免域安全策略），自定义拖拽模式 |
| `icon_extract.rs` | 图标提取使用 `SHDefExtractIconW` 提取 128×128 高清图标 |
| `grid_window.rs` | 视图参数 `icon_size` 默认 48，DPI 自适应 |

### 问题 1：拖拽图像黑底/缩小/不完整 — 根因分析

`icon_to_bitmap()` 函数（drag_image.rs:171-265）有两个致命缺陷：

**缺陷 A — Alpha 通道处理错误**：
```
1. DrawIconEx(hdc_mem, ..., DI_NORMAL) → 渲染 RGB 到 32-bit DIB
2. GetIconInfo → 获取 hbm_mask
3. GetDIBits(hbm_mask) → 读取 mask bits
4. 如果 mask_bits[i] & 0xFFFFFF == 0 → 设置 alpha=FF
   否则 → alpha=0
```

问题：
- `DrawIconEx` 渲染 32-bit ARGB 图标到 DIB 时，**alpha 通道已经是正确的**（对于现代图标），不需要 mask 修复
- mask 修复逻辑是二值的（FF 或 0），**破坏了半透明抗锯齿像素**
- 对于没有传统 mask 的 32-bit 图标，`hbm_mask` 可能返回异常数据
- 结果：黑色/灰色背景，边缘粗糙

**缺陷 B — 没有读取 hbm_color 的原始像素**：
- `d2d_render.rs::draw_icon_d2d()` 使用正确的方案：从 `icon_info.hbm_color` 直接 `GetDIBits` 读取原始 32-bit ARGB 像素
- `drag_image.rs` 却使用 `DrawIconEx` 间接渲染，丢失了原始 alpha 信息

**缺陷 C — 固定 48×48 尺寸**：
- 源图标为 128×128，覆盖窗口硬编码 48×48
- 应该使用原始图标尺寸或根据 DPI 自适应

### 问题 2：拖拽到桌面图标消失 — 根因分析

`drag_handler.rs::handle_box_to_desktop()` 当前只做了：
```rust
let removed = data.items.remove(item_index);  // 从框中移除
render_layered_window(hwnd);                   // 重绘框
save_config();                                 // 保存配置
```

**缺失的操作**：
- 没有调用 `DesktopIconManager::show_icon_at()` 将 SysListView32 中的图标移到释放位置
- `DesktopIconManager` 已经完整实现但从未被拖拽流程调用
- 结果：.lnk 文件还在桌面目录，但 SysListView32 中图标仍在屏幕外（隐藏状态），用户看不到

### Windows 原生拖拽体验参考

原生桌面拖拽特征：
1. 拖拽图像 = 原尺寸图标 + 半透明（约 70%）
2. 图标跟随光标，偏移 hotspot 位置（右下方）
3. 拖拽到桌面释放 → 图标出现在释放位置
4. 拖拽过程中光标变化（移动/复制/禁止）

## Requirements

### R1: 拖拽图像修复（与原生一致）
* 从 HICON 的 `hbm_color` 直接读取 32-bit ARGB 原始像素（复用 draw_icon_d2d 的方案）
* 不再使用 `DrawIconEx` + mask 修复方式
* 拖拽覆盖窗口尺寸匹配原始图标尺寸（或 DPI 自适应的 icon_size）
* 半透明效果（约 70% 不透明度）
* 图标完整显示，无黑色背景，边缘抗锯齿平滑

### R2: 拖拽到桌面恢复图标
* 使用 `DesktopIconManager::show_icon_at()` 将图标移到释放位置
* 释放坐标 = 鼠标屏幕坐标（物理像素）
* 需要在拖拽开始时初始化 DesktopIconManager（懒加载，首次拖拽时创建）
* 如果 DesktopIconManager 初始化失败 → 优雅降级（仅从框中移除，日志警告）

### R3: 拖拽覆盖窗口优化
* 覆盖窗口尺寸 = DPI 自适应的 icon_size（与分类框内图标大小一致）
* 窗口位置偏移 hotspot（与原生拖拽一致，光标在图标左上角附近）
* WS_EX_LAYERED + WS_EX_TRANSPARENT + WS_EX_TOPMOST 属性保持不变

### R4: 异常处理
* HICON 无 hbm_color（极罕见的单色图标）→ 回退到 DrawIconEx
* DesktopIconManager 初始化失败 → 降级为仅逻辑移动 + 日志
* SysListView32 找不到对应图标 → 仅从框中移除 + 日志警告
* explorer 重启后缓存失效 → rebuild_index 后重试

## Acceptance Criteria

* [ ] 拖拽时图标完整显示，无黑色背景，边缘平滑（抗锯齿正确）
* [ ] 拖拽图像大小与分类框内图标大小一致（DPI 自适应）
* [ ] 拖拽半透明效果（约 70% 不透明度）
* [ ] 拖拽到桌面空白区域释放 → 图标出现在释放位置（SysListView32 可见）
* [ ] 拖拽到同一分类框 → 取消（无变化）
* [ ] 拖拽到另一个分类框 → 框间移动（已实现，保持不变）
* [ ] DesktopIconManager 初始化失败时优雅降级
* [ ] `cargo fmt` + `cargo clippy` + `cargo test` green
* [ ] 所有拖拽事件有中文日志

## Definition of Done

* `cargo fmt` + `cargo clippy` + `cargo test` green
* 日志完整（拖拽图像创建/桌面恢复/降级均有中文日志）
* 每个新增/修改文件 ≤ 1000 行

## Technical Approach

### 修复 1: icon_to_bitmap() 重写

复用 `draw_icon_d2d()` 的正确方案，适配 UpdateLayeredWindow 需要的 straight alpha DIB：

```
旧方案（有缺陷）:
  DrawIconEx(DI_NORMAL) → mask 修复 alpha → 黑底

新方案（正确）:
  GetIconInfo → hbm_color
  GetDIBits(hbm_color, bi_bit_count=0) → 查询格式
  GetDIBits(hbm_color, 32bpp) → 读取原始 BGRA 像素
  → 像素已经是正确的 straight alpha
  → 创建 DIB Section → 拷贝像素 → UpdateLayeredWindow
```

关键点：
- UpdateLayeredWindow + BLENDFUNCTION(AC_SRC_ALPHA) 需要 **straight alpha**（非 premultiplied）
- draw_icon_d2d 转换为 premultiplied（D2D 需要），这里不需要转换
- 直接拷贝 hbm_color 的原始像素即可

### 修复 2: handle_box_to_desktop() 增加 SysListView32 恢复

```
handle_box_to_desktop(hwnd, item_index):
  1. 获取 removed item 的 file_path
  2. 从 file_path 提取文件名（如 "app.lnk"）
  3. 获取/创建 DesktopIconManager（全局懒加载）
  4. 按文件名查找 SysListView32 index
  5. 调用 show_icon_at(index, drop_x, drop_y)
  6. 从框中移除 item
  7. 保存配置
```

### 修改文件清单

| 文件 | 修改 | 说明 |
|------|------|------|
| `drag_image.rs` | **重写 icon_to_bitmap()** | 从 hbm_color 读取原始 ARGB 像素，不再用 DrawIconEx |
| `drag_image.rs` | **修改 create_drag_overlay()** | 接受 icon_size 参数（DPI 自适应） |
| `drag_handler.rs` | **修改 handle_box_to_desktop()** | 调用 DesktopIconManager 恢复图标 |
| `drag_handler.rs` | **新增全局 DesktopIconManager** | 懒加载单例（Mutex<Option<DesktopIconManager>>） |
| `drag_handler.rs` | **修改 check_drag_threshold()** | 传入 DPI 自适应的 icon_size |

## Decision (ADR-lite)

**Context**: 拖拽图像黑底和桌面图标消失是两个独立的 bug，分别需要修复 alpha 渲染和增加 SysListView32 恢复调用。

**Decision**: 
- 拖拽图像：从 `hbm_color` 直接读取原始 32-bit ARGB 像素（复用 draw_icon_d2d 已验证的方案）
- 桌面恢复：在 `handle_box_to_desktop` 中调用已实现的 `DesktopIconManager::show_icon_at()`
- 保持自定义覆盖窗口方案（不使用 OLE IDragSourceHelper），仅修复渲染逻辑

**Consequences**:
- 优点：修改范围小（~100 行核心改动），复用已验证的像素读取方案
- 优点：DesktopIconManager 已经完整实现，只需接入拖拽流程
- 风险：DesktopIconManager 跨进程操作可能因 explorer 状态变化失败（已有优雅降级）

## Out of Scope (explicit)

* 拖拽动画（"抬起"效果）
* 拖拽过程中源图标的半透明化
* 拖拽到任务栏/开始菜单等系统区域
* 多图标同时拖拽
* OLE IDragSourceHelper 集成
* macOS 支持

## Technical Notes

### 参考：draw_icon_d2d 的正确 alpha 方案

```rust
// d2d_render.rs:1062-1240 — 已验证的正确方案
// 1. GetIconInfo(hicon) → icon_info.hbm_color (32-bit ARGB)
// 2. GetDIBits(screen_dc, hbm_color, 0, 0, null, &mut bi, DIB_RGB_COLORS) // 查询格式
// 3. GetDIBits(screen_dc, hbm_color, 0, h, pixels, &mut bi, DIB_RGB_COLORS) // 读取像素
// 4. 像素是 straight alpha BGRA — D2D 需要转换为 premultiplied
// 5. 对于 UpdateLayeredWindow: 直接使用 straight alpha（不需要转换！）
```

### 关键区别：UpdateLayeredWindow vs D2D

| 渲染目标 | Alpha 格式 | 需要转换？ |
|----------|-----------|-----------|
| D2D DrawBitmap | premultiplied | 是（straight → premultiplied） |
| UpdateLayeredWindow + AC_SRC_ALPHA | straight alpha | 否（直接使用原始像素） |

### DesktopIconManager 接入点

```
drag_handler.rs 中新增：
static DRAG_ICON_MANAGER: Mutex<Option<DesktopIconManager>> = Mutex::new(None);

fn get_or_init_icon_manager() -> Option<&'static DesktopIconManager> {
    let mut mgr = DRAG_ICON_MANAGER.lock().ok()?;
    if mgr.is_none() {
        match DesktopIconManager::new() {
            Ok(m) => *mgr = Some(m),
            Err(e) => { log::warn!("DesktopIconManager 初始化失败: {}", e); return None; }
        }
    }
    mgr.as_ref()
}
```
