//! OLE 拖拽源实现（IDataObject + IDropSource）
//!

#![allow(dead_code)]
// COM vtable 函数通过函数指针调用，编译器无法静态追踪；IEnumFORMATETC 为架构预留
//! 为分类框提供标准 OLE 拖拽能力：
//! - 从分类框拖出图标到另一个分类框（框间移动）
//! - 从分类框拖出图标到桌面空白区域（恢复到桌面）
//!
//! # 数据格式
//!
//! | 格式 | 用途 | 目标 |
//! |------|------|------|
//! | CF_HDROP | 文件路径列表 | 桌面/Explorer/其他应用 |
//! | IkunBoxDragData（自定义）| 框间内部移动元数据 | 另一个分类框 |
//!
//! # COM 实现
//!
//! IDataObject 和 IDropSource 通过手动 vtable 实现（与 shell_menu.rs 一致）。
//! 对象在堆上分配（Box::into_raw），通过引用计数管理生命周期。

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use super::ffi_imports::*;
use super::ffi_types::*;

/// 自定义剪贴板格式名称（框间拖拽数据）
const BOX_DRAG_FORMAT_NAME: &[u16] = &[
    73, 107, 117, 110, 66, 111, 120, 68, 114, 97, 103, 68, 97, 116, 97, 0,
]; // "IkunBoxDragData"

/// 缓存的自定义格式 ID（首次使用时注册）
static BOX_DRAG_FORMAT_ID: AtomicU32 = AtomicU32::new(0);

/// 获取或注册自定义剪贴板格式 ID
pub fn get_box_drag_format_id() -> u32 {
    let cached = BOX_DRAG_FORMAT_ID.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let id = unsafe { RegisterClipboardFormatW(BOX_DRAG_FORMAT_NAME.as_ptr()) };
    if id != 0 {
        BOX_DRAG_FORMAT_ID.store(id, Ordering::Relaxed);
        log::info!("注册自定义剪贴板格式 IkunBoxDragData: id={}", id);
    }
    id
}

// =============================================================================
// IDataObject vtable 定义
// =============================================================================

#[repr(C)]
#[allow(non_snake_case)]
pub(crate) struct IDataObjectVtbl {
    // IUnknown
    pub(crate) query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    pub(crate) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(crate) release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IDataObject
    pub(crate) get_data:
        unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    pub(crate) get_data_here:
        unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    pub(crate) query_get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC) -> HRESULT,
    pub(crate) get_canonical_format_etc:
        unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut FORMATETC) -> HRESULT,
    pub(crate) set_data:
        unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM, i32) -> HRESULT,
    pub(crate) enum_format_etc:
        unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    pub(crate) d_advise: unsafe extern "system" fn(
        *mut c_void,
        *const FORMATETC,
        u32,
        *mut c_void,
        *mut u32,
    ) -> HRESULT,
    d_unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    enum_d_advise: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

// =============================================================================
// IDropSource vtable 定义
// =============================================================================

#[repr(C)]
#[allow(non_snake_case)]
struct IDropSourceVtbl {
    // IUnknown
    query_interface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IDropSource
    query_continue_drag: unsafe extern "system" fn(*mut c_void, i32, u32) -> HRESULT,
    give_feedback: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

// =============================================================================
// BoxDataObject — 拖拽数据对象
// =============================================================================

/// 分类框拖拽数据对象（实现 IDataObject）
///
/// 提供两种数据格式：
/// - CF_HDROP：文件路径列表（用于桌面/Explorer 拖放）
/// - IkunBoxDragData：自定义格式（用于框间内部移动）
#[repr(C)]
struct BoxDataObject {
    vtbl: *const IDataObjectVtbl,
    ref_count: u32,
    /// 文件路径列表（用于 CF_HDROP）
    file_paths: *const Vec<String>,
    /// 框间拖拽数据（JSON 序列化后存储）
    box_data_json: *const Vec<u8>,
    /// 自定义格式 ID
    custom_format_id: u32,
    /// SetData 存储的拖拽图像数据（IDragSourceHelper 写入）
    drag_image_medium: *mut STGMEDIUM,
    drag_image_format: u32,
}

/// BoxDataObject 的 vtable 静态实例
static DATA_OBJECT_VTBL: IDataObjectVtbl = IDataObjectVtbl {
    query_interface: data_object_query_interface,
    add_ref: data_object_add_ref,
    release: data_object_release,
    get_data: data_object_get_data,
    get_data_here: data_object_get_data_here,
    query_get_data: data_object_query_get_data,
    get_canonical_format_etc: data_object_get_canonical_format_etc,
    set_data: data_object_set_data,
    enum_format_etc: data_object_enum_format_etc,
    d_advise: data_object_d_advise,
    d_unadvise: data_object_d_unadvise,
    enum_d_advise: data_object_enum_d_advise,
};

// ----- IDataObject 方法实现 -----

unsafe extern "system" fn data_object_query_interface(
    this: *mut c_void,
    riid: *const u8,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }

    // 比较 IID（16 字节）
    let iid = std::slice::from_raw_parts(riid, 16);

