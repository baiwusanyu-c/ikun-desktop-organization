# 桌面图标智能管理

## 需求

应用启动时，仅隐藏分类框内的图标，不在分类框内的图标保持桌面可见。
支持桌面⇄分类框双向拖拽，不需要管理员权限。

### 用户故事

| 场景 | 期望行为 |
|------|---------|
| 启动应用 | 分类框内的图标从桌面消失，其他图标保持原位 |
| 退出应用 | 所有图标恢复桌面原位 |
| 拖桌面图标到分类框 | 图标从桌面消失，出现在分类框内 |
| 拖分类框图标到桌面 | 图标出现在桌面释放位置，从分类框消失 |
| 一键 AI 分类 | 所有桌面图标按类别分到多个框，全部从桌面隐藏 |
| 清除分类 | 所有图标恢复桌面 |

---

## 现状分析

### 当前组件

| 组件 | 文件 | 功能 | 状态 |
|------|------|------|------|
| ShowWindow 隐藏 | `icon_hider.rs` | `ShowWindow(SW_HIDE/SW_SHOW)` 隐藏/显示整个 SysListView32 | ✅ 可用，但隐藏全部图标 |
| Vault 仓库 | `icon_hider.rs` | `.lnk` 文件移动/复制到 `~/.ikun_organizer/managed_icons/` | ✅ 可用，拖拽时使用 |
| DesktopIconManager | `icon_manager.rs` | 跨进程 LVM 操作逐图标移屏/恢复 | ⚠️ 代码完整，0x052C 修复已合入，**未测试** |
| 自定义拖拽（框→外） | `drag_handler.rs` | SetCapture + WS_EX_LAYERED 覆盖窗口 | ✅ 可用 |
| OLE Drop Target（外→框） | `drop_target.rs` + `wnd_proc.rs` | IDropTarget + WM_USER_DRAG_DROP | ✅ 已接通 |
| OLE Drag Source（框→外） | `ole_dnd.rs` | IDataObject + IDropSource + DoDragDrop | ⚠️ 代码完整，未启用 |
| 分类框窗口 | `grid_window.rs` | WS_POPUP 分层窗口，D2D/DirectWrite 渲染 | ✅ 可用 |

### 当前问题

1. **ShowWindow 隐藏全部图标** — 不在分类框内的图标也被隐藏，不满足选择性隐藏需求
2. **DesktopIconManager 未验证** — 0x052C 修复已合入但用户在测试中 SysListView32 始终找不到
3. **框→桌面拖拽使用自定义 SetCapture** — 不携带 CF_HDROP 数据，桌面无法识别为文件拖放
4. **退出恢复时所有 Vault 图标导出到用户桌面** — 公共桌面图标产生重复副本

### 行业调研

| 产品 | 技术方案 | 核心原理 |
|------|---------|---------|
| **腾讯桌面整理 (DeskGo)** | 覆盖窗口 | 创建透明窗口覆盖在桌面图标之上，视觉分组但不操作 SysListView32 |
| **Stardock Fences** | Shell Extension + 覆盖窗口 | DLL 注入 explorer.exe + 半透明覆盖窗口分组 |
| **NoFences (开源)** | 透明覆盖窗口 | Windows Forms 透明窗口覆盖 |

**共同特点**：都不移动/删除文件，用覆盖窗口实现视觉分组。
**关键区别**：它们是在桌面上分组显示图标，而我们需要将图标"收纳"到独立分类框窗口中。

---

## 方案对比

### 方案 A：LVM 逐图标移屏 + Vault 持久化

**原理**：`LVM_SETITEMPOSITION` 将管理图标移到屏幕外 (-32000,-32000)，Vault 持久化文件状态。

```
启动: rebuild_index → hide managed icons (LVM off-screen) → vault ingest (user desktop)
退出: vault export → show managed icons (LVM back to saved position)
桌面→框: OLE drop → vault ingest + LVM hide
框→桌面: export vault + LVM show at drop position
```

| 优点 | 缺点 |
|------|------|
| 精确控制单个图标 | 依赖 SysListView32 可访问性 |
| 不需要管理员权限 | "自动排列"开启时需临时关闭 |
| 文件不移动，Explorer 不刷新 | explorer 重启后位置丢失 |
| 与腾讯桌面/Fences 类似的底层技术 | 跨进程内存操作可能失败 |

**前置条件**：`find_desktop_listview()` 必须可靠工作（0x052C 修复已合入，待验证）。

