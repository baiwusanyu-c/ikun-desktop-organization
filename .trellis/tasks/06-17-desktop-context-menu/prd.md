# 桌面右键菜单 & 创建分类框对话框

## Goal

重新设计右键菜单系统，在桌面空白区域和分类框区域右键时添加"创建分类框"选项，修复"显示更多选项"不一致问题，并用独立 egui 窗口实现创建对话框。

## Requirements

- [x] 移除注册表方式（`setup_desktop_registry`），添加注册表清理函数（清除已有 IkunCreateBox 项）
- [ ] 桌面空白区域右键：IContextMenu 编程式注入"创建分类框"
- [ ] 格子窗口空白区域右键：IContextMenu 桌面背景菜单 + "创建分类框"（与桌面空白区域相同内容）
- [ ] 文件项右键：保持现有 Shell 上下文菜单不变
- [ ] "显示更多选项"内容与未运行程序时完全一致（移除注册表即可解决）
- [ ] 统一使用 egui viewport 创建对话框，移除 Win32 对话框（desktop_hook.rs 中的）
- [ ] 移除信号文件机制和 `--create-box` 参数
- [ ] 创建对话框：独立 egui 窗口，16 字符必填，确定后创建分类框

## Acceptance Criteria

- [ ] 程序运行时，桌面空白区域右键显示原生桌面菜单 + "创建分类框"
- [ ] 程序运行时，分类框空白区域右键显示原生桌面菜单 + "创建分类框"
- [ ] 程序运行时，分类框内文件项右键显示 Shell 上下文菜单（不变）
- [ ] "显示更多选项"内容与未运行程序时一致
- [ ] 程序退出后，桌面右键菜单完全恢复原生行为（无残留）
- [ ] 点击"创建分类框"弹出独立 egui 窗口，输入名称后创建分类框
- [ ] 启动时自动清理旧的注册表项

## Decision (ADR-lite)

**Context**: 当前通过注册表 `Directory\Background\shell\IkunCreateBox` 注入菜单项，导致 Windows 11 "显示更多选项"行为改变。3 套触发机制并存，2 套对话框实现冗余。

**Decision**: 移除注册表方式，改为 IContextMenu 编程式注入。统一使用 egui viewport 对话框。移除信号文件和 `--create-box` 机制。

**Consequences**:
- 程序退出后无残留影响
- "显示更多选项"始终原生
- 代码大幅简化（移除 desktop_hook.rs 对话框、信号文件、AtomicBool）

## Technical Approach

### 1. shell_menu.rs 重构

**show_desktop_context_menu** 核心改造：
```
SHGetDesktopFolder() → IShellFolder
  → CreateViewObject(hwnd) → IShellView
    → GetItemObject(SVGIO_BACKGROUND) → IContextMenu
      → QueryContextMenu(hmenu, 0, FCIDM_SHVIEWFIRST, MENU_CMD_ID_LAST, flags)
        → InsertMenu: 分隔线 + "创建分类框" (cmd_id = CMD_CREATE_BOX, < FCIDM_SHVIEWFIRST)
          → TrackPopupMenu
            → if cmd == CMD_CREATE_BOX → PENDING_CREATE_BOX.store(true)
            → if cmd >= FCIDM_SHVIEWFIRST → InvokeCommand
```

关键点：
- 自定义项 ID 使用 1（< FCIDM_SHVIEWFIRST=100），不与 Shell 命令冲突
- 分隔线 + 自定义项插入到 QueryContextMenu 之后
- PENDING_CREATE_BOX 原子标志保留（格子窗口 → app.rs 通信）

### 2. 注册表清理（startup 时调用一次）

```rust
pub fn cleanup_desktop_registry() {
    // 删除 HKCU\Software\Classes\Directory\Background\shell\IkunCreateBox
    // 包括 command 子键
}
```

### 3. grid_window.rs 修改

WM_RBUTTONUP 处理不变：
- 命中文件项 → show_shell_context_menu（不变）
- 未命中 → show_desktop_context_menu（改造后自带"创建分类框"）

### 4. app.rs 简化

- 移除信号文件检测逻辑
- 保留 `take_pending_create_box()` 消费（右键菜单 → viewport）
- 保留 egui viewport 对话框（已有实现）
- 保留控制面板 "创建分类框" 按钮直接调用

### 5. desktop_hook.rs 简化

- 移除 `show_create_box_dialog()`（Win32 对话框，不再需要）
- 移除 `setup_desktop_registry()`（注册表方式）
- 新增 `cleanup_desktop_registry()`（一次性清理）
- 保留 `create_box_and_save()` 逻辑（移入 app.rs 或保留为公共函数）

### 6. main.rs 简化

- 移除 `--create-box` 参数处理
- 移除信号文件写入逻辑

## Out of Scope

- 拖拽文件到分类框
- 分类框锁定/删除按钮
- 系统托盘菜单修改
- Shell 扩展 DLL

## Technical Notes

### 涉及文件变更

| 文件 | 变更 |
|------|------|
| `shell_menu.rs` | 重写 show_desktop_context_menu（InsertMenu 注入） |
| `desktop_hook.rs` | 移除 Win32 对话框 + 注册表，新增清理函数 |
| `app.rs` | 移除信号文件机制，简化 |
| `main.rs` | 移除 --create-box 处理 |
| `platform/windows/mod.rs` | 更新导出（cleanup_desktop_registry） |
| `platform/mod.rs` | 更新 re-export |

### IContextMenu 菜单项 ID 范围

- `FCIDM_SHVIEWFIRST = 0x0064 (100)` — Shell 命令起始 ID
- `MENU_CMD_ID_LAST = FCIDM_SHVIEWFIRST + 0x7FFF`
- 自定义项 cmd ID 必须 < FCIDM_SHVIEWFIRST（使用 1）

### 架构约束

- `platform/` 负责所有 Win32 API 调用
- `box_core/` 纯业务逻辑
- `ui/` 只通过 AppContext 交互
- 单文件 ≤ 1000 行
- 统一 anyhow::Result
