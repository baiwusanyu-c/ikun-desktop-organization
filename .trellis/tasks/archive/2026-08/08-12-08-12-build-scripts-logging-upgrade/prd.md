# 打包脚本重构 + 日志架构升级

## 目标
为 Tauri 2 架构创建基于 Node.js 的跨平台打包脚本，升级日志系统支持三窗口标签和前端 IPC 日志上报，并对 b21a9fe4..2bbf02b3 的代码变更进行架构自检。

## 已知信息
- 项目原有 `scripts/` 目录不存在，无独立打包脚本
- `.github/workflows/build.yml` 引用不存在的二进制名
- 日志系统已有 2MB 分片和 7 天清理，但缺少窗口标签
- b21a9fe4..2bbf02b3 包含 11 个提交（退出修复、液态玻璃、UI 样式等）

## 需求
1. **打包脚本**：Node.js 跨平台脚本 `scripts/build.mjs`，支持 `--mode`/`--no-installer`/`--no-frontend`/`--target`
2. **日志升级**：`WindowLogGuard` RAII 标记窗口来源，`log_message` Tauri Command 接收前端日志
3. **架构自检**：检查 b21a9fe4..2bbf02b3 变更是否符合 CLAUDE.md + .trellis/spec 规范
4. **文档同步**：更新 CLAUDE.md、logging-guidelines.md、directory-structure.md、tauri-svelte-architecture.md
5. **单元测试**：5 个窗口上下文测试

## 验收标准
- [x] `cargo fmt` 通过
- [x] `cargo check` 编译成功
- [x] `cargo test` 515 tests passed
- [x] 打包脚本 `scripts/build.mjs` 可用
- [x] 日志窗口标签机制正确工作
- [x] 架构文档已同步

## 明确排除范围
- 不引入新的前端测试框架
- 不改变现有日志文件路径/命名规则
- 不修改日志 7 天清理策略（仅启动时执行）
