# 分类框图标拖拽（框间移动 + 恢复桌面 + 桌面拖入）

## Goal

实现分类框内快捷方式图标的完整拖拽交互：框→框移动、框→桌面恢复、桌面→框拖入。使用 OLE Drag-Drop 标准接口，提供与原生 Windows 拖拽一致的体验。

## Requirements

### R1: 框→框拖拽移动
* 鼠标按住分类框内图标，移动超过 5px 阈值后进入 OLE 拖拽模式
* 拖拽到另一个分类框上方释放 → 图标从源框移除，添加到目标框
* 拖拽悬停在目标框上时，目标框显示高亮边框（蓝色半透明）
* 锁定的分类框不允许拖出图标（拖拽不启动）
* 锁定的分类框不允许拖入图标（Drop 返回 DROPEFFECT_NONE）

### R2: 框→桌面拖拽恢复
* 从分类框拖拽图标到桌面空白区域释放
* 图标从源框移除，SysListView32 中对应图标从屏幕外移回到释放位置
* 释放位置为鼠标在桌面坐标系的物理像素位置
* .lnk 文件本身不移动（已在 Desktop 目录）

### R3: 桌面→框拖拽拖入
* 用户从桌面拖动一个快捷方式图标到分类框上方释放
* 接收文件路径（CF_HDROP），创建 GridFileItem 并添加到目标框
* SysListView32 中对应图标移到屏幕外（隐藏）
* 提取图标句柄（SHGetFileInfoW）用于 D2D 渲染

### R4: 拖拽视觉反馈
* OLE 系统拖影（IDragSourceHelper 或系统默认）
* 鼠标光标：移动操作显示移动箭头，无效目标显示禁止符号
* 目标分类框悬停时高亮边框
* 拖拽取消（Esc / 拖回原位 / 释放到无效区域）不产生变化

### R5: 持久化与配置
* 每次拖拽操作完成后自动 `collect_grid_layout()` + `save()`
* 桌面图标状态（在框内/在桌面）由配置隐式管理：在 `icon_names` 中 = 在框内

### R6: 删除框时恢复图标
* 删除分类框时，框内所有图标恢复到桌面（SysListView32 中移回原位）
* 使用图标上次已知的桌面位置，或自动排列到可用位置

### R7: 异常处理与优雅降级
* .lnk 文件不存在 → 跳过该项，日志警告，不影响其他操作
* SysListView32 找不到对应图标 → 仅更新逻辑分组（配置），日志警告
* explorer 重启后 PIDL 失效 → 通过文件名重新匹配 ListView index
* OLE 初始化失败 → 降级为无拖拽功能，日志错误
* 跨进程内存操作失败 → 降级为仅逻辑移动（不操作 SysListView32），日志错误

### R8: 预留多选拖拽接口
* 数据格式设计支持多文件（CF_HDROP 天然支持多文件列表）
* 自定义格式 `IkunBoxDragData` 包含 `Vec<DragItemInfo>` 而非单个
* MVP 只拖拽一个图标，但接口不限制为单个

## Acceptance Criteria

* [ ] 鼠标按住图标移动超过 5px 后启动 OLE DoDragDrop
* [ ] 拖拽到目标分类框释放 → 源框 items 移除 + 目标框 items 添加 + 配置保存
* [ ] 拖拽到桌面释放 → 源框 items 移除 + SysListView32 图标定位到释放位置 + 配置保存
* [ ] 从桌面拖入分类框 → SysListView32 图标隐藏 + 目标框 items 添加 + 配置保存
* [ ] 拖拽过程中系统拖影跟随鼠标
* [ ] 目标分类框悬停时显示蓝色高亮边框
* [ ] 锁定状态下不允许拖拽图标出框
* [ ] 锁定状态下不允许拖拽图标入框
* [ ] 删除分类框 → 框内图标全部恢复到桌面
* [ ] .lnk 文件不存在时拖拽优雅降级（跳过 + 日志）
* [ ] SysListView32 操作失败时拖拽优雅降级（仅逻辑移动 + 日志）
* [ ] `cargo fmt` + `cargo clippy` + `cargo test` green
* [ ] 所有拖拽事件有中文日志

## Definition of Done

* Tests added/updated（单元测试覆盖数据格式序列化、跨框移动逻辑）
* `cargo fmt` + `cargo clippy` + `cargo test` green
* 日志完整（拖拽开始/目标检测/释放/取消/错误均有中文日志）
* 每个新增/修改文件 ≤ 1000 行

## Technical Approach

### 核心决策

