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

/// 框→框移动：暂存待添加项信息（由 drag_handler 调用）
static PENDING_ADD_ITEM: Mutex<Option<(String, String)>> = Mutex::new(None);

/// 设置待添加项（框→框移动时，源框调用）
pub fn set_pending_add_item(file_path: String, display_name: String) {
    if let Ok(mut g) = PENDING_ADD_ITEM.lock() {
        *g = Some((file_path, display_name));
    }
}

/// 取出待添加项（框→框移动时，目标框调用）
pub fn take_pending_add_item() -> Option<(String, String)> {
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
    if has_box || has_hdrop {
        *effect = DROPEFFECT_MOVE;
        PostMessageW(target.target_hwnd, WM_USER_DRAG_ENTER, 0, 0);
        log::debug!("拖拽进入: box={} hdrop={}", has_box, has_hdrop);
    } else {
        *effect = DROPEFFECT_NONE;
    }
    S_OK
}

unsafe extern "system" fn drop_target_drag_over(
    _this: *mut c_void,
    _ks: u32,
    _pt: i64,
    effect: *mut u32,
) -> HRESULT {
    if *effect & DROPEFFECT_MOVE != 0 {
        *effect = DROPEFFECT_MOVE;
    }
    S_OK
}

unsafe extern "system" fn drop_target_drag_leave(this: *mut c_void) -> HRESULT {
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

    let file_paths = read_drop_files(data_obj);
    if file_paths.is_empty() {
        *effect = DROPEFFECT_NONE;
        log::warn!("拖拽放置: 未读取到文件路径");
        return S_OK;
    }

    if let Ok(mut g) = PENDING_DROP.lock() {
        *g = Some(PendingDrop {
            file_paths: file_paths.clone(),
            box_data: box_data.clone(),
            drop_x,
            drop_y,
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