### 方案 B：ShowWindow 全隐藏 + LVM 选择性恢复

**原理**：先 `ShowWindow(SW_HIDE)` 隐藏全部图标，再用 LVM 将非管理图标移回原位并显示。

```
启动: ShowWindow(HIDE) → LVM show unmanaged icons at saved positions
退出: LVM hide unmanaged → ShowWindow(SHOW) → restore positions
```

| 优点 | 缺点 |
|------|------|
| ShowWindow 可靠性高 | 需要区分"管理"和"非管理"图标 |
| 管理图标确定不可见 | 非管理图标需要保存/恢复位置 |
| 即使 LVM 部分失败，管理图标仍隐藏 | "自动排列"可能干扰恢复 |

### 方案 C：覆盖窗口（腾讯/Fences 方式）

**原理**：在桌面图标之上创建透明覆盖窗口，视觉遮挡管理图标。

| 优点 | 缺点 |
|------|------|
| 不修改 Shell 状态 | 需要精确定位每个图标的屏幕坐标 |
| 不受"自动排列"影响 | 图标位置变化时覆盖错位 |
| 最稳定可靠 | 实现复杂度高（需要持续跟踪图标位置） |
| | 覆盖窗口可能被用户操作穿透 |

### 选定方案（当前实现）：ShowWindow 全隐藏 + DesktopOrgOverlay 渲染未管理图标

**阶段 2-6 验证结果**：`LVM_SETITEMPOSITION` 在 Windows 11 桌面图标视图中会被 Shell 自动重排干扰，不能作为稳定的逐图标显示/隐藏方案。当前采用更接近 DeskGo/Fences 的桌面接管方式：隐藏原生 `SysListView32`，用自定义 `DesktopOrgOverlay` 重新绘制未进入分类框的桌面图标，分类框窗口独立绘制已管理图标。

**原理**：
1. `ShowWindow(SW_HIDE)` 隐藏整个 SysListView32（全部原生桌面图标不可见）
2. `DesktopOrgOverlay` 渲染**未管理**图标，并提供点击、双击、右键、拖拽交互
3. 分类框窗口用自己的 D2D 渲染管理图标副本
4. 桌面 ⇄ 分类框拖拽通过 Vault 仓库持久化，无需管理员权限

```
启动流程:
  1. restore_all_hidden_icons() 初始化 managed_icons 仓库
  2. hide_native_icons() → ShowWindow(HIDE) 隐藏原生 SysListView32
  3. create_desktop_overlay() → 渲染未管理图标
  4. create_grid_window(...) → 恢复分类框

退出流程:
  1. collect_grid_layout() → 保存分类框布局
  2. merge_latest_overlay_config() → 合并覆盖窗口最新图标位置
  3. destroy_all_grid_windows()
  4. destroy_desktop_overlay()
  5. restore_native_icons() → ShowWindow(SHOW) 恢复原生图标
```

| 优点 | 缺点 |
|------|------|
| ShowWindow 可靠性高（已验证） | 需要维护覆盖窗口图标渲染/交互 |
| 不依赖不稳定的 LVM 逐图标隐藏 | 多显示器暂时仍需扩展为每屏一个覆盖窗口 |
| 支持非管理图标可见和位置持久化 | Explorer 重启后需重新隐藏新 SysListView32 |
| Vault 提供无管理员拖拽持久化 | |

---

## 实现计划（当前阶段状态）

### 阶段 1：验证 DesktopIconManager ✅ 已完成

验证结果：LVM 可用，22 个图标全部索引成功。

### 阶段 2：桌面覆盖窗口渲染未管理图标 ✅ 已完成

**文件**：`src/platform/windows/desktop_overlay/*`

- 新增 `DesktopOrgOverlay` 分层窗口，创建在桌面层级之上、分类框之下
- 从桌面枚举结果中排除 `layout.boxes[*].icon_names` 管理图标
- 使用 Direct2D/DirectWrite + DIB/`UpdateLayeredWindow()` 渲染未管理图标
- 空白区域保持穿透，避免遮挡分类框和普通应用

### 阶段 3：覆盖窗口图标交互 ✅ 已完成

**文件**：`overlay_wnd_proc.rs`、`overlay_render.rs`、`drag_handler.rs`

- 支持 hover / 单击选中 / 双击打开 / 右键 Shell 文件菜单
- 支持覆盖窗口图标拖入分类框
- 支持分类框图标拖回覆盖窗口任意位置
- 拖拽时复用自定义分层拖拽图像，避免管理员权限和 OLE 权限问题

