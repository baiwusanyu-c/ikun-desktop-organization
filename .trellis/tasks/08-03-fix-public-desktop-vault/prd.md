# 修复 Public 桌面图标收纳路径与分类框显示

## 目标

修复需要管理员权限的 Public 桌面图标收纳流程：公共仓库根目录固定为 `C:\Users\Public\.ikun_organizer`，图标移入分类框后从 Public 桌面消失，并能立即及重启后在分类框中正常显示。

## 已知信息

- Public 桌面目录为 `C:\Users\Public\Desktop`。
- 当前 `public_app_data_root()` 错误地通过 Public 桌面目录拼接 `.ikun_organizer`，生成了 `C:\Users\Public\Desktop\.ikun_organizer`。
- Public 图标通过提升权限的 `IFileOperation` 移入公共仓库；当前用户图标通过普通文件移动进入用户仓库。
- `vault_path_for()`、`resolve_icon_read_path()`、`icon_exists_for_box()` 与 `list_vault_icons()` 仍只识别当前用户仓库，导致 Public 图标移动后无法从分类框读取，重启时也会被判定为缺失。
- 相关实现集中在 `src/platform/windows/icon_hider.rs`，拖放调用链位于 `src/platform/windows/drag_drop.rs`。

## 临时假设

- Public 图标继续保留原逻辑桌面路径 `C:\Users\Public\Desktop\<文件>`，仅实际读取路径切换到公共仓库。
- 公共仓库沿用独立目录，因此文件名不需要 `public__` 前缀；来源由仓库位置确定。
- 已经误放到旧目录的文件需要兼容读取和迁移，避免升级后丢失已管理图标。

## 待解决问题

- 无阻断问题；用户已明确要求创建任务并修复该行为。

## 需求

- Public 仓库根目录必须为 `C:\Users\Public\.ikun_organizer`，管理目录为其下的 `managed_icons`。
- Public 图标收纳成功后，原 `C:\Users\Public\Desktop` 文件必须消失。
- Public 拖放不能被 UAC/权限操作同步阻塞；图标先加入分类框，收纳操作在 drop 回调结束后异步执行。
- 分类框使用 Public 图标的逻辑桌面路径保存配置，同时能解析到公共仓库中的实际文件。
- Public 图标在收纳后立即显示，应用重启后仍显示。
- 用户桌面图标仓库行为保持不变。
- 当前用户图标必须继续使用 `~\\.ikun_organizer\\managed_icons`，不改变其移动、读取、恢复、命名和权限行为。
- 兼容旧错误目录 `C:\Users\Public\Desktop\.ikun_organizer\managed_icons` 中已有文件。
- 对路径选择与逻辑路径映射增加回归测试。

## 方案对比

### 方案 A：Public 图标复制到当前用户仓库

优点：改动最少，现有读取链路天然可用。

缺点：不满足公共数据应落在 `C:\Users\Public\.ikun_organizer` 的要求，也会保留 Public 桌面原文件。

### 方案 B：按图标来源路由到用户/公共仓库（采用）

优点：路径归属正确；逻辑路径与实际读取路径分离；现有 API 可统一处理用户与 Public 图标。

缺点：需要同步修复收纳、读取、存在性判断、枚举、导出和恢复等链路。

### 方案 C：全局只使用公共仓库

优点：仓库位置唯一。

缺点：所有普通用户图标操作都可能触发 UAC，破坏现有无权限收纳体验。

## 验收标准

- [ ] 收纳 `C:\Users\Public\Desktop\APP.lnk` 后，文件实际位于 `C:\Users\Public\.ikun_organizer\managed_icons\APP.lnk`。
- [ ] 收纳成功后 Public 桌面原图标消失。
- [ ] 分类框立即显示 APP 图标，保存配置并重启后仍显示。
- [ ] `resolve_icon_read_path()` 与 `icon_exists_for_box()` 能识别 Public 公共仓库文件。
- [ ] 旧错误目录中的文件可迁移到新公共仓库，或至少可兼容读取且不丢失。
- [ ] 用户桌面图标仍使用当前用户目录 `~\.ikun_organizer\managed_icons`。
- [ ] `cargo fmt --check`、`cargo check`、相关测试和 `cargo clippy` 通过。

## 完成定义

- 添加或更新单元测试覆盖用户/Public 路由与旧目录兼容。
- 格式化、编译、测试、Clippy 通过。
- 更新 Windows 平台规范中的公共仓库约定。
- 保留手工验证清单：UAC 确认、桌面消失、分类框显示、重启恢复。

## 明确排除范围

- 不改变 macOS 行为。
- 不修改配置文件 `~/.ikun_organizer.json` 的位置。
- 不重构现有手写 `IFileOperation` COM 调用。
- 不把普通用户桌面图标迁移到公共仓库。

## 技术笔记

- 相关规范：`.trellis/spec/backend/{directory-structure,quality-guidelines,error-handling,logging-guidelines,windows-platform-guidelines}.md`。
- 路径路由应集中在 `vault_path_for()`，避免调用方各自拼接仓库路径。
- Public 仓库枚举需要显式映射回 Public 桌面逻辑路径；用户仓库仍按 `user__`/`public__` 前缀兼容历史数据。
- Shell 文件移动成功后继续通过 `SHChangeNotify` 或 IFileOperation 自带通知刷新桌面。
