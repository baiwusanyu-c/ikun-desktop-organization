//! OLE 拖拽目标实现（覆盖窗口专用）
//!
//! 为桌面覆盖窗口提供接收外部拖拽的能力：
//! - 接收从资源管理器拖入的文件（Explorer → 覆盖区域）
//! - 接收从桌面拖入的快捷方式（桌面 → 覆盖区域）
//!
//! 拖拽放置后，文件被复制到用户桌面，图标出现在鼠标释放位置。

use std::ffi::c_void;
use std::sync::Mutex;

use crate::platform::windows::ffi_imports::*;
use crate::platform::windows::ffi_types::*;
use crate::platform::windows::ole_dnd::IDataObjectVtbl;

// =============================================================================
// 拖拽放置结果（全局暂存，供 overlay_wnd_proc 读取）
// =============================================================================

/// 待处理的覆盖窗口拖拽放置结果
pub struct PendingOverlayDrop {
    /// 文件路径列表
    pub file_paths: Vec<String>,
    /// 放置目标的屏幕坐标
    pub drop_x: i32,
    pub drop_y: i32,
}

static PENDING_OVERLAY_DROP: Mutex<Option<PendingOverlayDrop>> = Mutex::new(None);

/// 取出待处理的覆盖窗口拖拽放置结果（由 overlay_wnd_proc 调用）
pub fn take_pending_overlay_drop() -> Option<PendingOverlayDrop> {
    PENDING_OVERLAY_DROP.lock().ok().and_then(|mut g| g.take())
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
struct OverlayDropTarget {
    vtbl: *const IDropTargetVtbl,
    ref_count: u32,
    target_hwnd: HWND,
}

static OVERLAY_DROP_TARGET_VTBL: IDropTargetVtbl = IDropTargetVtbl {
    query_interface: overlay_drop_query_interface,
    add_ref: overlay_drop_add_ref,
    release: overlay_drop_release,
    drag_enter: overlay_drop_drag_enter,
    drag_over: overlay_drop_drag_over,
    drag_leave: overlay_drop_drag_leave,
    drop: overlay_drop_drop,
};

// ----- IUnknown -----

unsafe extern "system" fn overlay_drop_query_interface(
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
        overlay_drop_add_ref(this);
        S_OK
    } else {
        *ppv = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn overlay_drop_add_ref(this: *mut c_void) -> u32 {
    let target = &mut *(this as *mut OverlayDropTarget);
    target.ref_count += 1;
    target.ref_count
}

unsafe extern "system" fn overlay_drop_release(this: *mut c_void) -> u32 {
    let target = &mut *(this as *mut OverlayDropTarget);
    target.ref_count -= 1;
    let rc = target.ref_count;
    if rc == 0 {
        let _ = Box::from_raw(this as *mut OverlayDropTarget);
    }
    rc
}

// ----- IDropTarget -----

unsafe extern "system" fn overlay_drop_drag_enter(
    this: *mut c_void,
    data_obj: *mut c_void,
    _ks: u32,
    _pt: i64,
    effect: *mut u32,
) -> HRESULT {
    let has_files = query_format(data_obj, CF_HDROP);
    if has_files {
        *effect = DROPEFFECT_COPY;
        // 设置外部拖拽悬停状态，使覆盖窗口背景不透明以接收 drop
        let target = &*(this as *const OverlayDropTarget);
        if let Some(data) = super::overlay_window::get_overlay_data(target.target_hwnd) {
            if !data.external_drag_hover {
                data.external_drag_hover = true;
                super::overlay_render::render_desktop_overlay(target.target_hwnd);
            }
        }
    } else {
        *effect = DROPEFFECT_NONE;
    }
    S_OK
}

unsafe extern "system" fn overlay_drop_drag_over(
    this: *mut c_void,
    _ks: u32,
    _pt: i64,
    effect: *mut u32,
) -> HRESULT {
    *effect = DROPEFFECT_COPY;
    
    // 显示拖拽覆盖可视化（snap-to-grid 预览）
    let target = &*(this as *const OverlayDropTarget);
    let mut cursor_pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor_pt);
    
    // 更新外部拖拽悬停状态
    {
        let overlay_data = match super::overlay_window::get_overlay_data(target.target_hwnd) {
            Some(d) => d,
            None => return S_OK,
        };
        if !overlay_data.external_drag_hover {
            overlay_data.external_drag_hover = true;
            super::overlay_render::render_desktop_overlay(target.target_hwnd);
        }
    }
    
    // 显示 snap-to-grid 预览
    super::overlay_wnd_proc::update_overlay_drag_over_visual(target.target_hwnd, cursor_pt.x, cursor_pt.y);
    
    S_OK
}

unsafe extern "system" fn overlay_drop_drag_leave(this: *mut c_void) -> HRESULT {
    let target = &*(this as *const OverlayDropTarget);
    if let Some(data) = super::overlay_window::get_overlay_data(target.target_hwnd) {
        if data.external_drag_hover {
            data.external_drag_hover = false;
            super::overlay_render::render_desktop_overlay(target.target_hwnd);
        }
    }
    S_OK
}

unsafe extern "system" fn overlay_drop_drop(
    this: *mut c_void,
    data_obj: *mut c_void,
    _ks: u32,
    pt: i64,
    effect: *mut u32,
) -> HRESULT {
    let target = &*(this as *const OverlayDropTarget);
    let drop_x = (pt & 0xFFFFFFFF) as i32;
    let drop_y = (pt >> 32) as i32;

    let file_paths = read_drop_files(data_obj);
    if file_paths.is_empty() {
        *effect = DROPEFFECT_NONE;
        log::warn!("覆盖窗口拖拽放置: 未读取到文件路径");
        return S_OK;
    }

    log::info!(
        "覆盖窗口 OLE drop: {} 个文件, drop=({}, {})",
        file_paths.len(),
        drop_x,
        drop_y
    );

    if let Ok(mut g) = PENDING_OVERLAY_DROP.lock() {
        *g = Some(PendingOverlayDrop {
            file_paths,
            drop_x,
            drop_y,
        });
    }

    // 清除外部拖拽悬停状态和拖拽覆盖可视化
    if let Some(data) = super::overlay_window::get_overlay_data(target.target_hwnd) {
        data.external_drag_hover = false;
        data.drag_over_item = -1;
        data.drag_over_insert_col = -1;
        data.drag_over_insert_row = -1;
    }
    PostMessageW(target.target_hwnd, WM_USER_OVERLAY_DROP, 0, 0);
    *effect = DROPEFFECT_COPY;
    log::info!("覆盖窗口拖拽放置完成（数据已暂存）");
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

// =============================================================================
// 公共 API
// =============================================================================

/// 为覆盖窗口注册 OLE 拖拽目标
pub fn register_overlay_drop_target(hwnd: HWND) -> *mut c_void {
    let target = Box::new(OverlayDropTarget {
        vtbl: &OVERLAY_DROP_TARGET_VTBL,
        ref_count: 1,
        target_hwnd: hwnd,
    });
    let ptr = Box::into_raw(target) as *mut c_void;
    let hr = unsafe { RegisterDragDrop(hwnd, ptr) };
    if hr != S_OK {
        log::warn!("覆盖窗口 RegisterDragDrop 失败: hr={:#010X}", hr as u32);
        unsafe { overlay_drop_release(ptr) };
        return std::ptr::null_mut();
    }
    log::info!("覆盖窗口已注册拖拽目标: hwnd={:?}", hwnd);
    ptr
}

/// 撤销覆盖窗口的 OLE 拖拽目标注册
pub fn unregister_overlay_drop_target(hwnd: HWND, target_ptr: *mut c_void) {
    unsafe {
        RevokeDragDrop(hwnd);
        if !target_ptr.is_null() {
            overlay_drop_release(target_ptr);
        }
    }
    log::info!("覆盖窗口已撤销拖拽目标: hwnd={:?}", hwnd);
}
