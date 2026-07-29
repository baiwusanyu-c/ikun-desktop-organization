# 修复文件图标缩略图显示

## 目标
让程序绘制的文件图标与 Windows 原生资源管理器一致，对多媒体文件显示缩略图预览。

## 背景
当前 icon_extract.rs 使用 SHDefExtractIconW/SHGetFileInfoW 只返回文件类型图标，不返回缩略图。

## 技术方案
1. 新增 IShellItemImageFactory COM 接口
2. 新增 extract_file_thumbnail 函数
3. 修改 extract_file_icon 优先尝试缩略图
