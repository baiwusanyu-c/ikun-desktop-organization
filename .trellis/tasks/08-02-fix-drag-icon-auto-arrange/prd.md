# 修复分类框拖拽图标到桌面偶发定位失败问题

## 目标
修复从分类框拖拽图标到桌面时，图标偶发不出现在鼠标释放位置的问题。确保图标始终精确定位在用户释放鼠标的位置。

## 已知信息

### 问题表现
- 从分类框拖拽图标到桌面释放后，图标**偶发**不出现在鼠标释放位置
- 图标出现在桌面的其他位置（可能是 Shell 自动分配的"下一个空位"）
- 此问题已尝试多轮修复，仍未解决
- **关键发现**：问题复现时桌面**没有开启"自动排列图标"**

### 当前代码架构（从代码库分析）

**拖拽路径**：分类框→桌面使用的是**自定义 Timer 拖拽**（非 OLE）
- Timer 轮询检测鼠标位置（`handle_grid_drag_timer`，8ms 间隔）
- 当鼠标不在任何分类框上且不在 Explorer 窗口上时，走自定义拖拽路径
- 鼠标释放时调用 `end_drag` → `handle_box_to_desktop`

**`handle_box_to_desktop` 当前流程**（`src/platform/windows/drag_drop.rs` L243-318）：
```rust
1. export_icon() — 将文件从仓库复制到桌面
2. 启动后台线程，sleep 100ms
3. 后台线程调用 disable_desktop_auto_arrange() — 通过 COM IFolderView2 关闭自动排列
4. 调用 position_icon_at_screen() — 内部再次调用 disable/position/restore
5. 后台线程调用 restore_desktop_auto_arrange()（如果之前禁用了）
```

**`position_icon_at_screen` 内部流程**（`src/platform/windows/icon_manager.rs` L1452-1520）：
```rust
1. disable_desktop_auto_arrange() — 返回 Some(false)（因为没开）
2. find_listview_at_screen_point() — 查找 SysListView32
3. DesktopIconManager::with_listview() — 创建管理器
4. 重试查找图标（20 次 × 50ms）— 等待 Shell 创建 ListView 项
5. show_icon_at() — 通过 LVM_SETITEMPOSITION 设置位置
6. 不调用 restore（因为本来就没开自动排列）
```

## 根因分析（更新）

### ? 已排除：自动排列竞争
用户确认问题复现时**没有开启自动排列图标**，所以：
- `disable_desktop_auto_arrange()` 返回 `Some(false)`
- 不会调用 `restore_desktop_auto_arrange()`
- 自动排列不是问题

### ? 真正的根因：Shell 异步定位竞争

**当文件被复制到桌面文件夹时，Windows Shell 会异步执行以下操作**：
1. 文件系统监听（SHChangeNotify）检测到新文件
2. **异步创建** SysListView32 项
3. **自动定位**图标到"下一个可用位置"（按 Shell 内部规则，通常是按列从上到下、从左到右）
   - ?? 这不需要"自动排列"开启！这是 Shell 对新文件的默认行为

**竞争时序**：
```
时间线 →
├─ [我们] export_icon() 复制文件
├─ [Shell] 检测到文件变化，开始异步处理
├─ [我们] sleep 100ms
├─ [Shell] 创建 ListView 项，定位到"下一个空位"
├─ [我们] position_icon_at_screen() → LVM_SETITEMPOSITION
│   ├─ 情况 A：我们的定位先完成 → Shell 的定位后覆盖 → ? 失败
│   └─ 情况 B：Shell 的定位先完成 → 我们的定位成功 → ? 成功
```

**偶发原因**：
- Shell 的异步处理时间不确定（取决于系统负载、文件数量等）
- 有时 Shell 快，有时我们快 → 偶发成功/失败

### ?? 次生根因：100ms 延迟不足

当前代码只等待 100ms，但 Shell 可能需要更长时间：
- 文件复制到桌面后，Shell 需要：读取文件信息 → 提取图标 → 创建 ListView 项 → 定位
- 这个流程可能需要 200-500ms，甚至更长
- 虽然有重试机制（20 次 × 50ms = 1000ms），但重试是在**查找图标**阶段，不是在**定位后验证**阶段

### ?? 次生根因：缺少定位后验证

当前代码：
```rust
mgr.show_icon_at(index, client_pt.x, client_pt.y)?;
log::info!("桌面图标已定位到 ({}, {})", screen_x, screen_y);
```

定位后**没有验证**图标是否真的在目标位置，也没有检查是否被 Shell 覆盖。

## 临时假设
- Shell 的异步定位是主要根因
- 100ms 延迟不足是次要根因
- 缺少定位后验证是第三个根因

## 待解决问题
（暂无，需要与用户确认修复策略）

## 方案对比

### 方案 A：增加延迟 + 后置定位 + 验证重试（推荐）

**核心思路**：给 Shell 更多时间完成定位，然后定位并验证，被覆盖则重试。

**具体实现**：
```rust
std::thread::spawn(move || {
    // 1. 增加延迟：给 Shell 更多时间
    std::thread::sleep(std::time::Duration::from_millis(300));
    
    // 2. 定位图标
    position_icon_at_screen(&file_path_str, target_x, target_y);
    
    // 3. 验证+重试
    for attempt in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        
        // 读取实际位置
        let actual_pos = get_icon_position_by_name(&file_name);
        let (actual_x, actual_y) = actual_pos.unwrap_or((-1, -1));
        
        // 检查是否偏移（允许 20px 误差）
        let dx = (actual_x - target_x).abs();
        let dy = (actual_y - target_y).abs();
        
        if dx > 20 || dy > 20 {
            log::warn!("图标位置偏移: 期望 ({}, {}), 实际 ({}, {})", 
                       target_x, target_y, actual_x, actual_y);
            // 重新定位
            position_icon_at_screen(&file_path_str, target_x, target_y);
        } else {
            log::info!("图标定位成功");
            break;
        }
    }
});
```

