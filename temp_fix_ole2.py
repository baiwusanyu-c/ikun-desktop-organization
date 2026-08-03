import os

path = r'D:\github-project\ikun_organizer\src\platform\windows\drag_handler.rs'
with open(path, 'r', encoding='utf-8') as f:
    content = f.read()

# Fix 1: Multi-item OLE drag - copy to temp with original names, keep overlay alive
old_multi = '''        let (file_paths, drag_items, icon_handle) = {
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
                    log::info!("[GRID-OLE-MULTI] item[{}]: logical=\'{}\', resolved=\'{}\'",
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
        };

        // \u9500\u6bc1\u62d6\u62fd\u6b8b\u5f71
        unsafe { super::drag_image::destroy_drag_overlay(); }
        log::info!("\u5206\u7c7b\u6846\u2192Explorer OLE (multi): {} items", file_paths.len());

        let box_drag_data = IkunBoxDragData {
            source_hwnd: hwnd as usize,
            source_config_id: config_id_str,
            items: drag_items,
        };

        // \u9690\u85cf\u5206\u7c7b\u6846\uff0c\u8ba9 OLE hit-testing \u80fd\u5230\u8fbe Explorer
        unsafe { ShowWindow(hwnd, SW_HIDE); }
        let drop_succeeded = super::ole_dnd::start_box_drag(file_paths, box_drag_data, icon_handle);
        unsafe { ShowWindow(hwnd, SW_SHOWNA); }

        if drop_succeeded {
            log::info!("\u5206\u7c7b\u6846\u2192Explorer OLE \u591a\u6587\u4ef6\u62d6\u62fd\u6210\u529f\uff0c\u56fe\u6807\u4fdd\u6301\u539f\u4f4d");
        } else {
            log::info!("\u5206\u7c7b\u6846\u2192Explorer OLE \u591a\u6587\u4ef6\u62d6\u62fd\u53d6\u6d88\uff0c\u56fe\u6807\u4fdd\u6301\u539f\u4f4d");
        }
        super::d2d_render::render_layered_window(hwnd);'''

new_multi = '''        // \u2605 \u521b\u5efa\u4e34\u65f6\u76ee\u5f55\uff0c\u590d\u5236\u4ed3\u5e93\u6587\u4ef6\u5e76\u4f7f\u7528\u539f\u59cb\u6587\u4ef6\u540d
        let temp_dir = std::env::temp_dir().join("ikun_ole_drag");
        let _ = std::fs::create_dir_all(&temp_dir);
        
        let (file_paths, drag_items, icon_handle) = {
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
                    // \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84\uff08\u4ed3\u5e93\u8def\u5f84\uff09
                    let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);
                    
                    // \u2605 \u63d0\u53d6\u539f\u59cb\u6587\u4ef6\u540d\uff08\u4ece\u903b\u8f91\u8def\u5f84\uff09\u5e76\u590d\u5236\u5230\u4e34\u65f6\u76ee\u5f55
                    let original_name = std::path::Path::new(&item.file_path)
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| format!("file_{}", idx));
                    let temp_path = temp_dir.join(&original_name);
                    
                    if resolved.exists() {
                        if let Err(e) = std::fs::copy(&resolved, &temp_path) {
                            log::warn!("[GRID-OLE-MULTI] \u590d\u5236\u6587\u4ef6\u5931\u8d25: {:?} -> {:?}: {}", resolved, temp_path, e);
                            continue;
                        }
                    } else {
                        log::warn!("[GRID-OLE-MULTI] \u6e90\u6587\u4ef6\u4e0d\u5b58\u5728: {:?}", resolved);
                        continue;
                    }
                    
                    let temp_str = temp_path.to_string_lossy().to_string();
                    log::info!("[GRID-OLE-MULTI] item[{}]: original=\'{}\', temp=\'{}\'",
                        idx, original_name, temp_str);
                    paths.push(temp_str.clone());
                    items.push(IkunDragItemData {
                        index: idx,
                        file_path: temp_str,
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
        };

        log::info!("\u5206\u7c7b\u6846\u2192Explorer OLE (multi): {} items", file_paths.len());

        let box_drag_data = IkunBoxDragData {
            source_hwnd: hwnd as usize,
            source_config_id: config_id_str,
            items: drag_items,
        };

        // \u2605 \u4e0d\u9500\u6bc1\u62d6\u62fd\u6b8b\u5f71\uff0c\u8ba9\u5b83\u5728 OLE \u62d6\u62fd\u671f\u95f4\u4fdd\u6301\u5b58\u6d3b
        // \u9690\u85cf\u5206\u7c7b\u6846\uff0c\u8ba9 OLE hit-testing \u80fd\u5230\u8fbe Explorer
        unsafe { ShowWindow(hwnd, SW_HIDE); }
        let drop_succeeded = super::ole_dnd::start_box_drag(file_paths, box_drag_data, icon_handle);
        unsafe { ShowWindow(hwnd, SW_SHOWNA); }
        
        // \u2605 OLE \u62d6\u62fd\u5b8c\u6210\u540e\u9500\u6bc1\u6b8b\u5f71
        unsafe { super::drag_image::destroy_drag_overlay(); }
        
        // \u2605 \u6e05\u7406\u4e34\u65f6\u76ee\u5f55
        let _ = std::fs::remove_dir_all(&temp_dir);
        log::info!("[GRID-OLE-MULTI] \u4e34\u65f6\u76ee\u5f55\u5df2\u6e05\u7406: {:?}", temp_dir);

        if drop_succeeded {
            log::info!("\u5206\u7c7b\u6846\u2192Explorer OLE \u591a\u6587\u4ef6\u62d6\u62fd\u6210\u529f\uff0c\u56fe\u6807\u4fdd\u6301\u539f\u4f4d");
        } else {
            log::info!("\u5206\u7c7b\u6846\u2192Explorer OLE \u591a\u6587\u4ef6\u62d6\u62fd\u53d6\u6d88\uff0c\u56fe\u6807\u4fdd\u6301\u539f\u4f4d");
        }
        super::d2d_render::render_layered_window(hwnd);'''

if old_multi in content:
    content = content.replace(old_multi, new_multi, 1)
    print('OK: Fixed multi-item OLE drag')
else:
    print('ERROR: Could not find multi-item OLE drag code')

# Fix 2: Single-item OLE drag - same fixes
old_single = '''              let item = &data.items[item_idx as usize];
              // \u2605 \u89e3\u6790\u5230\u5b9e\u9645\u5b58\u5728\u7684\u8def\u5f84
              let resolved = super::drag_drop::resolve_drag_item_read_path(&item.file_path);
              let resolved_str = resolved.to_string_lossy().to_string();
              (
                  item_idx,
                  resolved_str,
                  item.display_name.clone(),
                  item.icon_handle,
              )'''

new_single = '''              let item = &data.items[item_idx as usize];
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

if old_single in content:
    content = content.replace(old_single, new_single, 1)
    print('OK: Fixed single-item OLE drag')
else:
    print('ERROR: Could not find single-item OLE drag code')

with open(path, 'w', encoding='utf-8') as f:
    f.write(content)

print('All fixes applied')
