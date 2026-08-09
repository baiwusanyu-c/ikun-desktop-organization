# Bug 修复：控制面板 state not managed 错误

## 问题描述

打开控制面板时报错：
```
无法加载配置：state not managed for field 'runtime' on command get_config.
You must call .manage() before using this command
```

## 根因分析

原代码在 .setup() 回调中调用 pp.manage(runtime.clone()) 注册状态。
虽然 setup 在窗口创建之前运行，但 Tauri 的 Builder::manage() 提供更早的
注册时机——在 Builder 构建阶段就完成状态注册，确保状态在任何 WebView
窗口创建时就已经可用。

## 修复方案

将 SharedTauriRuntime 的状态注册从 setup() 回调移到 Builder::manage()：

1. 在 Builder 之前创建 
ew_shared_runtime()
2. 使用 Builder::manage(runtime.clone()) 注册状态
3. setup() 只负责启动平台轮询线程

## 变更文件

- `src/main.rs`: 调整状态注册时机

## 验证

- [x] cargo build 构建成功
- [x] cargo test 全部 510 个测试通过
