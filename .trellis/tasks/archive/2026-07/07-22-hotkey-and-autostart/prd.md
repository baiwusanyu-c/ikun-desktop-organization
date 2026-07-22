# 全局热键呼出控制面板 + Windows 开机自启

## 目标
为 ikun_organizer 新增两个系统级功能：
1. 全局键盘快捷键呼出/隐藏控制面板窗口
2. Windows 开机自启动支持

## 需求
- **全局热键**：按 Ctrl+Alt+D（默认）可全局切换控制面板窗口的显示/隐藏
- **热键可配置**：通过配置文件的 hotkeys.toggle_panel 字段自定义
- **开机自启**：通过注册表 HKCU\Software\Microsoft\Windows\CurrentVersion\Run 实现
- **开机自启可配置**：通过 general.auto_start 字段开关，启动时自动同步状态到注册表

## 验收标准
- [ ] 在任意窗口按 Ctrl+Alt+D 可呼出控制面板
- [ ] 再次按 Ctrl+Alt+D 可隐藏控制面板
- [ ] 开机自启开启后，重启电脑自动启动程序
- [ ] 开机自启关闭后，注册表项被清除
- [ ] 编译通过，无 clippy 严重警告

## 技术笔记
- 使用 Win32 RegisterHotKey API 注册全局热键
- 热键消息通过隐藏消息窗口接收 WM_HOTKEY
- 开机自启使用现有 RegCreateKeyExW + RegSetValueExW 写入注册表
- 删除自启使用 RegDeleteValueW
- 集成到现有 tick() 轮询循环中消费热键事件
