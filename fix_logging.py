import re

with open('src/app.rs', 'r', encoding='utf-8') as f:
    content = f.read()

# Find the display_name resolution block and add more detailed logging
old_block = '''                log::info!(
                    "恢复图标: file_path={}, display_names[{}]={:?}",
                    file_path, idx,
                    box_data.display_names.get(idx).map(|s| s.as_str()).unwrap_or("<missing>")
                );
                // 根据路径类型决定显示名称：
                // - CLSID/shell::virtual:: 路径始终从路径本身解析（避免 display_names 数组不同步）
                // - 普通文件路径优先使用持久化的 display_name，回退到 file_stem
                let display_name = if file_path.starts_with("::{") {
                    // CLSID 路径（已知虚拟 Shell 对象）使用预定义的显示名称
                    crate::platform::clsid_display_name(file_path)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            log::warn!("CLSID 路径未在已知列表中: {}", file_path);
                            file_path.clone()
                        })'''

new_block = '''                log::info!(
                    "恢复图标: file_path={}, display_names[{}]={:?}",
                    file_path, idx,
                    box_data.display_names.get(idx).map(|s| s.as_str()).unwrap_or("<missing>")
                );
                // 根据路径类型决定显示名称：
                // - CLSID/shell::virtual:: 路径始终从路径本身解析（避免 display_names 数组不同步）
                // - 普通文件路径优先使用持久化的 display_name，回退到 file_stem
                let display_name = if file_path.starts_with("::{") {
                    log::info!("[restore] 检测到 CLSID 路径: {}", file_path);
                    // CLSID 路径（已知虚拟 Shell 对象）使用预定义的显示名称
                    let clsid_result = crate::platform::clsid_display_name(file_path);
                    log::info!("[restore] clsid_display_name 返回: {:?}", clsid_result);
                    clsid_result
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            log::warn!("[restore] CLSID 路径未在已知列表中: {}", file_path);
                            file_path.clone()
                        })'''

content = content.replace(old_block, new_block)

with open('src/app.rs', 'w', encoding='utf-8') as f:
    f.write(content)

print("Added detailed CLSID logging")
