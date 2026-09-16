# 修复：桌面/资源管理器拖拽文件夹到分类框不显示

## 目标
修复文件夹在拖拽操作中被静默丢弃的问题，使文件夹与文件一样可以在桌面、分类框、资源管理器之间自由拖拽。

## 根因分析

代码库中多处文件操作使用 `std::fs::copy` / `std::fs::remove_file`，这些 API **仅支持文件，不支持目录**。当拖拽对象是文件夹时，操作静默失败，导致文件夹不出现在分类框中。

### 受影响的拖拽路径

| 路径 | 失败点 | 原因 |
|------|--------|------|
| 资源管理器→分类框 | `copy_file_to_user_desktop()` (drag_drop.rs:867) | `std::fs::copy` 不支持目录 |
| 资源管理器→覆盖层 | `copy_file_to_desktop()` (overlay_drag_ops.rs:840) | 同上 |
| 分类框→资源管理器 | `start_grid_ole_drag()` (drag_handler.rs:560,670) | `std::fs::copy` 不支持目录 |
| 分类框→桌面 | `export_icon()` (icon_hider.rs:822) | `std::fs::remove_file` 不支持目录 |
| 仓库清理 | `ingest_icon()` (icon_hider.rs:741) | `std::fs::remove_file` 不支持目录 |
| 跨卷移动 | `move_file()` (icon_hider.rs:38-48) | `std::fs::copy` + `remove_file` 不支持目录 |

## 修复方案

在 `icon_hider.rs` 中新增两个目录感知辅助函数：
1. `copy_path_recursive(src, dst)` — 文件用 `fs::copy`，目录用 `fs::create_dir_all` + 递归复制
2. `remove_path(path)` — 文件用 `fs::remove_file`，目录用 `fs::remove_dir_all`

替换所有受影响的调用点。

## 验收标准
- [ ] 从资源管理器拖拽文件夹到分类框，文件夹出现在分类框中
- [ ] 从桌面拖拽文件夹到分类框，文件夹出现在分类框中
- [ ] 从分类框拖拽文件夹到桌面，文件夹出现在桌面
- [ ] 从分类框拖拽文件夹到资源管理器，文件夹出现在资源管理器
- [ ] 多选拖拽包含文件夹时正常工作
- [ ] 原有文件拖拽行为不受影响
- [ ] `cargo fmt` / `cargo clippy` / `cargo test` 通过

## 技术笔记
- `std::fs::rename` 在同卷时对目录有效，不需要修改
- 跨卷移动（`move_file`）的回退路径需要改为递归复制 + 递归删除
- 修改文件：`icon_hider.rs`、`drag_drop.rs`、`overlay_drag_ops.rs`、`drag_handler.rs`
