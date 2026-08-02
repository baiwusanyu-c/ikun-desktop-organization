//! OLE 拖拽目标实现（IDropTarget）

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! 为分类框提供接收拖拽的能力：
//! - 接收从其他分类框拖入的图标（框间移动）
//! - 接收从桌面拖入的快捷方式（桌面→框）
//!
//! 从 `ole_dnd.rs` 拆分以控制文件行数。

use std::ffi::c_void;
use std::sync::Mutex;

use super::ffi_imports::*;
use super::ffi_types::*;
use super::ole_dnd::{get_box_drag_format_id, IDataObjectVtbl};

// =============================================================================
// 拖拽放置结果（全局暂存，供 WndProc 读取）
// =============================================================================

/// 待处理的拖拽放置结果
pub struct PendingDrop {
    /// 文件路径列表
    pub file_paths: Vec<String>,
    /// 框间拖拽数据（有值 = 框→框移动，无值 = 桌面→框拖入）
    pub box_data: Option<IkunBoxDragData>,
    /// 放置目标的屏幕坐标
    pub drop_x: i32,
    pub drop_y: i32,
    /// 拖拽悬停时的插入模式状态（来自 drag_over 可视化反馈）
    pub insert_col: i32,
    pub insert_row: i32,
}

static PENDING_DROP: Mutex<Option<PendingDrop>> = Mutex::new(None);

/// 取出待处理的拖拽放置结果（由 WndProc 调用）
pub fn take_pending_drop() -> Option<PendingDrop> {
    PENDING_DROP.lock().ok().and_then(|mut g| g.take())
}

/// 待处理的源框信息（用于 WM_USER_DRAG_ITEM_REMOVED）
pub struct PendingSourceRemoval {
    pub file_path: String,
    pub display_name: String,
    pub item_index: usize,
}

static PENDING_SOURCE_REMOVAL: Mutex<Option<PendingSourceRemoval>> = Mutex::new(None);

/// 取出待处理的源框移除信息（由 WndProc 调用）
pub fn take_pending_source_removal() -> Option<PendingSourceRemoval> {
    PENDING_SOURCE_REMOVAL
        .lock()
        .ok()
        .and_then(|mut g| g.take())
}

/// 框→框移动：暂存待添加项信息（含释放坐标）
static PENDING_ADD_ITEM: Mutex<Option<(String, String, i32, i32, i32, i32)>> = Mutex::new(None);

/// 设置待添加项（包含释放位置的屏幕坐标）
pub fn set_pending_add_item(
    file_path: String,
    display_name: String,
    drop_x: i32,
    drop_y: i32,
    insert_col: i32,
    insert_row: i32,
) {
    if let Ok(mut g) = PENDING_ADD_ITEM.lock() {
        *g = Some((
            file_path,
            display_name,
            drop_x,
            drop_y,
            insert_col,
            insert_row,
        ));
    }
}

/// 取出待添加项（包含释放坐标）
pub fn take_pending_add_item() -> Option<(String, String, i32, i32, i32, i32)> {
    PENDING_ADD_ITEM.lock().ok().and_then(|mut g| g.take())
}

// =============================================================================
// IDropTarget vtable 和结构体
// =============================================================================

#[repr(C)]
#[allow(non_snake_case)]
struct IDropTargetVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    drag_enter: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, i64, *mut u32) -> HRESULT,
    drag_over: unsafe extern "system" fn(*mut c_void, u32, i64, *mut u32) -> HRESULT,
    drag_leave: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, i64, *mut u32) -> HRESULT,
}

#[repr(C)]
struct BoxDropTarget {
    vtbl: *const IDropTargetVtbl,
    ref_count: u32,
    target_hwnd: HWND,
    custom_format_id: u32,
}

/// 当前是否有来自分类框的拖拽悬停（用于区分外部拖拽，控制可视化反馈）
use std::sync::atomic::{AtomicBool, Ordering};
static BOX_DRAG_HOVER: AtomicBool = AtomicBool::new(false);

/// 检查当前悬停的拖拽是否来自分类框
pub fn is_box_drag_hover() -> bool {
    BOX_DRAG_HOVER.load(Ordering::Relaxed)
}

static DROP_TARGET_VTBL: IDropTargetVtbl = IDropTargetVtbl {
    query_interface: drop_target_query_interface,
    add_ref: drop_target_add_ref,
    release: drop_target_release,
    drag_enter: drop_target_drag_enter,
    drag_over: drop_target_drag_over,
    drag_leave: drop_target_drag_leave,
    drop: drop_target_drop,
};

// ----- IUnknown -----

