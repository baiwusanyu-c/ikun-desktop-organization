# Desktop Organizer - 全面重写实现计划

## 核心架构（对齐腾讯桌面整理）

```
┌─────────── 桌面（Explorer SysListView32）──────────┐
│                                                     │
│  原生图标全部移到 X=-3000（屏幕外，用户不可见）      │
│                                                     │
│  ┌─ 格子窗口 A ─────┐  ┌─ 格子窗口 B ─────┐        │
│  │ WS_POPUP          │  │ WS_POPUP          │        │
│  │ WS_EX_LAYERED     │  │ WS_EX_LAYERED     │        │
│  │ WS_EX_TOOLWINDOW  │  │ WS_EX_TOOLWINDOW  │        │
│  │                   │  │                   │        │
│  │ [📁 游戏] 标题栏  │  │ [💻 开发] 标题栏  │        │
│  │ ─────────────     │  │ ─────────────     │        │
│  │ 🎮Chrome  🎮Steam │  │ 📝VSCode  📝Git   │        │
│  │ 🎮Epic    🎮原神  │  │ 📝Node    📝Py    │        │
│  │ (GDI 自绘图标)    │  │ (GDI 自绘图标)    │        │
│  └───────────────────┘  └───────────────────┘        │
│                                                     │
│  拖动标题栏 → SetWindowPos → 整个窗口移动            │
│  图标用客户区相对坐标绘制 → 自然跟随窗口移动          │
│                                                     │
│  ┌─ 控制面板（egui 窗口）──┐                        │
│  │ [一键分类] [保存] [清除] │                        │
│  └─────────────────────────┘                        │
└─────────────────────────────────────────────────────┘
```

## 文件结构

```
src/
├── main.rs                    # egui 控制面板入口 + 中文字体加载
├── app.rs                     # 控制面板 UI + 调度分类/移动操作
│
├── desktop/
│   ├── mod.rs                 # 模块导出
│   ├── icons.rs               # DesktopIcon 数据结构（保留）
│   ├── icon_hider.rs          # [新建] 隐藏/恢复原生桌面图标
│   ├── file_icon.rs           # [新建] SHGetFileInfoW 提取文件图标
│   ├── grid_window.rs         # [新建] 格子窗口（核心！独立WS_POPUP + GDI自绘）
│   ├── windows.rs             # 文件系统读取桌面图标（保留简化）
│   └── overlay.rs             # [删除] 旧方案不再需要
│
├── classification/            # 保留不变
├── config/                    # 保留 + 扩展
├── ui/                        # 简化（box_widget 不再需要）
└── utils/                     # 保留
```

## Phase 详细步骤

### Phase 1: 隐藏原生桌面图标
**文件**: `icon_hider.rs`

1. 找到 SysListView32 句柄（Progman/WorkerW → SHELLDLL_DefView → SysListView32）
2. 关闭桌面自动排列（通过注册表或 IFolderView2）
3. 保存所有图标当前位置（用于退出时恢复）
4. 发送 LVM_SETITEMPOSITION 把所有图标移到 X=-3000
5. 程序退出时恢复图标到原始位置

**关键 API**:
- `FindWindowExW` - 逐层查找
- `SendMessageW(LVM_GETITEMPOSITION)` - 读取位置
- `SendMessageW(LVM_SETITEMPOSITION)` - 设置位置
- `VirtualAllocEx/ReadProcessMemory/WriteProcessMemory` - 跨进程

### Phase 2: 文件图标提取
**文件**: `file_icon.rs`

1. 对每个桌面文件调用 `SHGetFileInfoW` 获取 `HICON`
2. 将 HICON 转换为位图数据（用于 GDI 绘制）
3. 缓存图标句柄，避免重复提取

**关键 API**:
- `SHGetFileInfoW` - 获取文件图标、类型名
- `DrawIconEx` - 将 HICON 绘制到 DC
- `GetIconInfo` - 提取图标位图数据

### Phase 3: 格子窗口（核心）
**文件**: `grid_window.rs`

每个格子是一个独立的 Win32 窗口：

**窗口创建**:
- `RegisterClassExW` 注册窗口类
- `CreateWindowExW` 创建:
  - `WS_POPUP` (无边框弹窗)
  - `WS_EX_LAYERED` (分层透明)
  - `WS_EX_TOOLWINDOW` (不显示在任务栏)
  - `WS_EX_TOPMOST` (置顶)
- `SetLayeredWindowAttributes` 设置背景透明度

**WM_PAINT 自绘**:
- 双缓冲（CreateCompatibleDC + BitBlt）
- 绘制圆角半透明背景
- 绘制标题栏（分类名 + 图标数）
- 遍历 items，用 `DrawIconEx` 绘制文件图标 + 文件名
- 图标用**客户区相对坐标**（不是屏幕坐标）

**数据模型**:
```rust
struct GridFileItem {
    pidl_or_path: String,      // 文件路径
    display_name: String,      // 显示名
    icon_handle: HICON,        // 系统图标句柄（预加载）
    client_offset: (i32, i32), // 相对窗口客户区的坐标
}

struct GridWindow {
    hwnd: HWND,
    title: String,
    items: Vec<GridFileItem>,
    is_dragging: bool,
    drag_start: (i32, i32),
}
```

### Phase 4: 拖拽移动
**在 grid_window.rs 的 WNDPROC 中**:

- `WM_LBUTTONDOWN`: 检测点击在标题栏 → 开始拖拽
- `WM_MOUSEMOVE`: 拖拽中 → `SetWindowPos` 移动整个窗口
- `WM_LBUTTONUP`: 结束拖拽 → 保存位置
- 图标不需要任何处理（它们是客户区相对坐标，自动跟随）

### Phase 5: 一键分类 + 控制面板
**文件**: `app.rs`, `main.rs`

控制面板（egui 窗口）提供：
- [一键分类] → 分类文件 → 隐藏原生图标 → 创建格子窗口
- [保存布局] → 保存格子位置到 JSON
- [清除] → 销毁格子窗口 → 恢复原生图标
- 格子列表 + 选择

### Phase 6: 编译测试
- cargo check/build 确保编译通过
- 运行测试：一键分类 → 看到格子 → 拖动格子 → 图标跟随

## 关键避坑点

1. **不用子窗口** → 必须 WS_POPUP 独立弹窗
2. **WS_EX_TRANSPARENT 只用于空白区域** → 格子内部可以接收鼠标
3. **不依赖系统原生图标位置** → 所有布局由内存 GridFileItem 管理
4. **图标用客户区相对坐标** → 窗口移动时图标自动跟随
5. **PIDL 需手动释放** → 封装 RAII 释放逻辑
6. **双缓冲防闪烁** → CreateCompatibleDC + BitBlt
