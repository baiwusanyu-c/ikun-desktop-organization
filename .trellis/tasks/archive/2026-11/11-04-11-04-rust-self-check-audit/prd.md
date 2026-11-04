# Rust 项目自检：重构后过度设计/逻辑bug/规范符合性审计与修复

## 目标
对 20926b54（移除插件体系/legacy UI/macOS 预留，收敛 Windows-only Tauri 运行时）到 2bc9dfcf（v0.1.0-beta.7，接入 liquid-verre-rs 重构分类框渲染）区间的重构成果做全面自检：
1. 是否存在过度设计（假设性抽象、死代码、重复设施）
2. 是否存在逻辑 bug（并发/竞态、生命周期、几何计算、DPI、消息路由）
3. 功能重构后是否仍可用（渲染管线、拖拽、对齐参考线、IPC 契约）
4. 是否严格符合 Rust 开发规范与 trellis backend spec（unsafe、unwrap/expect、模块文档、单文件行数、clippy）

## 需求
- 审计范围：20926b54..2bc9dfcf 全部变更文件，重点：
  - `src/platform/windows/glass_engine.rs`（新增约 2081 行，liquid-verre-rs 集成）
  - `src/platform/windows/d2d_render.rs`、`grid_window.rs`、`wnd_proc.rs`、`drop_target.rs`
  - `src/platform/windows/alignment_guide/*`
  - `src/config/app_config.rs`、`src/tauri_commands/*`
- 运行质量门禁：cargo fmt / clippy --all-targets / test（如适用，前端 check）
- 发现问题直接修复，修复后功能必须保持正常（门禁全绿）
- 将本次审计发现的可泛化结论沉淀进 `.trellis/spec/`（backend 相关规范文件）

## 验收标准
- [x] 完成审计报告（问题清单：过度设计 / 逻辑bug / 重构回归 / 规范违规，含定位与严重级别）
- [x] 高危问题（逻辑 bug、功能不可用）全部修复
- [x] 中低危问题按成本收益处理，其余记录进任务 notes
- [x] cargo fmt / clippy / test 全绿（CI allowlist 不增加）
- [x] trellis spec 更新，纳入本次审计结论

## 技术笔记
- 单文件 <=1500 行（渲染引擎可放宽）；禁止平台 trait 预留、插件注册表
- 测试共享全局状态需串行锁
- liquid-verre-rs 0.5.3 为外部库（workspace 依赖），库 debug panic 类问题需以防御性调用规避


## 收尾状态（2026-11-04）
- 验收明细见同目录 audit-report.md。
- 未完成项：第四节「需实机验证」4 条依赖 Windows 桌面环境人工复验。