**优点**：
- 增加延迟（300ms）给 Shell 更多时间
- 定位后验证，确保成功
- 被覆盖时自动重试（最多 3 次）
- 不需要修改 `position_icon_at_screen` 内部逻辑

**缺点**：
- 后台线程存活时间更长（~1s）
- 用户可能看到图标短暂出现在错误位置

**适用场景**：大多数情况，特别是 Shell 异步定位不稳定的场景

### 方案 B：完全后置定位（方案 C 的简化版）

**核心思路**：完全放弃与 Shell 竞争，等 Shell 完成后再定位。

**具体实现**：
```rust
std::thread::spawn(move || {
    // 1. 长延迟：确保 Shell 已完成
    std::thread::sleep(std::time::Duration::from_millis(500));
    
    // 2. 轮询等待图标出现（最多 5s）
    let mut found = false;
    for _ in 0..50 {
        if icon_exists_in_desktop(&file_path_str) {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    
    if !found {
        log::warn!("图标未出现在桌面");
        return;
    }
    
    // 3. 定位（Shell 已完成，不会覆盖）
    position_icon_at_screen(&file_path_str, target_x, target_y);
});
```

**优点**：
- 完全避免与 Shell 竞争
- 定位成功率最高

**缺点**：
- 用户会看到图标先出现在错误位置，然后跳到正确位置（延迟 500ms-1s）
- 用户体验稍差

**适用场景**：方案 A 仍有问题时

### 方案 C：使用 Shell API 直接定位（长期方案）

**核心思路**：使用 `IFolderView::SelectAndPositionItems` 或 `SHAddToRecentDocs` 等 Shell API，让 Shell 自己定位，而不是通过 LVM 消息。

**优点**：
- 最稳定的方案，与 Shell 完全同步
- 不需要轮询和重试

**缺点**：
- 需要实现 COM 接口，代码量大
- 需要调研 Shell API 的具体用法

**适用场景**：长期优化，方案 A/B 都不稳定时

## 推荐路径

**第一阶段：实施方案 A**
1. 增加延迟到 300ms
2. 添加定位后验证逻辑
3. 验证失败时重试（最多 3 次）
4. 添加详细日志，记录每次定位的结果

**第二阶段（如果仍有问题）：叠加方案 B**
1. 增加延迟到 500ms
2. 轮询等待图标出现
3. 确保 Shell 完成后再定位

**第三阶段（长期）：调研方案 C**
1. 研究 Shell API 的定位能力
2. 如果可行，替换 LVM 消息方式

## 需求
1. 图标从分类框拖拽到桌面释放后，始终出现在鼠标释放位置
2. 无论桌面是否开启"自动排列图标"，都应正确定位
3. 定位失败时优雅降级（图标出现在默认位置，不丢失）
4. 后台线程不应阻塞主线程或影响应用响应

## 验收标准
- [ ] 连续拖拽 20 次图标到桌面不同位置，每次都出现在释放位置（允许 1 格误差，约 20px）
- [ ] 桌面关闭"自动排列图标"时，拖拽后图标正确定位（主要测试场景）
- [ ] 桌面开启"自动排列图标"时，拖拽后图标正确定位（次要测试场景）
- [ ] 定位失败时图标不丢失，出现在桌面默认位置
- [ ] 后台线程在 2s 内完成，不长时间占用资源
- [ ] 日志中可以看到定位结果（成功/重试次数）

## 完成定义（团队质量门槛）
* 添加/更新测试（单元/集成）
* Lint / typecheck / CI 通过
* 行为变更时更新文档/笔记

## 明确排除范围
- 不改变分类框内部的拖拽逻辑（同框/框间拖拽）
- 不改变 OLE 拖拽到 Explorer 文件管理器的逻辑
- 不改变桌面图标的隐藏/显示机制
- 不改变文件仓库（vault）的存储结构

## 技术笔记

### 关键文件
- `src/platform/windows/drag_drop.rs` — `handle_box_to_desktop`（定位逻辑入口，L243-318）
- `src/platform/windows/icon_manager.rs` — `position_icon_at_screen`（L1452-1520）、`DesktopIconManager`、`get_icon_position_by_name`（待实现）
- `src/platform/windows/icon_hider.rs` — `export_icon`（文件导出到桌面）
- `src/platform/windows/drag_handler.rs` — `end_drag`（拖拽结束调度）

### 关键 Windows API
- `LVM_SETITEMPOSITION` — 设置 SysListView32 项位置（当前使用）
- `LVM_GETITEMPOSITION` — 读取 SysListView32 项位置（用于验证）
- `SHChangeNotify` — Shell 文件变化通知（触发 ListView 更新）
- `IFolderView::SelectAndPositionItems` — Shell API 定位（备选方案）

### 风险点
- Shell 异步定位的时间不确定，可能需要更长的延迟
- 多显示器场景下坐标转换可能出错（需要验证 `ScreenToClient`）
- 后台线程与 Shell 的时序竞争可能仍然存在（方案 A 的重试机制应能缓解）

### 诊断日志（新增）
在 `handle_box_to_desktop` 和 `position_icon_at_screen` 中添加以下日志：
- 文件导出时间
- 后台线程启动时间
- 图标在 SysListView32 中出现的时间
- 每次定位的目标位置和实际位置
- 重试次数
- 最终定位结果（成功/失败）