| 决策 | 选择 | 理由 |
|------|------|------|
| 桌面图标管理 | 逐图标隐藏/显示 | 需要单个图标的独立可见性控制 |
| 拖拽协议 | OLE Drag-Drop | 需要桌面→框（标准 Drop Target） |
| 拖拽视觉 | 系统拖影 + 高亮 | 与 Windows 原生体验一致 |
| 框间通信 | 自定义 OLE 格式 + PostMessage | 避免 OLE 执行文件操作 |
| 拖拽到桌面 | CF_HDROP + DROPEFFECT_MOVE | Shell 处理图标定位 |

### 数据流

```
框→框移动:
  源框 WM_LBUTTONDOWN + mouse move > 5px
    → DoDragDrop(IDataObject[CF_HDROP + IkunBoxDragData], IDropSource)
    → 目标框 IDropTarget::DragEnter/DragOver → 显示高亮
    → 目标框 IDropTarget::Drop
      → 读取 IkunBoxDragData（自定义格式，含源框 HWND + item index）
      → 从源框 items 移除（PostMessage WM_USER_ITEM_REMOVED）
      → 添加到目标框 items
      → 重绘两个框 + 保存配置
    → DoDragDrop 返回 DROPEFFECT_MOVE

框→桌面恢复:
  源框 DoDragDrop(IDataObject[CF_HDROP + IkunBoxDragData])
    → 桌面（Shell）IDropTarget::Drop
      → Shell 处理 CF_HDROP（.lnk 已在 Desktop 目录，move = no-op）
      → Shell 将图标定位到释放位置
    → DoDragDrop 返回 DROPEFFECT_MOVE
    → 源框检查返回值，从 items 移除 + 保存配置

桌面→框拖入:
  Explorer DoDragDrop(CF_HDROP)
    → 目标框 IDropTarget::DragEnter/DragOver → 显示高亮
    → 目标框 IDropTarget::Drop
      → 读取 CF_HDROP 获取文件路径
      → SysListView32: 找到该图标，移到屏幕外（隐藏）
      → 创建 GridFileItem（提取 icon_handle）
      → 添加到目标框 items
      → 重绘 + 保存配置
    → 返回 DROPEFFECT_MOVE
```

### 新增/修改文件清单

| 文件 | 操作 | 说明 | 预估行数 |
|------|------|------|---------|
| `platform/windows/icon_manager.rs` | **新建** | 逐图标管理：hide/show/position/find | ~300 |
| `platform/windows/ole_dnd.rs` | **新建** | IDataObject + IDropSource + IDropTarget COM 实现 | ~600 |
| `platform/windows/wnd_proc.rs` | **修改** | WM_LBUTTONDOWN 启动拖拽、WM_USER_* 跨框消息 | +100 |
| `platform/windows/grid_window.rs` | **修改** | 注册 DropTarget、拖拽状态字段、删除恢复 | +80 |
| `platform/windows/ffi_imports.rs` | **修改** | 新增 OLE/VirtualAllocEx 相关 API 声明 | +30 |
| `platform/windows/ffi_types.rs` | **修改** | 新增 OLE 常量和类型 | +20 |
| `platform/windows/icon_hider.rs` | **重构** | 从整体隐藏改为逐图标管理（复用 find_desktop_listview） | ~100 |
| `box_core/interaction.rs` | **修改** | 拖拽状态扩展（拖拽中项、拖拽数据） | +30 |
| `config/app_config.rs` | **修改** | BoxLayoutData 可选增加 desktop_position 字段 | +10 |
| `Cargo.toml` | **修改** | 新增 Win32_System_Memory/Threading/Diagnostics_Debug features | +5 |

### OLE 自定义数据格式

```rust
/// 框间拖拽自定义数据（注册为 "IkunBoxDragData" 剪贴板格式）
pub struct IkunBoxDragData {
    /// 源分类框窗口句柄（用于 PostMessage 通知移除）
    pub source_hwnd: HWND,
    /// 源分类框 config_id（UUID，用于持久化匹配）
    pub source_config_id: uuid::Uuid,
    /// 被拖拽的项列表（MVP 为单个，预留多选）
    pub items: Vec<DragItemInfo>,
}

pub struct DragItemInfo {
    /// 在源框 items 中的索引
    pub index: usize,
    /// 文件路径（.lnk）
    pub file_path: String,
    /// 显示名称
    pub display_name: String,
}
```

### 跨进程 SysListView32 操作（icon_manager.rs）

