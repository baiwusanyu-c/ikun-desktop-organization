# 修复多选拖拽图标重叠 Bug

## 目标
修复从桌面/资源管理器多选图标拖拽到分类框时，所有图标重叠在同一单元格的问题。期望图标按网格布局排布，禁止重叠。

## Bug 定位分析

### 代码流
```
用户拖拽 → IDropTarget::Drop (drop_target.rs:376)
  → 存储 PendingDrop (所有文件路径 + 释放坐标 + insert_col/insert_row)
  → PostMessage(WM_USER_DRAG_DROP)
  → handle_ole_drop (drag_drop.rs:866)
    → 遍历每个文件
      → 计算 final_col/final_row (带 offset)
      → 如果 insert mode: 执行 shift 操作 (BUG 在这里!)
      → 添加到 data.items
```

### 根因：`handle_ole_drop` 中 insert mode 的 shift 逻辑在循环内部 (drag_drop.rs:1000-1021)

```rust
for file_path in &drop.file_paths {
    // ... 计算 final_col/final_row ...
    if is_insert {
        // BUG: shift 在循环内部！每次迭代都会移动已添加的新项
        for item in data.items.iter_mut() {
            let item_lin = item.grid_row * cols + item.grid_col;
            if item_lin >= insert_lin {
                let new_lin = item_lin + 1;
                item.grid_col = new_lin % cols;
                item.grid_row = new_lin / cols;
            }
        }
    }
    data.items.push(new_item at final_col, final_row);
}
```

### 具体推演（2 个现有项 + 插入 2 个新文件, insert_lin=1, cols=3）

| 迭代 | 操作 | Item_0 | Item_1 | New_A | New_B |
|------|------|--------|--------|-------|-------|
| 初始 | - | lin=0 | lin=1 | - | - |
| #1 | shift lin≥1 | lin=0 | lin=2 | - | - |
| #1 | push New_A at lin=1 | lin=0 | lin=2 | lin=1 | - |
| #2 | shift lin≥1 | lin=0 | lin=3 | **lin=2** | - |
| #2 | push New_B at lin=2 | lin=0 | lin=3 | lin=2 | **lin=2** |

**结果: New_A 和 New_B 都在 lin=2，重叠！**

### 次要风险: 正常模式 base=(-1,-1) 未处理
当 `compute_drop_grid_position` 返回 (-1, -1)（释放在网格外），正常模式的 offset 计算会产生负数坐标。

## 修复方案

将 insert mode shift 逻辑移到循环外部，一次性将所有现有项右移 N 个位置：

```rust
// 1. 循环前: 计算有效文件数
let valid_count = drop.file_paths.iter()
    .filter(|fp| !data.items.iter().any(|i| i.file_path == **fp))
    .count() as i32;

// 2. 循环前: 物化位置 + 一次性右移 N 位
if is_insert {
    for (i, item) in data.items.iter_mut().enumerate() { /* 物化 */ }
    for item in data.items.iter_mut() {
        let item_lin = item.grid_row * cols + item.grid_col;
        if item_lin >= insert_lin {
            let new_lin = item_lin + valid_count;  // 移 N 位而非 1 位
            item.grid_col = new_lin % cols;
            item.grid_row = new_lin / cols;
        }
    }
}

// 3. 循环内: 移除 shift 逻辑，只计算位置并添加
```

## 需要修改的文件
- `src/platform/windows/drag_drop.rs` — handle_ole_drop 函数重构

## 验收标准
- [ ] 多选拖拽到分类框（正常模式）: 图标依次排列在连续单元格
- [ ] 多选拖拽到分类框（插入模式）: 图标插入到指定位置，现有项正确右移，无重叠
- [ ] 单文件拖拽: 行为不变
- [ ] 框→框拖拽: 行为不变
- [ ] cargo check / cargo clippy 通过

## 技术笔记
- `handle_external_drop` (drop_target.rs:845) 是死代码，不被调用
- `handle_ole_drop` (drag_drop.rs:866) 是实际处理函数
- 正常模式已有 offset 逻辑（`base_lin + added`），无需修改
