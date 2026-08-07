# Windows Shell / COM API Thinking Guide

> **Purpose**: Prevent Windows Shell API misuse by documenting correct call patterns.
> **Origin**: 分类框右键菜单任务 — 4 轮修复才正确。

---

## When to Use This Guide

- [ ] Calling any Windows Shell API (IShellFolder, IContextMenu, IShellView, etc.)
- [ ] Working with PIDLs (ITEMIDLIST)
- [ ] Creating context menus (file or background)
- [ ] Using COM interfaces through `windows` crate or FFI

---

## Core Principle: The Shell Object Model

```
IShellFolder (文件夹)
  ├── CreateViewObject::<IShellView>()  → Shell 视图
  │     ├── GetItemObject::<IContextMenu>(SVGIO_BACKGROUND)  → 背景右键菜单
  │     └── GetItemObject::<IContextMenu>(SVGIO_SELECTION)   → 选中项菜单
  │
  ├── GetUIObjectOf(hwnd, &[相对PIDL], riid)  → 子项的对象
  │     └── 返回 IContextMenu / IExtractIcon / IDataObject 等
  │
  └── ParseDisplayName()  → 绝对 PIDL
```

**关键区别**：
| 方法 | 用途 | PIDL 要求 |
|------|------|-----------|
| `GetUIObjectOf` | 获取**子项**的 Shell 对象 | **相对 PIDL**（父文件夹的直接子项） |
| `CreateViewObject::<IContextMenu>` | 文件夹级别的上下文菜单 | 无 PIDL |
| `CreateViewObject::<IShellView>` → `GetItemObject(SVGIO_BACKGROUND)` | **文件夹空白区域**的背景菜单 | 无 PIDL |

---

## Trap 1: 绝对 PIDL vs 相对 PIDL

**错误**：`SHParseDisplayName` 返回**绝对 PIDL**（多段路径），直接传给 `GetUIObjectOf`。

```rust
// ❌ 错误：绝对 PIDL 传给 GetUIObjectOf
let mut full_pidl = std::ptr::null_mut();
SHParseDisplayName(path, None, &mut full_pidl, 0, None)?;
desktop.GetUIObjectOf(hwnd, &[full_pidl], None)?;  // 💥 深层路径会崩溃

// ✅ 正确：使用 SHBindToParent 获取父文件夹 + 相对 PIDL
let mut full_pidl = std::ptr::null_mut();
SHParseDisplayName(path, None, &mut full_pidl, 0, None)?;

let mut rel_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
let parent: IShellFolder = SHBindToParent(full_pidl as *const _, Some(&mut rel_pidl))?;
parent.GetUIObjectOf(hwnd, &[rel_pidl as *const _], None)?;  // ✅

ILFree(Some(full_pidl));  // 只释放 full_pidl，rel_pidl 指向其内部
```

**原理**：`GetUIObjectOf` 的 `apidl` 参数要求是**父文件夹的直接子项 PIDL**（单段），而非完整路径 PIDL（多段）。

---

## Trap 2: 文件夹菜单 vs 背景菜单

**错误**：用 `CreateViewObject::<IContextMenu>` 获取桌面/文件夹空白区域菜单。

```rust
// ❌ 错误：文件夹级菜单（缺少 查看/排序/刷新/新建 等背景菜单项）
let cm: IContextMenu = desktop.CreateViewObject(hwnd)?;

// ✅ 正确：通过 IShellView 获取背景菜单
let view: IShellView = desktop.CreateViewObject(hwnd)?;
let cm: IContextMenu = view.GetItemObject(SVGIO_BACKGROUND)?;  // 真正的背景菜单
```

**原理**：
- `CreateViewObject::<IContextMenu>` → 文件夹对象自身的菜单（属性、自定义等）
- `IShellView::GetItemObject(SVGIO_BACKGROUND)` → shell 视图提供的背景菜单（查看、排序、新建等）

---

## Trap 3: InvokeCommand verb 偏移

**错误**：`TrackPopupMenu` 返回的 cmd ID 直接传给 `InvokeCommand`。

```rust
let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, x, y, 0, hwnd, null());

// ❌ 错误：cmd 是绝对菜单 ID，不是 verb 偏移
lpVerb: PCSTR(cmd as usize as *const u8),

// ✅ 正确：减去 idCmdFirst 得到 verb 偏移
lpVerb: PCSTR((cmd as u32 - FCIDM_SHVIEWFIRST) as usize as *const u8),
```

**原理**：`QueryContextMenu` 使用 `idCmdFirst` 到 `idCmdLast` 的范围分配菜单 ID。`InvokeCommand` 的 `lpVerb` 期望的是相对于 `idCmdFirst` 的偏移量。

---

## Trap 4: 手动 COM vtable FFI

**错误**：用 `#[repr(C)]` 手动定义 COM vtable 结构体并调用。