unsafe extern "system" fn drop_target_query_interface(
    this: *mut c_void,
    riid: *const u8,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    let iid = std::slice::from_raw_parts(riid, 16);
    if iid == IID_IUNKNOWN || iid == IID_IDROPTARGET {
        *ppv = this;
        drop_target_add_ref(this);
        S_OK
    } else {
        *ppv = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn drop_target_add_ref(this: *mut c_void) -> u32 {
    let obj = &mut *(this as *mut BoxDropTarget);
    obj.ref_count += 1;
    obj.ref_count
}

unsafe extern "system" fn drop_target_release(this: *mut c_void) -> u32 {
    let obj = &mut *(this as *mut BoxDropTarget);
    obj.ref_count -= 1;
    if obj.ref_count == 0 {
        drop(Box::from_raw(obj));
        return 0;
    }
    obj.ref_count
}

// ----- IDropTarget 方法 -----

unsafe extern "system" fn drop_target_drag_enter(
    this: *mut c_void,
    data_obj: *mut c_void,
    _ks: u32,
    _pt: i64,
    effect: *mut u32,
) -> HRESULT {
    let target = &*(this as *const BoxDropTarget);
    let has_box = query_format(data_obj, target.custom_format_id);
    let has_hdrop = query_format(data_obj, CF_HDROP);
    // 检查是否有虚拟 Shell 对象（回收站、此电脑等）
    let has_shell_idlist = if !has_hdrop && !has_box {
        let fmt_name: Vec<u16> = "Shell IDList Array\0".encode_utf16().collect();
        let cf = super::ffi_imports::RegisterClipboardFormatW(fmt_name.as_ptr());
        cf != 0 && query_format(data_obj, cf)
    } else {
        false
    };
    if has_box || has_hdrop || has_shell_idlist {
        *effect = DROPEFFECT_MOVE;
        BOX_DRAG_HOVER.store(has_box, Ordering::Relaxed);
        PostMessageW(target.target_hwnd, WM_USER_DRAG_ENTER, 0, 0);
        log::info!("[DRAG-ENTER] 拖拽进入: box={} hdrop={} shell_idlist={}", has_box, has_hdrop, has_shell_idlist);
    } else {
        *effect = DROPEFFECT_NONE;
        log::debug!("[DRAG-ENTER] 拒绝拖拽: 无支持的格式");
    }
    S_OK
}

unsafe extern "system" fn drop_target_drag_over(
    this: *mut c_void,
    _ks: u32,
    _pt: i64,
    effect: *mut u32,
) -> HRESULT {
    if *effect & DROPEFFECT_MOVE != 0 {
        *effect = DROPEFFECT_MOVE;
    }

    // 外部拖拽（桌面→框）：跳过所有视觉更新，保留 Windows 原生拖拽残影
    let is_box_hover = BOX_DRAG_HOVER.load(Ordering::Relaxed);
    if !is_box_hover {
        return S_OK;
    }

    // 框间拖拽：显示插入指示器
    let target = &*(this as *const BoxDropTarget);
    let should_update = super::grid_window::get_data(target.target_hwnd)
        .map(|d| !d.drag_active)
        .unwrap_or(false);
    if should_update {
        let mut cursor_pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut cursor_pt) };
        super::drag_visual::handle_cross_box_drag_over(
            target.target_hwnd,
            cursor_pt.x,
            cursor_pt.y,
        );
    }

    S_OK
}

unsafe extern "system" fn drop_target_drag_leave(this: *mut c_void) -> HRESULT {
    BOX_DRAG_HOVER.store(false, Ordering::Relaxed);
    let target = &*(this as *const BoxDropTarget);
    PostMessageW(target.target_hwnd, WM_USER_DRAG_LEAVE, 0, 0);
    S_OK
}

