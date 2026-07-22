# 图标位置保存修复 - 第二版

## 问题修正

第一版修复失败，因为：
- 在 `ingest_icon` 中尝试从 SysListView32 读取位置时，图标已经不在桌面上了
- 图标从覆盖窗口拖到分类框时，已经被我们的程序管理，不在 SysListView32 中

## 正确方案

### 根因分析（修正版）

1. **覆盖窗口管理的图标**：在覆盖窗口中显示，位置保存在 `icon.x` 和 `icon.y`（逻辑像素坐标）
2. **拖到分类框时**：覆盖窗口的 `end_overlay_drag` 函数处理，图标从覆盖窗口移除
3. **程序退出时**：`restore_all_hidden_icons` 将仓库中的图标还原到桌面，但 Windows 不知道原始位置

### 解决方案

在**覆盖窗口拖拽到分类框**时保存位置（而不是在 `ingest_icon` 中）：

1. **位置保存**（`overlay_wnd_proc.rs::end_overlay_drag`）：
   - 在从覆盖窗口移除图标之前，获取其逻辑像素坐标 `(icon.x, icon.y)`
   - 转换为屏幕坐标：`screen_x = window.x + physical_x`（其中 `physical_x = icon.x * dpi / 96`）
   - 保存屏幕坐标到 `vault_icon_positions` 配置

2. **位置恢复**（`icon_hider.rs::restore_all_hidden_icons`）：
   - 还原文件后等待 500ms 让 Windows 识别
   - 调用 `position_icon_at_screen` 将图标移回保存的屏幕位置
   - 该函数会自动处理查找图标和坐标转换

### 坐标系统

- **覆盖窗口坐标**：逻辑像素（96 DPI 基准），相对于工作区左上角
- **屏幕坐标**：物理像素，相对于主显示器左上角
- **转换公式**：`screen = window_offset + logical * dpi / 96`

### 修改的文件

1. **src/config/app_config.rs**
   - 添加 `vault_icon_positions: HashMap<String, [f32; 2]>` 字段

2. **src/platform/windows/desktop_overlay/overlay_wnd_proc.rs**
   - 在 `end_overlay_drag` 中，移除图标前保存屏幕位置

3. **src/platform/windows/icon_hider.rs**
   - 添加 `restore_icon_positions()` 函数
   - 修改 `restore_all_hidden_icons()` 在还原后恢复位置
   - 移除 `ingest_icon` 中的位置保存逻辑（已移到覆盖窗口处理）

4. **src/platform/windows/icon_manager.rs**
   - 将 `get_icon_position()` 和 `set_icon_position()` 改为 `pub`（虽然当前未使用）

### 测试步骤

```powershell
cargo run
```

**场景1：从覆盖区域拖拽到分类框**
1. 观察桌面上的图标（如 Docker Desktop），记住其位置
2. 将图标从覆盖区域拖拽到分类框
3. 退出程序
4. **预期**：图标应该回到原来的桌面位置

**场景2：从分类框拖到另一个分类框**
1. 将图标从分类框A拖到分类框B
2. 退出程序
3. **预期**：图标保持在分类框B中（不涉及桌面位置）

### 日志验证

查看 `~/.ikun_organizer.log`，应该看到：

```
[DIAG] 覆盖窗口→分类框: 保存 'C:\...\Docker Desktop.lnk' 屏幕位置 (340, 620) [逻辑: (170, 310), 工作区: (0, 0)]
...
[DIAG] restore_all_hidden_icons: 还原=1, 跳过=0
[DIAG] restore_icon_positions: 尝试恢复 'C:\...\Docker Desktop.lnk' 到屏幕位置 (340, 620)
桌面图标已定位到 (340, 620) [客户区 (340, 620)]
```

### 已知限制

1. **500ms 等待时间**：硬编码等待 Windows 识别还原的图标
2. **多显示器**：如果工作区跨多个显示器，坐标转换可能有问题
3. **DPI 变化**：如果 DPI 设置改变，保存的屏幕坐标可能不准确

### 后续优化

1. 使用轮询检测图标是否已出现，而非固定等待
2. 添加位置恢复失败时的重试机制
3. 考虑保存相对位置（相对于工作区）而非绝对屏幕坐标

## 下一步

请测试并反馈结果。如果还有问题，请提供完整的日志文件。