### 阶段 4：Vault 持久化与无管理员双向移动 ✅ 已完成

**文件**：`icon_hider.rs`、`drag_handler.rs`

- 用户桌面图标拖入分类框：移动到 `~/.ikun_organizer/managed_icons/`
- 公共桌面图标拖入分类框：复制到 Vault，保留公共桌面原文件
- 分类框图标拖回桌面：导出到用户桌面或覆盖窗口释放位置

### 阶段 5：覆盖窗口图标位置持久化 ✅ 已完成

**文件**：`app_config.rs`、`overlay_window.rs`、`app.rs`

- 新增 `AppConfig.overlay.icon_positions`
- 覆盖窗口内拖动图标后保存逻辑坐标（96 DPI 基准）
- 应用保存布局前合并磁盘上的最新 overlay 配置，避免退出时覆盖位置

### 阶段 6：覆盖窗口与应用生命周期绑定 ✅ 已完成

**文件**：`app.rs`、`icon_hider.rs`

- 启动时：初始化 Vault → 隐藏原生 `SysListView32` → 创建 `DesktopOrgOverlay` → 恢复分类框
- 空布局时仍创建覆盖窗口，保持桌面接管一致性
- 退出时：保存布局 → 销毁分类框 → 停止桌面文件监听 → 销毁覆盖窗口 → 恢复原生桌面图标 → 清理注册表菜单
- 新增 `native_icons_hidden()`，通过 `IsWindow` + `IsWindowVisible` 检测 Explorer 重启或原生图标窗口恢复
- 运行中检测到原生图标窗口恢复后，重新隐藏并保持覆盖窗口运行

### 阶段 7：桌面文件监听刷新 ✅ 已完成

**文件**：`desktop_overlay/desktop_watcher.rs`、`desktop_overlay/overlay_window.rs`

- 使用 `notify` 监听用户桌面和公共桌面目录
- 对新增、删除、重命名、修改事件进行约 500ms debounce
- debounce 后通过 `WM_USER_REFRESH_OVERLAY` 回到覆盖窗口线程刷新图标列表
- 刷新时复用 `collect_overlay_icons()` 重新过滤 managed icons，并保留 `overlay.icon_positions` 位置持久化
- 覆盖窗口销毁时停止监听线程，避免退出后残留后台监听

### 阶段 8：后续增强（待实现）

- 多显示器：为每个显示器创建覆盖窗口并按显示器保存/恢复位置
- Explorer 重启后重建覆盖窗口图标列表和窗口层级
- 桌面空白右键菜单在覆盖窗口接管模式下的完整转发/显示

### 修改文件清单

| 文件 | 修改 |
|------|------|
| `app.rs` | 阶段 6 生命周期编排：启动接管、退出恢复、Explorer 恢复检测、overlay 配置合并 |
| `config/app_config.rs` | 新增 `OverlayConfig` 与位置持久化单元测试 |
| `platform/mod.rs` | 导出覆盖窗口与原生图标隐藏状态 API |
| `platform/windows/mod.rs` | 导出 `desktop_overlay`、drop target、拖拽数据和 icon hider API |
| `platform/windows/icon_hider.rs` | ShowWindow 隐藏/恢复、Vault 仓库、`native_icons_hidden()` 状态检测 |
| `platform/windows/desktop_overlay/*` | 桌面覆盖窗口创建/销毁、渲染、交互、位置保存、桌面文件监听刷新 |
| `platform/windows/drag_handler.rs` | 分类框 ⇄ 覆盖窗口拖拽和 Vault 导入/导出 |
| `platform/windows/drop_target.rs` | OLE/自定义拖拽数据通道 |

---

## 验收标准

- [x] 启动后分类框内图标不在原生桌面显示，未管理图标由 `DesktopOrgOverlay` 显示
- [x] 桌面/覆盖窗口图标可拖入分类框（从覆盖窗口消失 + 出现在框内）
- [x] 分类框图标可拖到桌面覆盖窗口任意位置（出现在释放位置 + 从框消失）
- [x] 退出应用后销毁覆盖窗口并恢复原生桌面图标
- [x] 运行期新增/删除/重命名桌面文件后覆盖窗口自动刷新
- [ ] 一键 AI 分类：所有图标分到对应框，全部从桌面隐藏
- [ ] 多显示器每屏覆盖窗口
- [x] 不需要管理员权限
- [x] 所有操作有中文日志
