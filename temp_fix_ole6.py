path = r'D:\github-project\ikun_organizer\src\platform\windows\drag_handler.rs'
with open(path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# Replace lines 507-509 (0-indexed: 506-508) with temp dir version
new_lines = [
    '            // \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84\u5e76\u590d\u5236\u5230\u4e34\u65f6\u76ee\u5f55\u4f7f\u7528\u539f\u59cb\u6587\u4ef6\u540d\n',
    '            let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);\n',
    '            let temp_dir = std::env::temp_dir().join("ikun_ole_drag");\n',
    '            let _ = std::fs::create_dir_all(&temp_dir);\n',
    '            let original_name = std::path::Path::new(&item.file_path)\n',
    '                .file_name()\n',
    '                .map(|s| s.to_string_lossy().to_string())\n',
    '                .unwrap_or_else(|| format!("file_{}", item_idx));\n',
    '            let temp_path = temp_dir.join(&original_name);\n',
    '            if resolved.exists() {\n',
    '                let _ = std::fs::copy(&resolved, &temp_path);\n',
    '            }\n',
    '            let resolved_str = temp_path.to_string_lossy().to_string();\n',
]

lines[506:509] = new_lines
print(f'OK: Replaced lines 507-509 with {len(new_lines)} new lines')

# Now find and fix the single-item destroy_drag_overlay call
# It should be around line 518-519 (now shifted by 10 lines)
for i in range(len(lines)):
    if '销毁拖拽残影' in lines[i] and i > 510:
        # Check next line
        if 'destroy_drag_overlay' in lines[i+1]:
            # Comment out the destroy call and add comment
            lines[i] = '        // \u2605 \u4e0d\u9500\u6bc1\u62d6\u62fd\u6b8b\u5f71\uff0c\u8ba9\u5b83\u5728 OLE \u62d6\u62fd\u671f\u95f4\u4fdd\u6301\u5b58\u6d3b\n'
            lines[i+1] = '        // unsafe { super::drag_image::destroy_drag_overlay(); }\n'
            print(f'OK: Commented out destroy_drag_overlay at line {i+2}')
            break

# Find the single-item OLE drag completion and add cleanup
for i in range(len(lines)):
    if '分类框→Explorer OLE 拖拽取消' in lines[i] and i > 520:
        # Insert cleanup code after this block
        insert_pos = i + 3  # After the else block
        cleanup = [
            '        \n',
            '        // \u2605 OLE \u62d6\u62fd\u5b8c\u6210\u540e\u9500\u6bc1\u6b8b\u5f71\u5e76\u6e05\u7406\u4e34\u65f6\u76ee\u5f55\n',
            '        unsafe { super::drag_image::destroy_drag_overlay(); }\n',
            '        let _ = std::fs::remove_dir_all(&temp_dir);\n',
        ]
        lines[insert_pos:insert_pos] = cleanup
        print(f'OK: Added cleanup code at line {insert_pos+1}')
        break

with open(path, 'w', encoding='utf-8') as f:
    f.writelines(lines)

print('Single-item fixes applied')
