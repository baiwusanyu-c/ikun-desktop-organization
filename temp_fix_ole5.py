path = r'D:\github-project\ikun_organizer\src\platform\windows\drag_handler.rs'
with open(path, 'r', encoding='utf-8') as f:
    content = f.read()

# Use the exact text from the file
search = 'resolve_drag_item_read_path(&item.file_path);\n              let resolved_str = resolved.to_string_lossy().to_string();\n              (\n                  item_idx,\n                  resolved_str,'

if search in content:
    replacement = '''resolve_drag_item_read_path(&item.file_path);
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
              let resolved_str = temp_path.to_string_lossy().to_string();
              (
                  item_idx,
                  resolved_str,'''
    content = content.replace(search, replacement, 1)
    print('OK: Fixed single-item path')
else:
    print('ERROR: Could not find search text')

with open(path, 'w', encoding='utf-8') as f:
    f.write(content)
