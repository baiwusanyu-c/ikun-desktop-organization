path = r'D:\github-project\ikun_organizer\src\platform\windows\drag_handler.rs'
with open(path, 'r', encoding='utf-8') as f:
    content = f.read()

# Fix single-item OLE drag
old = '''              let item = &data.items[item_idx as usize];
              // \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84
              let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);
              let resolved_str = resolved.to_string_lossy().to_string();
              (
                  item_idx,
                  resolved_str,
                  item.display_name.clone(),
                  item.icon_handle,
              )'''

new = '''              let item = &data.items[item_idx as usize];
              // \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84\u5e76\u590d\u5236\u5230\u4e34\u65f6\u76ee\u5f55
              let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);
              let temp_dir = std::env::temp_dir().join("ikun_ole_drag");
              let _ = std::fs::create_dir_all(&temp_dir);
              let original_name = std::path::Path::new(&item.file_path)
                  .file_name()
                  .map(|s| s.to_string_lossy().to_string())
                  .unwrap_or_else(|| format!("file_{}", item_idx));
              let temp_path = temp_dir.join(&original_name);
              if resolved.exists() {
                  let _ = std::fs::copy(&resolved, &temp_path);
              }
              let temp_str = temp_path.to_string_lossy().to_string();
              (
                  item_idx,
                  temp_str,
                  item.display_name.clone(),
                  item.icon_handle,
              )'''

if old in content:
    content = content.replace(old, new, 1)
    print('OK: Fixed single-item OLE drag')
else:
    print('ERROR: Could not find single-item OLE drag code')

with open(path, 'w', encoding='utf-8') as f:
    f.write(content)
