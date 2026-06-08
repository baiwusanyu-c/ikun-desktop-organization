# Windows Shell API 集成与单元测试

## Goal

完成 Day 3-4 开发计划：实现 Windows Shell API 集成，能够真实读取 Windows 桌面图标信息，并编写单元测试验证核心功能。

## Requirements

### P0 核心功能（必须完成）

1. **COM 初始化封装**
   - 使用 `CoInitializeEx` 初始化 COM
   - 在 `Drop` 中调用 `CoUninitialize` 清理
   - 错误处理：失败时记录日志并返回错误

2. **IShellFolder 接口调用**
   - 获取桌面文件夹 `IShellFolder`
   - 枚举桌面项目 `IEnumIDList`
   - 获取项目名称和属性

3. **桌面图标枚举实现**
   - 实现完整的 `get_desktop_icons()` 方法
   - 返回真实的桌面图标信息（名称、路径、位置）
   - 替换当前的模拟数据

4. **快捷方式解析**
   - 解析 `.lnk` 文件获取目标路径
   - 使用 `IShellLink` 接口
   - 处理解析失败的情况

5. **图标位置获取**
   - 获取桌面图标的位置坐标 (x, y)
   - 使用 `IShellView::GetItemPosition` 或其他方法

### P1 重要功能（尽量完成）

6. **单元测试编写**
   - 测试 COM 初始化和清理
   - 测试图标枚举逻辑
   - 测试快捷方式解析
   - 测试覆盖率目标：80%

### 技术约束

- 使用 `windows-rs` crate (v0.58)
- 所有代码使用 `#[cfg(windows)]` 条件编译
- 错误处理使用 `anyhow::Result`
- 遵循项目质量规范（无 unsafe、无 unwrap、文档注释）

## Acceptance Criteria

- [ ] COM 初始化成功，Drop 正确清理
- [ ] 可枚举桌面图标并获取真实数据（至少获取名称）
- [ ] 可解析 `.lnk` 快捷方式获取目标路径
- [ ] 单元测试覆盖核心功能，测试通过
- [ ] 代码通过 `cargo fmt` 和 `cargo clippy`
- [ ] 公开 API 有完整的文档注释
- [ ] 无 `unsafe` 代码（Windows API 调用除外）
- [ ] 错误处理遵循规范（使用 `.context()`）

## Technical Notes

### Windows API 调用流程

```
1. CoInitializeEx(NULL, COINIT_APARTMENTTHUNKED)
2. SHGetDesktopFolder(&pDesktopFolder)
3. pDesktopFolder->EnumObjects(NULL, SHCONTF_FOLDERS | SHCONTF_NONFOLDERS, &pEnum)
4. 循环 pEnum->Next(1, &pidl, &fetched)
5. 对每个 pidl:
   - GetDisplayNameOf(pidl, SHGDN_NORMAL) → 名称
   - 解析 .lnk → 目标路径
   - 获取位置信息
6. CoUninitialize()
```

### 关键 windows-rs API

```rust
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::Storage::FileSystem::*;
```

### 图标位置获取方案

Windows 桌面图标位置存储在注册表或通过 `IShellView` 获取：
- 方案 A: 使用 `IShellView::GetItemPosition`
- 方案 B: 读取注册表 `HKEY_CURRENT_USER\Software\Microsoft\Windows\Shell\Bags`

### 测试策略

由于 Windows API 依赖真实环境：
- 在 Windows 上运行测试（Parallels Desktop 或 GitHub Actions）
- macOS 上使用条件编译跳过 Windows 测试：`#[cfg(windows)]`
- 可以编写逻辑测试（数据解析等）在 macOS 上运行

## Related Files

- `src/desktop/windows.rs` - 主要实现文件
- `src/desktop/mod.rs` - Trait 定义
- `src/desktop/icons.rs` - 数据结构
- `Cargo.toml` - windows crate 配置