```rust
pub struct DesktopIconManager {
    listview_hwnd: HWND,
    explorer_process: HANDLE,
    /// 文件名 → ListView index 的缓存（explorer 重启后重建）
    name_to_index: HashMap<String, i32>,
}

impl DesktopIconManager {
    /// 查找桌面 SysListView32（复用 icon_hider 的逻辑）
    pub fn new() -> Result<Self>;
    
    /// 按文件名查找图标的 ListView index
    pub fn find_icon_by_name(&self, name: &str) -> Result<i32>;
    
    /// 隐藏单个图标（移到屏幕外 x=-32000）
    pub fn hide_icon(&self, index: i32) -> Result<()>;
    
    /// 显示单个图标（移到指定桌面坐标）
    pub fn show_icon_at(&self, index: i32, x: i32, y: i32) -> Result<()>;
    
    /// 批量隐藏图标
    pub fn hide_icons(&self, indices: &[i32]) -> Result<()>;
    
    /// 重建 name→index 映射（explorer 重启后调用）
    pub fn rebuild_index(&mut self) -> Result<()>;
    
    /// 释放 explorer 进程句柄
    pub fn cleanup(&self);
}
```

## Decision (ADR-lite)

**Context**: 需要实现分类框图标的三向拖拽交互（框→框、框→桌面、桌面→框），同时支持逐图标的可见性控制。

**Decision**: 
- 采用 OLE Drag-Drop 标准协议（IDropSource + IDropTarget + DoDragDrop）
- 使用 CF_HDROP 支持与 Shell/桌面互操作
- 使用自定义 `IkunBoxDragData` 格式标识框间内部移动
- 将 icon_hider.rs 从整体隐藏重构为逐图标管理（icon_manager.rs）
- COM 接口在 Rust 中通过 vtable 结构手动实现

**Consequences**:
- 优点：与 Windows 原生拖拽体验完全一致，系统拖影、光标反馈均由 OLE 自动处理
- 优点：桌面→框的拖入功能自然支持（标准 Drop Target）
- 缺点：COM 接口实现较复杂（~600 行），需要 careful memory management
- 风险：windows crate v0.58 的 OLE 接口绑定可能需要额外的 unsafe 代码
- 风险：跨进程 SysListView32 操作依赖 explorer.exe 内部结构，未来 Windows 版本可能变化

## Out of Scope (explicit)

* 多图标同时拖拽（MVP 单图标，但接口已预留多选）
* 框内拖拽排序（重排图标顺序）
* 拖拽到任务栏/开始菜单等系统区域
* macOS 支持
* 拖拽动画（拖拽起始时的"抬起"效果）
* 拖拽过程中源图标的半透明化

## Technical Notes

### 已检查的文件（关键发现）

| 文件 | 关键发现 |
|------|---------|
| `wnd_proc.rs` (1015行) | 处理 WM_LBUTTONDOWN（选中）、WM_MOUSEMOVE（hover），无拖拽 |
| `grid_window.rs` (1162行) | `GridWindowData` 含 items、GRID_WINDOWS 全局列表、collect_grid_layout |
| `icon_hider.rs` (156行) | `find_desktop_listview()` 可复用，整体 ShowWindow 隐藏 |
| `interaction.rs` (66行) | `InteractionState` 有 is_dragging/drag_start，但未被使用 |
| `box_model.rs` (77行) | `BoxItem` 有 file_path，`ClassificationBox` 有 items |
| `app_config.rs` (939行) | `BoxLayoutData.icon_names: Vec<String>` 存储文件路径 |
| `腾讯桌面方案调研.md` | IDropTarget + LVM_SETITEMPOSITION 方案参考 |
| `06-10-box-container-icons-move/prd.md` | 之前的框移动方案（已归档，未实现）|

### 新增 Cargo.toml features

```toml
windows = { version = "0.58", features = [
    # ... 现有 features ...
    "Win32_System_Memory",           # VirtualAllocEx, VirtualFreeEx
    "Win32_System_Threading",        # OpenProcess, GetCurrentProcessId
    "Win32_System_Diagnostics_Debug", # PROCESS_VM_* 权限常量
    "Win32_System_Ole",              # IDropTarget, IDropSource, DoDragDrop
    "Win32_System_Com",              # CoInitialize, STGMEDIUM, FORMATETC
    "Win32_System_DataExchange",     # RegisterClipboardFormatW, CF_HDROP
]}
```

### Windows OLE API 调用链

```
拖拽启动:
  1. RegisterClipboardFormatW("IkunBoxDragData") → 自定义格式 ID
  2. 构建 IDataObject（CF_HDROP + IkunBoxDragData）
  3. 构建 IDropSource
  4. DoDragDrop(data_object, drop_source, DROPEFFECT_MOVE, &mut effect)
  5. 根据返回的 effect 执行后续操作

拖拽接收:
  1. RegisterDragDrop(hwnd, drop_target) — 窗口创建时注册
  2. IDropTarget::DragEnter → 检查数据格式，设置 effect
  3. IDropTarget::DragOver → 更新高亮
  4. IDropTarget::DragLeave → 清除高亮
  5. IDropTarget::Drop → 读取数据，执行移动

窗口销毁时:
  RevokeDragDrop(hwnd)
```