```rust
// ❌ 错误：手动 vtable 可能有 ABI 不匹配
#[repr(C)]
struct IContextMenuVtbl {
    QueryInterface: unsafe extern "system" fn(...) -> HRESULT,
    // ... 手动定义的函数指针
}
let cm_raw: *mut IContextMenu_vt = cm.as_raw() as *mut _;
((*(*cm_raw).lpVtbl).QueryContextMenu)(...);  // 💥 可能崩溃

// ✅ 正确：使用 windows crate 的安全方法
cm.QueryContextMenu(hmenu, 0, first, last, flags)?;
cm.InvokeCommand(&invoke_info)?;
```

**原理**：`windows` crate 自动生成的 vtable 经过验证，包含正确的类型映射和调用约定。手动定义容易出现参数类型不匹配、对齐问题等。

---

## Trap 5: 混合手动 FFI 和 windows crate

**规则**：同一个 COM 调用链中，不要混用手动 FFI 类型和 `windows` crate 类型。

```rust
// ❌ 错误：混合使用
let cm: windows::Win32::UI::Shell::IContextMenu = ...;  // crate 类型
let info: MyCMINVOKECOMMANDINFO = ...;  // 手动定义的结构体
cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO);  // 💥

// ✅ 正确：全部使用 windows crate 类型
let cm: windows::Win32::UI::Shell::IContextMenu = ...;
let info: windows::Win32::UI::Shell::CMINVOKECOMMANDINFO = ...;
cm.InvokeCommand(&info)?;
```

---

## Required Feature Flags (Cargo.toml)

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Win32_UI_Shell",              # IShellFolder, IContextMenu, SHGetDesktopFolder
    "Win32_UI_Shell_Common",       # ITEMIDLIST, SHParseDisplayName, SHBindToParent
    "Win32_System_Ole",            # IShellView 依赖
    "Win32_UI_WindowsAndMessaging", # HMENU, MSG
    "Win32_System_Com",            # CoInitializeEx, CLSCTX
    "Win32_Foundation",            # HWND, HRESULT, HANDLE
]}
```

---

## Quick Reference: 常见 Shell 操作

### 文件右键菜单
```
SHParseDisplayName → 绝对 PIDL
SHBindToParent → 父 IShellFolder + 相对 PIDL
parent.GetUIObjectOf(相对PIDL) → IContextMenu
cm.QueryContextMenu → TrackPopupMenu → cm.InvokeCommand(verb = cmd - idCmdFirst)
ILFree(绝对PIDL)
```

### 文件夹/桌面背景菜单
```
SHGetDesktopFolder → IShellFolder
desktop.CreateViewObject → IShellView
view.GetItemObject(SVGIO_BACKGROUND) → IContextMenu
cm.QueryContextMenu → TrackPopupMenu → cm.InvokeCommand(verb = cmd - idCmdFirst)
```

### 选中项菜单
```
view.GetItemObject(SVGIO_SELECTION) → IContextMenu
```

---

## Vault Staging 机制

分类框中的图标文件存储在 managed_icons/ vault 目录（文件名带 user_/public_ 前缀），
不直接暴露在桌面。右键菜单操作需要将文件路径传给 Shell API，但直接使用 vault 路径会导致：
- **删除**：回收站显示 user_Chrome.lnk（泄漏内部前缀）
- **剪切/复制**：剪贴板包含 vault 路径（粘贴时文件名带前缀）

**解决方案**：在 Shell 操作前创建 staging 副本

```
vault 文件: ~/.ikun_organizer/managed_icons/user__Chrome.lnk
                  ↓ fs::copy (原始文件名)
staging 副本: ~/.ikun_organizer/staging/Chrome.lnk
                  ↓ Shell 菜单操作（用户看到 Chrome.lnk）
操作完成 → cleanup_staging() 删除 staging 副本
```

**关键函数**：
- icon_hider::create_staging_for_shell(logical_path) → 创建 staging 副本
- icon_hider::cleanup_staging(staging_path) → 清理 staging 文件
- icon_hider::is_vault_path(path) → 判断是否需要 staging
- shell_menu::ShellMenuResponse { result, staging_path } → 携带 staging 路径

**注意事项**：
- NeedsDelete 和 NeedsCutMonitor 必须使用原始 ile_path（非 staging 路径）进行 vault 清理
- start_cut_monitor 监控 staging 文件是否存在，粘贴后 staging 消失时触发 vault 清理

---

## Checklist for Shell API Code Review

- [ ] PIDL 类型正确？（绝对 vs 相对，是否用 SHBindToParent）
- [ ] PIDL 释放正确？（只释放 SHParseDisplayName 返回的，不释放 SHBindToParent 的 ppidlLast）
- [ ] 菜单层级正确？（文件夹菜单 vs 背景菜单 vs 选中项菜单）
- [ ] InvokeCommand verb 已减去 idCmdFirst？
- [ ] 全部使用 `windows` crate 类型？（无手动 COM vtable）
- [ ] Cargo.toml feature flags 完整？

---

## Lesson Learned

> **每次 Shell API 调用前，先画完整的调用链图。确认每一步的 PIDL 类型、对象层级和返回值语义。不要假设 —— 查文档。**
