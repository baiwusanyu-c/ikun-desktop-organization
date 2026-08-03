path = r'D:\github-project\ikun_organizer\src\platform\windows\drag_handler.rs'
with open(path, 'r', encoding='utf-8') as f:
    content = f.read()

# Fix single-item: use tab indentation to match source
old = '\t            let item = &data.items[item_idx as usize];\n              // \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84\n              let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);\n              let resolved_str = resolved.to_string_lossy().to_string();\n              (\n                  item_idx,\n                  resolved_str,\n                  item.display_name.clone(),\n                  item.icon_handle,\n              )'

# Let me try a simpler approach - find by a unique substring
search = '// \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84\n              let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);\n              let resolved_str = resolved.to_string_lossy().to_string();'

if search in content:
    replacement = '''// \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84\u5e76\u590d\u5236\u5230\u4e34\u65f6\u76ee\u5f55\u4f7f\u7528\u539f\u59cb\u6587\u4ef6\u540d
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
              let resolved_str = temp_path.to_string_lossy().to_string();'''
    content = content.replace(search, replacement, 1)
    print('OK: Fixed single-item path')
else:
    print('ERROR: Could not find search text')
    # Debug
    idx = content.find('\u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728')
    if idx >= 0:
        snippet = content[max(0,idx-20):idx+100]
        print(f'Found at position {idx}: {repr(snippet)}')

with open(path, 'w', encoding='utf-8') as f:
    f.write(content)