unsafe extern "system" fn drop_target_drop(
    this: *mut c_void,
    data_obj: *mut c_void,
    _ks: u32,
    pt: i64,
    effect: *mut u32,
) -> HRESULT {
    let target = &*(this as *const BoxDropTarget);
    PostMessageW(target.target_hwnd, WM_USER_DRAG_LEAVE, 0, 0);

    let drop_x = (pt & 0xFFFFFFFF) as i32;
    let drop_y = (pt >> 32) as i32;

    let box_data = if target.custom_format_id != 0 {
        read_box_drag_data(data_obj, target.custom_format_id)
    } else {
        None
    };

    let mut file_paths = read_drop_files(data_obj);
    if file_paths.is_empty() {
        // CF_HDROP 为空，尝试从 CFSTR_SHELLIDLIST 读取虚拟 Shell 对象
        log::info!("[DROP] CF_HDROP 为空，尝试读取 PIDL...");
        file_paths = read_shell_idlist_paths(data_obj);
    }
    if file_paths.is_empty() {
        *effect = DROPEFFECT_NONE;
        log::warn!("[DROP] 未读取到文件路径（CF_HDROP 和 PIDL 均为空）");
        return S_OK;
    }
    log::info!("[DROP] 最终 file_paths={:?}", file_paths);

    if let Ok(mut g) = PENDING_DROP.lock() {
        *g = Some(PendingDrop {
            file_paths: file_paths.clone(),
            box_data: box_data.clone(),
            drop_x,
            drop_y,
            insert_col: super::grid_window::get_data(target.target_hwnd)
                .map(|d| d.drag_over_insert_col)
                .unwrap_or(-1),
            insert_row: super::grid_window::get_data(target.target_hwnd)
                .map(|d| d.drag_over_insert_row)
                .unwrap_or(-1),
        });
    }

    // 框→框：通知源框移除
    if let Some(ref bd) = box_data {
        if let Some(item) = bd.items.first() {
            if let Ok(mut g) = PENDING_SOURCE_REMOVAL.lock() {
                *g = Some(PendingSourceRemoval {
                    file_path: item.file_path.clone(),
                    display_name: item.display_name.clone(),
                    item_index: item.index,
                });
            }
            let src = bd.source_hwnd as *mut c_void;
            PostMessageW(src, WM_USER_DRAG_ITEM_REMOVED, item.index, 0);
        }
    }

    PostMessageW(target.target_hwnd, WM_USER_DRAG_DROP, 0, 0);
    *effect = DROPEFFECT_MOVE;
    log::info!("拖拽放置完成（数据已暂存）");
    S_OK
}

// =============================================================================
// 辅助函数
// =============================================================================

unsafe fn query_format(data_obj: *mut c_void, cf: u32) -> bool {
    if data_obj.is_null() || cf == 0 {
        return false;
    }
    let vtbl = *(data_obj as *const *const IDataObjectVtbl);
    let fe = FORMATETC {
        cf_format: cf,
        ptd: std::ptr::null_mut(),
        dw_aspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    };
    ((*vtbl).query_get_data)(data_obj, &fe) == S_OK
}

unsafe fn read_drop_files(data_obj: *mut c_void) -> Vec<String> {
    if data_obj.is_null() {
        return Vec::new();
    }
    let vtbl = *(data_obj as *const *const IDataObjectVtbl);
    let fe = FORMATETC {
        cf_format: CF_HDROP,
        ptd: std::ptr::null_mut(),
        dw_aspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    };
    let mut medium = STGMEDIUM {
        tymed: 0,
        h_global: std::ptr::null_mut(),
        unk_for_release: std::ptr::null_mut(),
    };
    if ((*vtbl).get_data)(data_obj, &fe, &mut medium) != S_OK || medium.h_global.is_null() {
        return Vec::new();
    }
    let mut paths = Vec::new();
    let ptr = GlobalLock(medium.h_global);
    if !ptr.is_null() {
        let df = &*(ptr as *const DROPFILES);
        if df.f_wide != 0 {
            let text = (ptr as *const u8).add(df.p_files as usize) as *const u16;
            let mut off = 0;
            loop {
                let mut len = 0;
                while *text.add(off + len) != 0 {
                    len += 1;
                }
                if len == 0 {
                    break;
                }
                if let Ok(s) = String::from_utf16(std::slice::from_raw_parts(text.add(off), len)) {
                    if !s.is_empty() {
                        paths.push(s);
                    }
                }
                off += len + 1;
            }
        }
        GlobalUnlock(medium.h_global);
    }
    if medium.unk_for_release.is_null() && !medium.h_global.is_null() {
        GlobalFree(medium.h_global);
    }
    paths
}


