# 修复程序退出时部分图标不恢复桌面

## 目标
修复程序退出时，仓库(vault)中的图标没有被还原到用户桌面的 bug。

## 需求
- 重写 restore_all_hidden_icons() 函数：遍历仓库目录中的所有文件
- 根据仓库文件名前缀(user__/public__) 还原原始桌面路径
- 将文件从仓库移回桌面
- 调用 notify_shell_path() 通知 Shell 刷新

## 技术笔记
- 仓库文件命名规则：user__<filename> / public__<filename>
- 还原路径函数：logical_path_from_vault_name()
- 导出路径函数：export_desktop_path()
