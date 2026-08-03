path = r'D:\github-project\ikun_organizer\src\platform\windows\drag_handler.rs'
with open(path, 'r', encoding='utf-8') as f:
    content = f.read()

old = '''        let (file_paths, drag_items, icon_handle) = {
            let data = match unsafe { get_data(hwnd) } {
                Some(d) => d,
                None => {
                    unsafe { super::drag_image::destroy_drag_overlay(); }
                    super::d2d_render::render_layered_window(hwnd);
                    return;
                }
            };

            let mut paths = Vec::new();
            let mut items = Vec::new();
            let mut primary_icon = std::ptr::null_mut();
            for &idx in &multi_drag_items {
                if idx < data.items.len() {
                    let item = &data.items[idx];
                    paths.push(item.file_path.clone());
                    items.push(IkunDragItemData {
                        index: idx,
                        file_path: item.file_path.clone(),
                        display_name: item.display_name.clone(),
                    });
                    if idx == item_idx as usize {
                        primary_icon = item.icon_handle;
                    }
                }
            }
            if primary_icon.is_null() && item_idx >= 0 && (item_idx as usize) < data.items.len() {
                primary_icon = data.items[item_idx as usize].icon_handle;
            }
            (paths, items, primary_icon)
        };'''

new = '''        let (file_paths, drag_items, icon_handle) = {
            let data = match unsafe { get_data(hwnd) } {
                Some(d) => d,
                None => {
                    unsafe { super::drag_image::destroy_drag_overlay(); }
                    super::d2d_render::render_layered_window(hwnd);
                    return;
                }
            };

            let mut paths = Vec::new();
            let mut items = Vec::new();
            let mut primary_icon = std::ptr::null_mut();
            for &idx in &multi_drag_items {
                if idx < data.items.len() {
                    let item = &data.items[idx];
                    // \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84\uff08\u4ed3\u5e93\u8def\u5f84\uff09\uff0c\u5426\u5219 Explorer \u627e\u4e0d\u5230\u6587\u4ef6
                    let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);
                    let resolved_str = resolved.to_string_lossy().to_string();
                    log::info!("[GRID-OLE-MULTI] item[{}]: logical='{}', resolved='{}'",
                        idx, item.file_path, resolved_str);
                    paths.push(resolved_str.clone());
                    items.push(IkunDragItemData {
                        index: idx,
                        file_path: resolved_str,
                        display_name: item.display_name.clone(),
                    });
                    if idx == item_idx as usize {
                        primary_icon = item.icon_handle;
                    }
                }
            }
            if primary_icon.is_null() && item_idx >= 0 && (item_idx as usize) < data.items.len() {
                primary_icon = data.items[item_idx as usize].icon_handle;
            }
            (paths, items, primary_icon)
        };'''

if old in content:
    content = content.replace(old, new, 1)
    print('OK: Multi-item OLE drag resolves vault paths')
else:
    print('ERROR: Could not find multi-item OLE drag text')

# Also fix single-item path
old2 = '''            let item = &data.items[item_idx as usize];
            (
                item_idx,
                item.file_path.clone(),
                item.display_name.clone(),
                item.icon_handle,
            )
        };

        // \u9500\u6bc1\u62d6\u62fd\u6b8b\u5f71
        unsafe { super::drag_image::destroy_drag_overlay(); }

        log::info!("\u5206\u7c7b\u6846\u2192Explorer OLE: {} (path={})", display_name, file_path);'''

new2 = '''            let item = &data.items[item_idx as usize];
            // \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84
            let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);
            let resolved_str = resolved.to_string_lossy().to_string();
            (
                item_idx,
                resolved_str,
                item.display_name.clone(),
                item.icon_handle,
            )
        };

        // \u9500\u6bc1\u62d6\u62fd\u6b8b\u5f71
        unsafe { super::drag_image::destroy_drag_overlay(); }

        log::info!("\u5206\u7c7b\u6846\u2192Explorer OLE: {} (path={})", display_name, file_path);'''

if old2 in content:
    content = content.replace(old2, new2, 1)
    print('OK: Single-item OLE drag resolves vault path')
else:
    print('ERROR: Could not find single-item OLE drag text')

with open(path, 'w', encoding='utf-8') as f:
    f.write(content)