/// 从 IDataObject 读取 Shell IDList 格式的 PIDL，提取虚拟 Shell 对象的 CLSID 路径。
///
/// 当从桌面拖拽回收站、此电脑等虚拟对象时，CF_HDROP 为空，
/// 需要从 CFSTR_SHELLIDLIST 格式读取 PIDL，然后通过 SHGetFileInfoW + SHGFI_PIDL
/// 获取显示名称，再映射到已知的 CLSID 路径。
unsafe fn read_shell_idlist_paths(data_obj: *mut c_void) -> Vec<String> {
    if data_obj.is_null() {
        return Vec::new();
    }

    // 注册 CFSTR_SHELLIDLIST 格式
    let fmt_name: Vec<u16> = "Shell IDList Array\0".encode_utf16().collect();
    let cf_shellidlist = super::ffi_imports::RegisterClipboardFormatW(fmt_name.as_ptr());
    if cf_shellidlist == 0 {
        log::warn!("[PIDL] RegisterClipboardFormat 失败");
        return Vec::new();
    }

    let vtbl = *(data_obj as *const *const IDataObjectVtbl);
    let fe = FORMATETC {
        cf_format: cf_shellidlist,
        ptd: std::ptr::null_mut(),
        dw_aspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    };
    let mut medium = STGMEDIUM {
        tymed: 0,
        h_global: std::ptr::null_mut(),
        unk_for_release: std::ptr::null_mut(),
    };
    if ((*vtbl).get_data)(data_obj, &fe, &mut medium) != S_OK || medium.h_global.is_null() {
        log::debug!("[PIDL] 无 CFSTR_SHELLIDLIST 数据");
        return Vec::new();
    }

    let mut paths = Vec::new();
    let ptr = GlobalLock(medium.h_global);
    if !ptr.is_null() {
        // CIDA 结构: cidl(u32) + (cidl+1) 个 u32 偏移量
        let cidl = *(ptr as *const u32);
        let offsets = std::slice::from_raw_parts(
            (ptr as *const u8).add(4) as *const u32,
            (cidl + 1) as usize,
        );

        log::info!("[PIDL] CIDA cidl={}, offsets={:?}", cidl, &offsets[..std::cmp::min(offsets.len(), 5)]);

        // SHGetFileInfoW + SHGFI_PIDL 声明
        #[repr(C)]
        struct ShellFileInfo {
            h_icon: *mut std::ffi::c_void,
            i_icon: i32,
            dw_attributes: u32,
            sz_display_name: [u16; 260],
            sz_type_name: [u16; 80],
        }
        const SHGFI_PIDL: u32 = 0x00000008;
        const SHGFI_DISPLAYNAME: u32 = 0x00000200;

        extern "system" {
            fn SHGetFileInfoW(
                pszPath: *const u8,
                dwFileAttributes: u32,
                psfi: *mut ShellFileInfo,
                cbFileInfo: u32,
                uFlags: u32,
            ) -> usize;
        }

        // 已知的虚拟 Shell 对象显示名称 → CLSID 映射
        let known_objects: &[(&str, &str)] = &[
            ("回收站", "::{645FF040-5081-101B-9F08-00AA002F954E}"),
            ("此电脑", "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"),
            ("网络", "::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}"),
            ("控制面板", "::{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}"),
        ];

        // 遍历每个 PIDL（跳过第一个，它是父文件夹 PIDL）
        for i in 1..=cidl as usize {
            if i >= offsets.len() {
                break;
            }
            let pidl = (ptr as *const u8).add(offsets[i] as usize);

            // 用 SHGetFileInfoW + SHGFI_PIDL 获取显示名称
            let mut info = ShellFileInfo {
                h_icon: std::ptr::null_mut(),
                i_icon: 0,
                dw_attributes: 0,
                sz_display_name: [0u16; 260],
                sz_type_name: [0u16; 80],
            };
            let result = SHGetFileInfoW(
                pidl,
                0,
                &mut info,
                std::mem::size_of::<ShellFileInfo>() as u32,
                SHGFI_PIDL | SHGFI_DISPLAYNAME,
            );

            if result != 0 {
                let name_len = info.sz_display_name.iter().position(|&c| c == 0).unwrap_or(0);
                let display_name = String::from_utf16_lossy(&info.sz_display_name[..name_len]);
                log::info!("[PIDL] 解析到显示名称: '{}'", display_name);

                // 查找已知虚拟对象
                let mut found = false;
                for (known_name, clsid) in known_objects {
                    if display_name == *known_name {
                        paths.push(clsid.to_string());
                        log::info!("[PIDL] 映射到 CLSID: {} → {}", display_name, clsid);
                        found = true;
                        break;
                    }
                }
                if !found {
                    log::warn!("[PIDL] 未找到 '{}' 对应的 CLSID 映射", display_name);
                }
            } else {
                log::warn!("[PIDL] SHGetFileInfoW 失败, offset={}", offsets[i]);
            }
        }
        GlobalUnlock(medium.h_global);
    }
    if medium.unk_for_release.is_null() && !medium.h_global.is_null() {
        GlobalFree(medium.h_global);
    }
    log::info!("[PIDL] 最终解析到 {} 个虚拟 Shell 对象: {:?}", paths.len(), paths);
    paths
}
unsafe fn read_box_drag_data(data_obj: *mut c_void, fmt_id: u32) -> Option<IkunBoxDragData> {
    if data_obj.is_null() || fmt_id == 0 {
        return None;
    }
    let vtbl = *(data_obj as *const *const IDataObjectVtbl);
    let fe = FORMATETC {
        cf_format: fmt_id,
        ptd: std::ptr::null_mut(),
        dw_aspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    };
    let mut medium = STGMEDIUM {
        tymed: 0,
        h_global: std::ptr::null_mut(),
        unk_for_release: std::ptr::null_mut(),
    };
    if ((*vtbl).get_data)(data_obj, &fe, &mut medium) != S_OK || medium.h_global.is_null() {
        return None;
    }
    let result = {
        let ptr = GlobalLock(medium.h_global);
        if ptr.is_null() {
            None
        } else {
            let bytes = ptr as *const u8;
            let mut len = 0;
            while *bytes.add(len) != 0 && len < 65536 {
                len += 1;
            }
            let r =
                serde_json::from_slice::<IkunBoxDragData>(std::slice::from_raw_parts(bytes, len))
                    .ok();
            GlobalUnlock(medium.h_global);
            r
        }
    };
    if medium.unk_for_release.is_null() && !medium.h_global.is_null() {
        GlobalFree(medium.h_global);
    }
    result
}

