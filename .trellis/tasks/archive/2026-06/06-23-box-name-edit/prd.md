# 分类框名称编辑功能

## Goal
实现分类框名称的内联编辑：双击标题文字进入编辑模式，输入新名称后保存。

## Requirements
- 双击分类框标题文字区域（非锁定/更多按钮）进入名称编辑模式
- 编辑模式：标题栏显示输入框 + 清除按钮（×）+ 保存按钮
- 输入框最大 16 个字符（支持 Unicode）
- 清除按钮清空输入框内容
- 保存按钮保存新名称并退出编辑模式
- Enter 键保存，Escape 键取消
- 失去焦点时取消编辑
- 编辑模式下禁止窗口拖拽和边缘缩放

## Acceptance Criteria
- [x] 双击标题进入编辑模式
- [x] 输入框限制 16 字符
- [x] 清除按钮功能
- [x] 保存按钮功能
- [x] 键盘快捷键（Enter/Escape/方向键/Home/End/Backspace/Delete）
- [x] 编辑模式禁止拖拽/缩放
- [x] 失去焦点取消编辑
- [x] 保存后名称持久化到配置文件

## Technical Notes
- 编辑模式通过 `GridWindowData.editing_name` 标志控制
- 输入缓冲区 `edit_buffer` 存储编辑中的文字
- 光标位置 `edit_cursor` 以字符索引（非字节索引）计算
- 渲染使用 D2D/DirectWrite，输入框为圆角矩形+蓝色边框
- 保存时更新 `data.title`，`collect_grid_layout()` 自动持久化到 `~/.ikun_organizer.json`

## Changed Files
- `src/platform/windows/ffi_types.rs` — WM_CHAR, WM_KILLFOCUS, VK_BACK 等常量
- `src/platform/windows/ffi_imports.rs` — SetFocus/GetFocus FFI 声明
- `src/platform/windows/grid_window.rs` — GridWindowData 新字段 + 布局辅助函数
- `src/platform/windows/wnd_proc.rs` — 编辑模式消息处理
- `src/platform/windows/d2d_render.rs` — 编辑模式 UI 渲染
