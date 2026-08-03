path = r'D:\github-project\ikun_organizer\src\platform\windows\drag_handler.rs'
with open(path, 'r', encoding='utf-8') as f:
    content = f.read()

# Fix: Move temp_dir outside the inner block for single-item
old = '''    } else {
        // Single-item OLE drag (existing logic)
        let (item_idx, file_path, display_name, icon_handle) = {'''

new = '''    } else {
        // Single-item OLE drag (existing logic)
        let temp_dir = std::env::temp_dir().join("ikun_ole_drag");
        let _ = std::fs::create_dir_all(&temp_dir);
        let (item_idx, file_path, display_name, icon_handle) = {'''

if old in content:
    content = content.replace(old, new, 1)
    print('OK: Moved temp_dir outside inner block')
else:
    print('ERROR: Could not find single-item block start')

# Remove duplicate temp_dir inside the inner block
old2 = '''            // \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84\u5e76\u590d\u5236\u5230\u4e34\u65f6\u76ee\u5f55\u4f7f\u7528\u539f\u59cb\u6587\u4ef6\u540d
            let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);
            let temp_dir = std::env::temp_dir().join("ikun_ole_drag");
            let _ = std::fs::create_dir_all(&temp_dir);
            let original_name'''

new2 = '''            // \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84\u5e76\u590d\u5236\u5230\u4e34\u65f6\u76ee\u5f55\u4f7f\u7528\u539f\u59cb\u6587\u4ef6\u540d
            let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);
            let original_name'''

if old2 in content:
    content = content.replace(old2, new2, 1)
    print('OK: Removed duplicate temp_dir inside inner block')
else:
    print('ERROR: Could not find duplicate temp_dir')

with open(path, 'w', encoding='utf-8') as f:
    f.write(content)