// =============================================================================
// 公共 API
// =============================================================================

/// 为分类框窗口注册 OLE 拖拽目标
pub fn register_box_drop_target(hwnd: HWND) -> *mut c_void {
    let custom_format_id = get_box_drag_format_id();
    let target = Box::new(BoxDropTarget {
        vtbl: &DROP_TARGET_VTBL,
        ref_count: 1,
        target_hwnd: hwnd,
        custom_format_id,
    });
    let ptr = Box::into_raw(target) as *mut c_void;
    let hr = unsafe { RegisterDragDrop(hwnd, ptr) };
    if hr != S_OK {
        log::warn!("RegisterDragDrop 失败: hr={:#010X}", hr as u32);
        unsafe { drop_target_release(ptr) };
        return std::ptr::null_mut();
    }
    log::info!("已注册拖拽目标: hwnd={:?}", hwnd);
    ptr
}

/// 撤销分类框窗口的 OLE 拖拽目标注册
pub fn unregister_box_drop_target(hwnd: HWND, target_ptr: *mut c_void) {
    unsafe {
        RevokeDragDrop(hwnd);
        if !target_ptr.is_null() {
            drop_target_release(target_ptr);
        }
    }
    log::info!("已撤销拖拽目标: hwnd={:?}", hwnd);
}

#[cfg(test)]
mod pending_drop_tests {
    use super::*;

    #[test]
    fn test_pending_drop_with_insert_state() {
        let pending = PendingDrop {
            file_paths: vec!["test.lnk".to_string()],
            box_data: None,
            drop_x: 100,
            drop_y: 200,
            insert_col: 2,
            insert_row: 3,
        };

        assert_eq!(pending.file_paths.len(), 1);
        assert_eq!(pending.file_paths[0], "test.lnk");
        assert_eq!(pending.drop_x, 100);
        assert_eq!(pending.drop_y, 200);
        assert_eq!(pending.insert_col, 2);
        assert_eq!(pending.insert_row, 3);
        assert!(pending.box_data.is_none());
    }

    #[test]
    fn test_pending_drop_no_insert_mode() {
        let pending = PendingDrop {
            file_paths: vec!["file1.lnk".to_string(), "file2.lnk".to_string()],
            box_data: None,
            drop_x: 50,
            drop_y: 75,
            insert_col: -1,
            insert_row: -1,
        };

        assert_eq!(pending.file_paths.len(), 2);
        assert_eq!(pending.insert_col, -1);
        assert_eq!(pending.insert_row, -1);
    }

    #[test]
    fn test_take_pending_drop_empty() {
        // Clear any existing pending drop
        if let Ok(mut g) = PENDING_DROP.lock() {
            *g = None;
        }

        let result = take_pending_drop();
        assert!(result.is_none());
    }

    #[test]
    fn test_take_pending_drop_with_data() {
        // Set up a pending drop
        if let Ok(mut g) = PENDING_DROP.lock() {
            *g = Some(PendingDrop {
                file_paths: vec!["test.lnk".to_string()],
                box_data: None,
                drop_x: 100,
                drop_y: 200,
                insert_col: 1,
                insert_row: 2,
            });
        }

        let result = take_pending_drop();
        assert!(result.is_some());

        let drop = result.unwrap();
        assert_eq!(drop.file_paths[0], "test.lnk");
        assert_eq!(drop.insert_col, 1);
        assert_eq!(drop.insert_row, 2);

        // Verify it's cleared after taking
        let result2 = take_pending_drop();
        assert!(result2.is_none());
    }
}