    if iid == IID_IUNKNOWN || iid == IID_IDATAOBJECT {
        *ppv = this;
        data_object_add_ref(this);
        S_OK
    } else {
        *ppv = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn data_object_add_ref(this: *mut c_void) -> u32 {
    let obj = &mut *(this as *mut BoxDataObject);
    obj.ref_count += 1;
    obj.ref_count
}

unsafe extern "system" fn data_object_release(this: *mut c_void) -> u32 {
    let obj = &mut *(this as *mut BoxDataObject);
    obj.ref_count -= 1;
    if obj.ref_count == 0 {
        // 释放堆上的数据
        drop(Box::from_raw(obj.file_paths as *mut Vec<String>));
        drop(Box::from_raw(obj.box_data_json as *mut Vec<u8>));
        // 释放 SetData 存储的拖拽图像数据
        if !obj.drag_image_medium.is_null() {
            let medium = &*obj.drag_image_medium;
            if medium.tymed == TYMED_HGLOBAL as u32
                && !medium.h_global.is_null()
                && medium.unk_for_release.is_null()
            {
                GlobalFree(medium.h_global);
            }
            drop(Box::from_raw(obj.drag_image_medium));
        }
        drop(Box::from_raw(obj));
        return 0;
    }
    obj.ref_count
}

unsafe extern "system" fn data_object_get_data(
    this: *mut c_void,
    format_etc: *const FORMATETC,
    medium: *mut STGMEDIUM,
) -> HRESULT {
    if format_etc.is_null() || medium.is_null() {
        return E_INVALIDARG;
    }

    let obj = &*(this as *const BoxDataObject);
    let fmt = &*format_etc;

    // 只支持 TYMED_HGLOBAL + DVASPECT_CONTENT
    if fmt.dw_aspect != DVASPECT_CONTENT || fmt.tymed & TYMED_HGLOBAL == 0 {
        return DV_E_FORMATETC;
    }

    if fmt.cf_format == obj.custom_format_id && obj.custom_format_id != 0 {
        // 构建自定义框间数据
        let json_data = &*obj.box_data_json;
        match build_hglobal_from_bytes(json_data) {
            Some(hglobal) => {
                (*medium).tymed = TYMED_HGLOBAL as u32;
                (*medium).h_global = hglobal;
                (*medium).unk_for_release = std::ptr::null_mut();
                S_OK
            }
            None => E_OUTOFMEMORY,
        }
    } else if !obj.drag_image_medium.is_null() && fmt.cf_format == obj.drag_image_format {
        // 返回 SetData 存储的拖拽图像数据的副本（调用者负责释放）
        let stored = &*obj.drag_image_medium;
        if stored.tymed == TYMED_HGLOBAL as u32 && !stored.h_global.is_null() {
            extern "system" {
                fn GlobalSize(hMem: HGLOBAL) -> usize;
            }
            let size = GlobalSize(stored.h_global);
            if size > 0 {
                let src = GlobalLock(stored.h_global);
                if !src.is_null() {
                    let copy = GlobalAlloc(GHND, size);
                    if !copy.is_null() {
                        let dst = GlobalLock(copy);
                        if !dst.is_null() {
                            std::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, size);
                            GlobalUnlock(copy);
                            GlobalUnlock(stored.h_global);
                            (*medium).tymed = TYMED_HGLOBAL as u32;
                            (*medium).h_global = copy;
                            (*medium).unk_for_release = std::ptr::null_mut();
                            return S_OK;
                        }
                        GlobalFree(copy);
                    }
                    GlobalUnlock(stored.h_global);
                }
            }
        }
        DV_E_FORMATETC
    } else if fmt.cf_format == CF_HDROP {
        let file_paths = &*obj.file_paths;
        match build_drop_files_hglobal(file_paths) {
            Some(hglobal) => {
                (*medium).tymed = TYMED_HGLOBAL as u32;
                (*medium).h_global = hglobal;
                (*medium).unk_for_release = std::ptr::null_mut();
                S_OK
            }
            None => E_OUTOFMEMORY,
        }
    } else {
        DV_E_FORMATETC
    }
}

unsafe extern "system" fn data_object_get_data_here(
    _this: *mut c_void,
    _format_etc: *const FORMATETC,
    _medium: *mut STGMEDIUM,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn data_object_query_get_data(
    this: *mut c_void,
    format_etc: *const FORMATETC,
) -> HRESULT {
    if format_etc.is_null() {
        return E_INVALIDARG;
    }

    let obj = &*(this as *const BoxDataObject);
    let fmt = &*format_etc;

    if (fmt.cf_format == obj.custom_format_id && obj.custom_format_id != 0)
        || (!obj.drag_image_medium.is_null() && fmt.cf_format == obj.drag_image_format)
        || fmt.cf_format == CF_HDROP
    {
        S_OK
    } else {
        DV_E_FORMATETC
    }
}

unsafe extern "system" fn data_object_get_canonical_format_etc(
    _this: *mut c_void,
    _format_etc_in: *const FORMATETC,
    _format_etc_out: *mut FORMATETC,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn data_object_set_data(
    this: *mut c_void,
    format_etc: *const FORMATETC,
    medium: *mut STGMEDIUM,
    _f_release: i32,
) -> HRESULT {
    if format_etc.is_null() || medium.is_null() {
        return E_INVALIDARG;
    }
    let obj = &mut *(this as *mut BoxDataObject);

    // 释放旧的存储数据
    if !obj.drag_image_medium.is_null() {
        let old = &*obj.drag_image_medium;
        if old.tymed == TYMED_HGLOBAL as u32
            && !old.h_global.is_null()
            && old.unk_for_release.is_null()
        {
            GlobalFree(old.h_global);
        }
        drop(Box::from_raw(obj.drag_image_medium));
        obj.drag_image_medium = std::ptr::null_mut();
        obj.drag_image_format = 0;
    }

    // 存储策略：始终复制 HGLOBAL，确保我们拥有独立副本
    if (*medium).tymed == TYMED_HGLOBAL as u32 {
        let src = (*medium).h_global;
        if src.is_null() {
            return E_INVALIDARG;
        }

        let ptr = GlobalLock(src);
        if ptr.is_null() {
            return E_UNEXPECTED;
        }

        // 使用 GlobalSize 获取实际大小（添加 FFI）
        extern "system" {
            fn GlobalSize(hMem: HGLOBAL) -> usize;
        }
        let size = GlobalSize(src);
        if size == 0 {
            GlobalUnlock(src);
            return E_UNEXPECTED;
        }

        let copy = GlobalAlloc(GHND, size);
        if copy.is_null() {
            GlobalUnlock(src);
            return E_OUTOFMEMORY;
        }

        let dst = GlobalLock(copy);
        if dst.is_null() {
            GlobalFree(copy);
            GlobalUnlock(src);
            return E_OUTOFMEMORY;
        }

        std::ptr::copy_nonoverlapping(ptr as *const u8, dst as *mut u8, size);
        GlobalUnlock(copy);
        GlobalUnlock(src);

        // 存储副本（我们拥有此 HGLOBAL）
        let stored = Box::new(STGMEDIUM {
            tymed: TYMED_HGLOBAL as u32,
            h_global: copy,
            unk_for_release: std::ptr::null_mut(), // 我们自己管理释放
        });
        obj.drag_image_medium = Box::into_raw(stored);
        obj.drag_image_format = (*format_etc).cf_format;
    } else {
        // 非 HGLOBAL 类型：直接复制 STGMEDIUM 结构
        let stored = Box::new(*medium);
        obj.drag_image_medium = Box::into_raw(stored);
        obj.drag_image_format = (*format_etc).cf_format;
    }

    S_OK
}

unsafe extern "system" fn data_object_enum_format_etc(
    this: *mut c_void,
    direction: u32,
    ppenum: *mut *mut c_void,
) -> HRESULT {
    if ppenum.is_null() {
        return E_POINTER;
    }

    let obj = &*(this as *const BoxDataObject);

    // 仅支持 DATADIR_GET (1)
    if direction != 1 {
        *ppenum = std::ptr::null_mut();
        return E_NOTIMPL;
    }

    // 构建格式列表：CF_HDROP（文件路径）+ 自定义格式
    let mut count: u32 = 0;
    let mut formats = [FORMATETC {
        cf_format: 0,
        ptd: std::ptr::null_mut(),
        dw_aspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    }; 4];

    // CF_HDROP 格式（Explorer/其他应用使用）
    if !obj.file_paths.is_null() {
        let paths = &*obj.file_paths;
        if !paths.is_empty() {
            formats[count as usize] = FORMATETC {
                cf_format: CF_HDROP,
                ptd: std::ptr::null_mut(),
                dw_aspect: DVASPECT_CONTENT,
                lindex: -1,
                tymed: TYMED_HGLOBAL,
            };
            count += 1;
        }
    }

    // 自定义格式（框间移动）
    if obj.custom_format_id != 0 {
        formats[count as usize] = FORMATETC {
            cf_format: obj.custom_format_id,
            ptd: std::ptr::null_mut(),
            dw_aspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL,
        };
        count += 1;
    }

    // SetData 存储的拖拽图像格式（IDragSourceHelper 写入的）
    if !obj.drag_image_medium.is_null() && obj.drag_image_format != 0 {
        formats[count as usize] = FORMATETC {
            cf_format: obj.drag_image_format,
            ptd: std::ptr::null_mut(),
            dw_aspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL,
        };
        count += 1;
    }

    // 创建枚举器
    let enumerator = Box::new(BoxFormatEnum {
        vtbl: &FORMAT_ENUM_VTBL,
        ref_count: 1,
        formats,
        count,
        index: 0,
    });
    *ppenum = Box::into_raw(enumerator) as *mut c_void;
    S_OK
}

// =============================================================================
// IEnumFORMATETC — 格式枚举器
// =============================================================================

#[repr(C)]
#[allow(non_snake_case)]
struct IEnumFORMATETCVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    next: unsafe extern "system" fn(*mut c_void, u32, *mut FORMATETC, *mut u32) -> HRESULT,
    skip: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    clone: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

/// 格式枚举器（实现 IEnumFORMATETC）
#[repr(C)]
struct BoxFormatEnum {
    vtbl: *const IEnumFORMATETCVtbl,
    ref_count: u32,
    formats: [FORMATETC; 4],
    count: u32,
    index: u32,
}

// IID_IEnumFORMATETC = {00000103-0000-0000-C000-000000000046}
const IID_IENUMFORMATETC: [u8; 16] = [
    0x03, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

static FORMAT_ENUM_VTBL: IEnumFORMATETCVtbl = IEnumFORMATETCVtbl {
    query_interface: format_enum_query_interface,
    add_ref: format_enum_add_ref,
    release: format_enum_release,
    next: format_enum_next,
    skip: format_enum_skip,
    reset: format_enum_reset,
    clone: format_enum_clone,
};

unsafe extern "system" fn format_enum_query_interface(
    this: *mut c_void,
    riid: *const u8,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    let iid = std::slice::from_raw_parts(riid, 16);
    if iid == IID_IUNKNOWN || iid == IID_IENUMFORMATETC {
        *ppv = this;
        format_enum_add_ref(this);
        S_OK
    } else {
        *ppv = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn format_enum_add_ref(this: *mut c_void) -> u32 {
    let obj = &mut *(this as *mut BoxFormatEnum);
    obj.ref_count += 1;
    obj.ref_count
}

unsafe extern "system" fn format_enum_release(this: *mut c_void) -> u32 {
    let obj = &mut *(this as *mut BoxFormatEnum);
    obj.ref_count -= 1;
    if obj.ref_count == 0 {
        drop(Box::from_raw(obj));
        return 0;
    }
    obj.ref_count
}

unsafe extern "system" fn format_enum_next(
    this: *mut c_void,
    celt: u32,
    rgelt: *mut FORMATETC,
    pcelt_fetched: *mut u32,
) -> HRESULT {
    let obj = &mut *(this as *mut BoxFormatEnum);
    let mut fetched: u32 = 0;

    while fetched < celt && obj.index < obj.count {
        *rgelt.add(fetched as usize) = obj.formats[obj.index as usize];
        fetched += 1;
        obj.index += 1;
    }

    if !pcelt_fetched.is_null() {
        *pcelt_fetched = fetched;
    }

    if fetched == celt {
        S_OK
    } else {
        S_FALSE
    }
}

unsafe extern "system" fn format_enum_skip(this: *mut c_void, celt: u32) -> HRESULT {
    let obj = &mut *(this as *mut BoxFormatEnum);
    obj.index += celt;
    if obj.index < obj.count {
        S_OK
    } else {
        obj.index = obj.count;
        S_FALSE
    }
}

unsafe extern "system" fn format_enum_reset(this: *mut c_void) -> HRESULT {
    let obj = &mut *(this as *mut BoxFormatEnum);
    obj.index = 0;
    S_OK
}

unsafe extern "system" fn format_enum_clone(
    this: *mut c_void,
    ppenum: *mut *mut c_void,
) -> HRESULT {
    if ppenum.is_null() {
        return E_POINTER;
    }
    let obj = &*(this as *const BoxFormatEnum);
    let cloned = Box::new(BoxFormatEnum {
        vtbl: obj.vtbl,
        ref_count: 1,
        formats: obj.formats,
        count: obj.count,
        index: obj.index,
    });
    *ppenum = Box::into_raw(cloned) as *mut c_void;
    S_OK
}

unsafe extern "system" fn data_object_d_advise(
    _this: *mut c_void,
    _format_etc: *const FORMATETC,
    _flags: u32,
    _sink: *mut c_void,
    _connection: *mut u32,
) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn data_object_d_unadvise(_this: *mut c_void, _connection: u32) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn data_object_enum_d_advise(
    _this: *mut c_void,
    _ppenum: *mut *mut c_void,
) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

// =============================================================================
// BoxDropSource — 拖拽源
// =============================================================================

/// 分类框拖拽源（实现 IDropSource）
///
/// 控制拖拽过程中的光标反馈和取消逻辑。
#[repr(C)]
struct BoxDropSource {
    vtbl: *const IDropSourceVtbl,
    ref_count: u32,
}

static DROP_SOURCE_VTBL: IDropSourceVtbl = IDropSourceVtbl {
    query_interface: drop_source_query_interface,
    add_ref: drop_source_add_ref,
    release: drop_source_release,
    query_continue_drag: drop_source_query_continue_drag,
    give_feedback: drop_source_give_feedback,
};

unsafe extern "system" fn drop_source_query_interface(
    this: *mut c_void,
    riid: *const u8,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }

    let iid = std::slice::from_raw_parts(riid, 16);

    if iid == IID_IUNKNOWN || iid == IID_IDROPSOURCE {
        *ppv = this;
        drop_source_add_ref(this);
        S_OK
    } else {
        *ppv = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn drop_source_add_ref(this: *mut c_void) -> u32 {
    let obj = &mut *(this as *mut BoxDropSource);
    obj.ref_count += 1;
    obj.ref_count
}

unsafe extern "system" fn drop_source_release(this: *mut c_void) -> u32 {
    let obj = &mut *(this as *mut BoxDropSource);
    obj.ref_count -= 1;
    if obj.ref_count == 0 {
        drop(Box::from_raw(obj));
        return 0;
    }
    obj.ref_count
}

/// 检查是否应继续拖拽
///
/// - 按 Esc 键 → 取消拖拽（DRAGDROP_S_CANCEL）
/// - 鼠标左键释放 → 执行放置（DRAGDROP_S_DROP）
/// - 其他 → 继续（S_OK）
unsafe extern "system" fn drop_source_query_continue_drag(
    _this: *mut c_void,
    escape_pressed: i32,
    _key_state: u32,
) -> HRESULT {
    if escape_pressed != 0 {
        return DRAGDROP_S_CANCEL;
    }
    // 使用 GetAsyncKeyState 检查物理鼠标状态（避免消息队列状态不准确）
    let lbutton_down = GetAsyncKeyState(VK_LBUTTON) & 0x8000u16 as i16 != 0;
    if !lbutton_down {
        return DRAGDROP_S_DROP;
    }
    S_OK
}

/// 设置拖拽光标反馈
///
/// 返回 DRAGDROP_S_USEDEFAULTCURSORS 让系统自动处理光标。
unsafe extern "system" fn drop_source_give_feedback(_this: *mut c_void, _effect: u32) -> HRESULT {
    // 更新拖拽残影位置，保持残影跟随光标
    super::drag_image::update_drag_overlay();
    // 返回 S_OK 表示我们自己处理了反馈（不使用系统默认光标）
    S_OK
}

// =============================================================================
// 辅助函数
// =============================================================================

/// DV_E_FORMATETC 常量（DATA_S_SAMEFORMATETC 的负值）
const DV_E_FORMATETC: HRESULT = -2147221400; // 0x80040064

/// 构建 CF_HDROP 格式的 HGLOBAL
///
/// 数据布局：
/// ```text
/// [DROPFILES (20 bytes)]
/// [file1\0 (UTF-16)]
/// [file2\0 (UTF-16)]
/// [\0\0 (double null terminator)]
/// ```
unsafe fn build_drop_files_hglobal(file_paths: &[String]) -> Option<HGLOBAL> {
    if file_paths.is_empty() {
        return None;
    }

    // 计算总大小
    let dropfiles_size = std::mem::size_of::<DROPFILES>();

    // 每个路径编码为 UTF-16 + null terminator
    let mut total_chars: usize = 0;
    let mut encoded_paths: Vec<Vec<u16>> = Vec::with_capacity(file_paths.len());

    for path in file_paths {
        let mut wide: Vec<u16> = path.encode_utf16().collect();
        wide.push(0); // null terminator
        total_chars += wide.len();
        encoded_paths.push(wide);
    }
    total_chars += 1; // double null terminator

    let total_bytes = dropfiles_size + total_chars * 2;

    // 分配全局内存
    let hglobal = GlobalAlloc(GHND, total_bytes);
    if hglobal.is_null() {
        return None;
    }

    let ptr = GlobalLock(hglobal);
    if ptr.is_null() {
        GlobalFree(hglobal);
        return None;
    }

    // 写入 DROPFILES 头部
    let dropfiles = DROPFILES {
        p_files: dropfiles_size as u32,
        pt_x: 0,
        pt_y: 0,
        f_nc: 0,
        f_wide: 1, // UTF-16
    };
    std::ptr::copy_nonoverlapping(
        &dropfiles as *const DROPFILES as *const u8,
        ptr as *mut u8,
        dropfiles_size,
    );

    // 写入文件路径
    let mut offset = dropfiles_size;
    for wide in &encoded_paths {
        let byte_len = wide.len() * 2;
        std::ptr::copy_nonoverlapping(
            wide.as_ptr() as *const u8,
            (ptr as *mut u8).add(offset),
            byte_len,
        );
        offset += byte_len;
    }
    // double null terminator（已由 GHND/GMEM_ZEROINIT 初始化为 0）

    GlobalUnlock(hglobal);
    Some(hglobal)
}

/// 从字节数组构建 HGLOBAL（追加 null terminator 便于读取）
unsafe fn build_hglobal_from_bytes(data: &[u8]) -> Option<HGLOBAL> {
    if data.is_empty() {
        return None;
    }

    // 额外分配 1 字节 null terminator（GHND 自动清零）
    let hglobal = GlobalAlloc(GHND, data.len() + 1);
    if hglobal.is_null() {
        return None;
    }

    let ptr = GlobalLock(hglobal);
    if ptr.is_null() {
        GlobalFree(hglobal);
        return None;
    }

    std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
    GlobalUnlock(hglobal);
    Some(hglobal)
}

// 2026-11-04 自检审计：STGMEDIUM 读取/释放与 Shell 数据边界校验集中在本模块，
// 避免 drop_target.rs 超过单文件行数上限；调用方仍只消费解析后的安全数据。
// windows crate 的高层 ReleaseStgMedium 受 Win32_System_Com_StructuredStorage
// feature 门控；本任务不允许改 Cargo.toml，故用 windows::core::link! 绑定同一
// ole32!ReleaseStgMedium 符号，STGMEDIUM ABI 与 Win32::System::Com 保持一致。
windows::core::link!("ole32.dll" "system" fn ReleaseStgMedium(pmedium: *mut STGMEDIUM));

#[link(name = "kernel32")]
extern "system" {
    fn GlobalSize(h_mem: HGLOBAL) -> usize;
}

// 2026-11-04 自检审计：所有成功 GetData 的 STGMEDIUM 都交给 RAII 统一释放，
// 由 OLE 按 tymed 与 pUnkForRelease 做正确结算，避免手工 GlobalFree 漏释放或误释放。
struct StgMediumGuard {
    medium: *mut STGMEDIUM,
}

impl Drop for StgMediumGuard {
    fn drop(&mut self) {
        unsafe {
            ReleaseStgMedium(self.medium);
        }
    }
}

struct GlobalLockGuard {
    h_global: HGLOBAL,
    ptr: *mut c_void,
}

impl GlobalLockGuard {
    unsafe fn lock(h_global: HGLOBAL) -> Option<Self> {
        let ptr = GlobalLock(h_global);
        if ptr.is_null() {
            None
        } else {
            Some(Self { h_global, ptr })
        }
    }
}

impl Drop for GlobalLockGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalUnlock(self.h_global);
        }
    }
}

unsafe fn pidl_within_bounds(base: *const u8, offset: usize, size: usize) -> bool {
    let mut position = offset;
    loop {
        let Some(item_id_end) = position.checked_add(2) else {
            return false;
        };
        if item_id_end > size {
            return false;
        }
        let cb = std::ptr::read_unaligned(base.add(position) as *const u16) as usize;
        if cb == 0 {
            return true;
        }
        if cb < 2 {
            return false;
        }
        let Some(next) = position.checked_add(cb) else {
            return false;
        };
        if next > size {
            return false;
        }
        position = next;
    }
}

pub(crate) unsafe fn read_drop_files(data_obj: *mut c_void) -> Vec<String> {
    const MAX_DROP_FILE_CHARS: usize = 64 * 1024;

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
    if ((*vtbl).get_data)(data_obj, &fe, &mut medium) != S_OK {
        return Vec::new();
    }
    let _medium_guard = StgMediumGuard {
        medium: &mut medium,
    };
    if medium.tymed != TYMED_HGLOBAL as u32 || medium.h_global.is_null() {
        log::warn!("[DROP] CF_HDROP medium 类型无效: tymed={:#X}", medium.tymed);
        return Vec::new();
    }

    let size = GlobalSize(medium.h_global);
    if size < std::mem::size_of::<DROPFILES>() {
        log::warn!("[DROP] CF_HDROP GlobalSize 过小: {}", size);
        return Vec::new();
    }
    let Some(lock) = GlobalLockGuard::lock(medium.h_global) else {
        log::warn!("[DROP] CF_HDROP GlobalLock 失败");
        return Vec::new();
    };

    let dropfiles = std::ptr::read_unaligned(lock.ptr as *const DROPFILES);
    if dropfiles.f_wide == 0 {
        // 2026-11-04 自检审计：ANSI DROPFILES 不静默返回空列表，明确拒绝并记录。
        log::warn!("[DROP] CF_HDROP 为 ANSI 格式（f_wide=0），明确拒绝");
        return Vec::new();
    }

    let header_size = std::mem::size_of::<DROPFILES>();
    let files_offset = dropfiles.p_files as usize;
    if files_offset < header_size || files_offset >= size || !files_offset.is_multiple_of(2) {
        log::warn!(
            "[DROP] CF_HDROP p_files 越界或未对齐: offset={}, size={}",
            files_offset,
            size
        );
        return Vec::new();
    }

    let mut paths = Vec::new();
    let mut offset = files_offset;
    let mut terminated = false;
    let mut total_chars = 0usize;
    while offset <= size.saturating_sub(2) {
        let mut byte_len = 0usize;
        let mut found_terminator = false;
        while offset.saturating_add(byte_len).saturating_add(2) <= size {
            let code_unit = std::ptr::read_unaligned(
                (lock.ptr as *const u8).add(offset + byte_len) as *const u16,
            );
            if code_unit == 0 {
                found_terminator = true;
                break;
            }
            byte_len += 2;
            total_chars += 1;
            if total_chars > MAX_DROP_FILE_CHARS {
                log::warn!("[DROP] CF_HDROP 字符串总量超过上限，拒绝继续读取");
                return Vec::new();
            }
        }
        if !found_terminator {
            log::warn!("[DROP] CF_HDROP 字符串缺少终止符，拒绝越界读取");
            break;
        }
        if byte_len == 0 {
            terminated = true;
            break;
        }
        let units = (0..byte_len / 2)
            .map(|index| {
                std::ptr::read_unaligned(
                    (lock.ptr as *const u8).add(offset + index * 2) as *const u16
                )
            })
            .collect::<Vec<_>>();
        if let Ok(path) = String::from_utf16(&units) {
            if !path.is_empty() {
                paths.push(path);
            }
        }
        offset += byte_len + 2;
    }
    if !terminated {
        log::warn!("[DROP] CF_HDROP 列表未以双 NUL 结束，已丢弃不完整结果");
        paths.clear();
    }
    paths
}

/// 从 IDataObject 读取 Shell IDList 格式的 PIDL，提取虚拟 Shell 对象的 CLSID 路径。
///
/// 当从桌面拖拽回收站、此电脑等虚拟对象时，CF_HDROP 为空，
/// 需要从 CFSTR_SHELLIDLIST 格式读取 PIDL，然后通过 SHGetFileInfoW + SHGFI_PIDL
/// 获取显示名称，再映射到已知的 CLSID 路径。
pub(crate) unsafe fn read_shell_idlist_paths(data_obj: *mut c_void) -> Vec<String> {
    const MAX_CIDL: u32 = 4096;

    if data_obj.is_null() {
        return Vec::new();
    }

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
    if ((*vtbl).get_data)(data_obj, &fe, &mut medium) != S_OK {
        log::debug!("[PIDL] 无 CFSTR_SHELLIDLIST 数据");
        return Vec::new();
    }
    let _medium_guard = StgMediumGuard {
        medium: &mut medium,
    };
    if medium.tymed != TYMED_HGLOBAL as u32 || medium.h_global.is_null() {
        log::warn!(
            "[PIDL] Shell IDList medium 类型无效: tymed={:#X}",
            medium.tymed
        );
        return Vec::new();
    }

    let size = GlobalSize(medium.h_global);
    if size < 4 {
        log::warn!("[PIDL] GlobalSize 过小: {}", size);
        return Vec::new();
    }
    let Some(lock) = GlobalLockGuard::lock(medium.h_global) else {
        log::warn!("[PIDL] GlobalLock 失败");
        return Vec::new();
    };

    let cidl = std::ptr::read_unaligned(lock.ptr as *const u32);
    if cidl > MAX_CIDL {
        log::warn!("[PIDL] cidl={} 超过安全上限 {}", cidl, MAX_CIDL);
        return Vec::new();
    }
    let table_entries = match (cidl as usize).checked_add(1) {
        Some(value) => value,
        None => return Vec::new(),
    };
    let table_bytes = match table_entries.checked_mul(std::mem::size_of::<u32>()) {
        Some(value) => value,
        None => return Vec::new(),
    };
    let table_end = match 4usize.checked_add(table_bytes) {
        Some(value) if value <= size => value,
        _ => {
            log::warn!(
                "[PIDL] 偏移表越界: cidl={}, table_end={}, size={}",
                cidl,
                table_bytes.saturating_add(4),
                size
            );
            return Vec::new();
        }
    };
    let offsets = (0..table_entries)
        .map(|index| {
            std::ptr::read_unaligned((lock.ptr as *const u8).add(4 + index * 4) as *const u32)
                as usize
        })
        .collect::<Vec<_>>();

    log::info!(
        "[PIDL] CIDA cidl={}, offsets={:?}",
        cidl,
        &offsets[..std::cmp::min(offsets.len(), 5)]
    );

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

    let known_objects: &[(&str, &str)] = &[
        ("回收站", "::{645FF040-5081-101B-9F08-00AA002F954E}"),
        ("此电脑", "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"),
        ("网络", "::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}"),
        ("控制面板", "::{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}"),
    ];

    let mut paths = Vec::new();
    for i in 1..=cidl as usize {
        let Some(&pidl_offset) = offsets.get(i) else {
            break;
        };
        if pidl_offset < table_end || pidl_offset >= size {
            log::warn!(
                "[PIDL] 第 {} 个 PIDL 偏移越界: offset={}, table_end={}, size={}",
                i,
                pidl_offset,
                table_end,
                size
            );
            continue;
        }
        if !pidl_within_bounds(lock.ptr as *const u8, pidl_offset, size) {
            log::warn!("[PIDL] 第 {} 个 PIDL 缺少有界终止符，已跳过", i);
            continue;
        }
        let pidl = (lock.ptr as *const u8).add(pidl_offset);

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
            let name_len = info
                .sz_display_name
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(0);
            let display_name = String::from_utf16_lossy(&info.sz_display_name[..name_len]);
            log::info!("[PIDL] 解析到显示名称: {}", display_name);

            let mut found = false;
            for (known_name, clsid) in known_objects {
                if display_name == *known_name {
                    paths.push(clsid.to_string());
                    log::info!("[PIDL] 映射到 CLSID: {} -> {}", display_name, clsid);
                    found = true;
                    break;
                }
            }
            if !found {
                log::warn!("[PIDL] 未找到 {} 对应的 CLSID 映射", display_name);
            }
        } else {
            log::warn!("[PIDL] SHGetFileInfoW 失败, offset={}", pidl_offset);
        }
    }
    log::info!(
        "[PIDL] 最终解析到 {} 个虚拟 Shell 对象: {:?}",
        paths.len(),
        paths
    );
    paths
}

pub(crate) unsafe fn read_box_drag_data(
    data_obj: *mut c_void,
    fmt_id: u32,
) -> Option<IkunBoxDragData> {
    const MAX_BOX_DRAG_JSON_BYTES: usize = 64 * 1024;

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
    if ((*vtbl).get_data)(data_obj, &fe, &mut medium) != S_OK {
        return None;
    }
    let _medium_guard = StgMediumGuard {
        medium: &mut medium,
    };
    if medium.tymed != TYMED_HGLOBAL as u32 || medium.h_global.is_null() {
        log::warn!(
            "[DROP] IkunBoxDragData medium 类型无效: tymed={:#X}",
            medium.tymed
        );
        return None;
    }

    let size = GlobalSize(medium.h_global);
    if size == 0 || size > MAX_BOX_DRAG_JSON_BYTES {
        log::warn!("[DROP] IkunBoxDragData 大小非法: {}", size);
        return None;
    }
    let Some(lock) = GlobalLockGuard::lock(medium.h_global) else {
        log::warn!("[DROP] IkunBoxDragData GlobalLock 失败");
        return None;
    };

    let bytes = lock.ptr as *const u8;
    let mut len = 0usize;
    // 2026-11-04 自检审计：先判上界再解引用，避免恶意/损坏 medium 造成越界扫描。
    while len < size && std::ptr::read_unaligned(bytes.add(len)) != 0 {
        len += 1;
    }
    if len == size {
        log::warn!("[DROP] IkunBoxDragData 缺少 NUL 终止符，拒绝解析");
        return None;
    }
    serde_json::from_slice::<IkunBoxDragData>(std::slice::from_raw_parts(bytes, len)).ok()
}

// =============================================================================
// 公共 API
// =============================================================================

/// 启动 OLE 拖拽操作
///
/// 在分类框的 WM_LBUTTONDOWN + 鼠标移动超过阈值后调用。
///
/// # 参数
///
/// - `file_paths`: 被拖拽的文件路径列表（用于 CF_HDROP）
/// - `box_drag_data`: 框间拖拽元数据（用于自定义格式）
/// - `icon_handle`: 被拖拽项的图标句柄（用于拖拽图像，可为 null）
///
/// # 返回值
///
/// 2026-11-04 自检审计：OLE 侧只协商 COPY；框间移动由自定义载荷与
/// WM_USER_DRAG_ITEM_REMOVED 消息显式驱动，避免 Shell 因 MOVE 重复删除源文件。
/// 返回 true 表示 drop 成功 (DRAGDROP_S_DROP)，false 表示取消。
pub fn start_box_drag(
    file_paths: Vec<String>,
    box_drag_data: IkunBoxDragData,
    _icon_handle: HICON,
) -> bool {
    let custom_format_id = get_box_drag_format_id();

    // 序列化框间数据为 JSON
    let box_data_json = match serde_json::to_vec(&box_drag_data) {
        Ok(json) => json,
        Err(e) => {
            log::error!("序列化拖拽数据失败: {}", e);
            return false;
        }
    };

    // 在堆上创建 COM 对象
    let file_paths_box = Box::new(file_paths);
    let box_data_box = Box::new(box_data_json);

    let data_obj = Box::new(BoxDataObject {
        vtbl: &DATA_OBJECT_VTBL,
        ref_count: 1,
        file_paths: Box::into_raw(file_paths_box),
        box_data_json: Box::into_raw(box_data_box),
        custom_format_id,
        drag_image_medium: std::ptr::null_mut(),
        drag_image_format: 0,
    });
    let data_obj_ptr = Box::into_raw(data_obj) as *mut c_void;

    // 拖拽图像已通过自定义覆盖窗口实现（drag_image.rs），不再使用 OLE IDragSourceHelper

    let drop_src = Box::new(BoxDropSource {
        vtbl: &DROP_SOURCE_VTBL,
        ref_count: 1,
    });
    let drop_src_ptr = Box::into_raw(drop_src) as *mut c_void;

    // 调用 DoDragDrop
    // 设置拖拽进行中标志：DoDragDrop 是阻塞调用，期间 WM_WINDOWPOSCHANGING 不干预 Z-order，
    // 否则分类框被强制 HWND_BOTTOM 后可能低于 overlay，导致 OLE hit-testing 失败。
    super::zorder::set_drag_in_progress(true);
    let mut effect = DROPEFFECT_NONE;
    let hr = unsafe { DoDragDrop(data_obj_ptr, drop_src_ptr, DROPEFFECT_COPY, &mut effect) };
    super::zorder::set_drag_in_progress(false);

    let drop_succeeded = hr == DRAGDROP_S_DROP;

    // 释放 COM 对象（DoDragDrop 内部会 AddRef，这里释放初始引用）
    unsafe {
        data_object_release(data_obj_ptr);
        drop_source_release(drop_src_ptr);
    }

    log::info!(
        "DoDragDrop: hr={:#010X}, effect={}, drop_succeeded={}",
        hr as u32,
        effect,
        drop_succeeded
    );

    drop_succeeded
}

// =============================================================================
// 单元测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ole_constants() {
        assert_eq!(CF_HDROP, 15);
        assert_eq!(DROPEFFECT_NONE, 0);
        assert_eq!(DROPEFFECT_MOVE, 2);
        assert_eq!(DROPEFFECT_COPY, 1);
        assert_eq!(TYMED_HGLOBAL, 1);
        assert_eq!(DVASPECT_CONTENT, 1);
    }

    #[test]
    fn test_hresult_constants() {
        assert_eq!(S_OK, 0);
        assert_eq!(S_FALSE, 1);
        assert_eq!(E_NOTIMPL, -2147467263);
        assert_eq!(E_NOINTERFACE, -2147467262);
        assert_eq!(E_POINTER, -2147467261);
        assert_eq!(DRAGDROP_S_DROP, 0x00040100);
        assert_eq!(DRAGDROP_S_CANCEL, 0x00040101);
    }

    #[test]
    fn test_iid_constants() {
        // IID_IUnknown 最后 8 字节应为 C0 00 00 00 00 00 00 46
        assert_eq!(IID_IUNKNOWN[8], 0xC0);
        assert_eq!(IID_IUNKNOWN[15], 0x46);

        // IID_IDataObject 第一组为 0E 01 00 00
        assert_eq!(IID_IDATAOBJECT[0], 0x0E);
        assert_eq!(IID_IDATAOBJECT[1], 0x01);

        // IID_IDropSource 第一组为 21 01 00 00
        assert_eq!(IID_IDROPSOURCE[0], 0x21);

        // IID_IDropTarget 第一组为 22 01 00 00
        assert_eq!(IID_IDROPTARGET[0], 0x22);
    }

    #[test]
    fn test_formatetc_size() {
        let size = std::mem::size_of::<FORMATETC>();
        if cfg!(target_pointer_width = "64") {
            // u32(4) + pad(4) + ptr(8) + u32(4) + i32(4) + i32(4) + pad(4) = 32
            assert_eq!(size, 32, "FORMATETC 在 64-bit 应为 32 字节（含对齐填充）");
        }
    }

    #[test]
    fn test_stgmedium_size() {
        let size = std::mem::size_of::<STGMEDIUM>();
        if cfg!(target_pointer_width = "64") {
            assert_eq!(size, 24, "STGMEDIUM 在 64-bit 应为 24 字节");
        }
    }

    #[test]
    fn test_dropfiles_size() {
        assert_eq!(
            std::mem::size_of::<DROPFILES>(),
            20,
            "DROPFILES 应为 20 字节"
        );
    }

    #[test]
    fn test_ikun_box_drag_data_serde() {
        let data = IkunBoxDragData {
            source_hwnd: 0x12345,
            source_config_id: "test-uuid".to_string(),
            items: vec![
                IkunDragItemData {
                    index: 0,
                    file_path: "C:\\Users\\test\\Desktop\\app.lnk".to_string(),
                    display_name: "应用".to_string(),
                },
                IkunDragItemData {
                    index: 1,
                    file_path: "C:\\Users\\test\\Desktop\\game.lnk".to_string(),
                    display_name: "游戏".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&data).unwrap();
        let parsed: IkunBoxDragData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.source_hwnd, 0x12345);
        assert_eq!(parsed.source_config_id, "test-uuid");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            parsed.items[0].file_path,
            "C:\\Users\\test\\Desktop\\app.lnk"
        );
        assert_eq!(parsed.items[1].display_name, "游戏");
    }

    #[test]
    fn test_ikun_drag_item_data_single_item() {
        let data = IkunBoxDragData {
            source_hwnd: 100,
            source_config_id: "uuid-1".to_string(),
            items: vec![IkunDragItemData {
                index: 3,
                file_path: "test.lnk".to_string(),
                display_name: "测试".to_string(),
            }],
        };

        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].index, 3);

        let json = serde_json::to_vec(&data).unwrap();
        let parsed: IkunBoxDragData = serde_json::from_slice(&json).unwrap();
        assert_eq!(parsed.items[0].file_path, "test.lnk");
    }

    #[test]
    fn test_global_alloc_constants() {
        assert_eq!(GMEM_MOVEABLE, 0x0002);
        assert_eq!(GMEM_ZEROINIT, 0x0040);
        assert_eq!(GHND, 0x0042);
    }
}
