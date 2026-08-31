# 修复开机自启间歇性失效（续）

## 目标
上一轮修复（08-28，commit 5b4ba3b：Run 值加引号 + StartupApproved 启用标记 + 错误日志 + 测试防污染）后，另一台电脑 cargo run 场景下开机自启仍间歇性失效。本轮找到并修复残留的自毁路径，并让诊断日志真正可见。

## 背景与根因分析（本轮新发现）

### 根因 1：配置加载失败 → 启动同步**删除** Run 键（自毁路径）
- `AppContext::new()` 中 `AppConfig::load()` 失败时静默回退 `AppConfig::default()`（auto_start=false）。
- 随后 `TauriRuntime::initialize()` 第 7 步按该默认值执行 `set_auto_start(false)`，把注册表 Run 值与 StartupApproved 标记删掉。
- 触发条件：配置文件损坏/读取失败。而 `config.save()` 用 `std::fs::write` 直接覆盖（非原子），进程在写入中途被杀（强制关机/断电）即可制造损坏文件 → 下次开机自启被自己删除。这完美解释"时灵时不灵、重新勾选又好了"。

### 根因 2：诊断日志不可见
- `init_logger()` 用 `env_logger::Builder::from_default_env()`，未设 RUST_LOG 时默认级别为 **error**。
- 上轮加的开机上下文、同步失败告警（log::warn/info）在另一台电脑上**全部不会写入日志文件**，导致上轮修复"无法被验证也无法定位"。

### 根因 3：崩溃无痕迹
- `#![windows_subsystem = "windows"]` 下若启动早期 panic，进程静默退出，日志无任何记录，无法区分"Windows 没拉起"与"拉起后崩溃"。

### 次要：同步时机太晚
- 自启同步是 initialize() 第 7 步，排在托盘初始化、布局恢复（含最长 5s 重试）之后；前面任何一步异常都会跳过同步，当次开机无法重写被系统标记为禁用的 StartupApproved。

## 需求
- R1 配置加载失败时跳过自启同步（绝不按回退默认值删除注册表条目），并记 error 日志。AppContext 暴露 `config_loaded` 标志。
- R2 `AppConfig::save()` 改为原子写入（临时文件 + rename），杜绝半截配置文件。
- R3 日志默认级别改为 info（保留 RUST_LOG 覆盖），让开机诊断在用户机器上真正可见。
- R4 自启同步提前到 initialize() 第一步；失败重试最多 3 次。
- R5 启动时先输出自启状态诊断（Run 值原文、是否指向当前 exe、StartupApproved 首字节），再执行同步。
- R6 main.rs 安装 panic hook，panic 时把信息与回溯写入日志文件。

## 验收标准
- [ ] 配置加载失败场景下不再调用 set_auto_start(false)（代码路径审查 + 单测）。
- [ ] save() 原子写入：写入后无 .tmp 残留，内容可被 load() 读回。
- [ ] 默认日志级别为 info（未设 RUST_LOG 时 warn/info 可见）。
- [ ] cargo fmt / clippy / cargo test 全部通过。

## 技术笔记
- env_logger 0.11：`Builder::from_env(Env::default().default_filter_or("info"))`。
- std::fs::rename 在 Windows 上原子替换已存在文件。
- StartupApproved 首字节：02=启用，03=禁用。
