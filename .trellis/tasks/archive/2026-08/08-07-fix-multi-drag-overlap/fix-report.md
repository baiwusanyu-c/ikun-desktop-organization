# 修复报告：多选拖拽图标重叠 Bug

## 问题描述
从桌面/资源管理器多选图标拖拽到分类框时，所有图标重叠在鼠标释放的同一个单元格上。

## 根因分析
在 `src/platform/windows/drag_drop.rs` 的 `handle_ole_drop` 函数中，**insert mode 的 shift 逻辑放在了 per-file 循环内部**：

```rust
// 修复前（错误）
for file_path in &drop.file_paths {
    // ... 计算位置 ...
    if is_insert {
        shift_existing_items_by_1();  // ← 每次迭代都移 1 位
    }
    push_item();
}
```

**问题**：每次迭代只右移 1 位，但之前迭代新增的项也会被错误移位，导致多个新项重叠在同一位置。

### 具体推演（现有 2 项 + 拖入 2 个新文件, insert_lin=1）

| 迭代 | 操作 | 结果 |
|------|------|------|
| 初始 | A@0, B@1 | A@0, B@1 |
| #1 | shift ≥ 1 | A@0, B@2 |
| #1 | push X@1 | A@0, B@2, X@1 |
| #2 | shift ≥ 1 | A@0, B@3, **X@2** ← 被错误移位 |
| #2 | push Y@2 | A@0, B@3, X@2, **Y@2** 💥 重叠 |

## 修复方案

将 shift 逻辑**移到循环外部**，一次性将所有现有项右移 **N 位**（N = 有效文件数）：

```rust
// 修复后（正确）
let valid_count = count_valid_files();  // 预计算
if is_insert {
    materialize_positions();
    shift_existing_items_by_N(valid_count);  // ← 一次性腾出 N 个位置
}
for file_path in &drop.file_paths {
    push_item();  // 位置已由 offset_lin 保证唯一
}
```

## 代码变更

**文件**: `src/platform/windows/drag_drop.rs`  
**函数**: `handle_ole_drop`  
**变更**: +37 行, -24 行

### 新增代码（第 914-948 行）
```rust
// [FIX] Insert mode: pre-shift existing items right by N positions (one-time)
let is_insert = drop.insert_col >= 0 && drop.insert_row >= 0;
if is_insert {
    let insert_lin = drop.insert_row * cols + drop.insert_col;
    let valid_count = drop
        .file_paths
        .iter()
        .filter(|fp| !data.items.iter().any(|i| i.file_path == **fp))
        .count() as i32;

    // Materialize all auto-layout positions
    for (i, item) in data.items.iter_mut().enumerate() {
        if item.grid_col < 0 {
            item.grid_col = (i as i32) % cols;
            item.grid_row = (i as i32) / cols;
        }
    }

    // Shift existing items at or after insert position right by N
    for item in data.items.iter_mut() {
        let item_lin = item.grid_row * cols + item.grid_col;
        if item_lin >= insert_lin {
            let new_lin = item_lin + valid_count;
            item.grid_col = new_lin % cols;
            item.grid_row = new_lin / cols;
        }
    }
}
```

### 删除代码
循环内的旧 `if is_insert { ... }` shift 块（原第 1022-1044 行）

## 验证结果

✅ **cargo check**: 通过（无编译错误）  
✅ **cargo clippy**: 通过（drag_drop.rs 无新警告）

## 测试建议

### 手动测试用例
1. **多选拖拽（正常模式）**: 选择 3-5 个图标 → 拖到分类框空白区域 → 图标应依次排列在连续单元格
2. **多选拖拽（插入模式）**: 选择 3-5 个图标 → 拖到现有图标左侧 30% 区域 → 图标应插入到指定位置，现有项正确右移
3. **单文件拖拽**: 行为不变（N=1 等价于原逻辑）
4. **框→框拖拽**: 行为不变

### 预期结果
- ✅ 无图标重叠
- ✅ 图标按网格布局排布
- ✅ 插入模式下现有项正确右移 N 位

## 影响范围

- **修复场景**: 多选拖拽到分类框（insert mode）
- **不受影响**: 
  - 单文件拖拽（N=1，行为等价）
  - 正常模式拖拽（已有 offset 逻辑）
  - 框→框拖拽（独立逻辑）

## 相关文件

- `src/platform/windows/drag_drop.rs` - 主要修改
- `src/platform/windows/drop_target.rs` - IDropTarget::Drop 入口（未修改）
- `src/platform/windows/drag_visual.rs` - 拖拽可视化反馈（未修改）

## 提交建议

```
fix(drag): 修复多选拖拽到分类框时图标重叠问题

将 insert mode 的 shift 逻辑从 per-file 循环内移到循环外，
一次性右移 N 位（N=有效文件数），避免后续迭代错误移位
导致多个新项重叠在同一位置。

- 预计算 valid_count（去重后的文件数）
- 循环前一次性物化位置并右移现有项 N 位
- 删除循环内的旧 shift 块

Fixes: 多选拖拽图标重叠 bug
```
