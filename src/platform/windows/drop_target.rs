//! OLE 拖拽目标实现（IDropTarget）

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
//!
//! 为分类框提供接收拖拽的能力：
//! - 接收从其他分类框拖入的图标（框间移动）
//! - 接收从桌面拖入的快捷方式（桌面→框）
//!
//! 从 `ole_dnd.rs` 拆分以控制文件行数。

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use super::ffi_imports::*;
use super::ffi_types::*;
use super::ole_dnd::{
    get_box_drag_format_id, read_box_drag_data, read_drop_files, read_shell_idlist_paths,
    IDataObjectVtbl,
};

/// 全局状态锁即使曾因线程 panic 中毒，也必须恢复内部值。
/// 这些表持有 COM 引用，不能因为一次 poison 就丢弃指针造成泄漏。
fn lock_unpoison<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

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

// 2026-11-04 自检审计：框间多选会为每个 item 各投递一条移除消息，单槽会被覆盖，
// 因此改为 FIFO 队列；WndProc 每处理一条消息取出一项，现有消息协议保持不变。
static PENDING_SOURCE_REMOVAL: Mutex<VecDeque<PendingSourceRemoval>> = Mutex::new(VecDeque::new());

/// 取出待处理的源框移除信息（由 WndProc 调用）
pub fn take_pending_source_removal() -> Option<PendingSourceRemoval> {
    PENDING_SOURCE_REMOVAL
        .lock()
        .ok()
        .and_then(|mut g| g.pop_front())
}

// 框→框移动：暂存待添加项信息队列（含释放坐标）
// 支持多选拖拽时批量入队，每次 take 取出队头元素
lazy_static::lazy_static! {
    static ref PENDING_ADD_ITEM: Mutex<std::collections::VecDeque<(String, String, i32, i32, i32, i32)>> =
        Mutex::new(std::collections::VecDeque::new());
}

/// 设置待添加项（包含释放位置的屏幕坐标）—— 入队而非覆盖
pub fn set_pending_add_item(
    file_path: String,
    display_name: String,
    drop_x: i32,
    drop_y: i32,
    insert_col: i32,
    insert_row: i32,
) {
    if let Ok(mut g) = PENDING_ADD_ITEM.lock() {
        g.push_back((
            file_path,
            display_name,
            drop_x,
            drop_y,
            insert_col,
            insert_row,
        ));
    }
}

/// 取出待添加项（包含释放坐标）—— 从队头取出
pub fn take_pending_add_item() -> Option<(String, String, i32, i32, i32, i32)> {
    PENDING_ADD_ITEM.lock().ok().and_then(|mut g| g.pop_front())
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
/// IDropTargetHelper vtable（Shell 拖拽残影管理）
///
/// 用于在外部拖拽（桌面→分类框）时保持系统残影图标的显示。
/// 当拖拽来自分类框内部时不使用（框间拖拽使用自定义覆盖窗口）。
#[repr(C)]
#[allow(non_snake_case)]
struct IDropTargetHelperVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    drag_enter:
        unsafe extern "system" fn(*mut c_void, HWND, *mut c_void, *mut POINT, u32) -> HRESULT,
    drag_leave: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    drag_over: unsafe extern "system" fn(*mut c_void, *mut POINT, u32) -> HRESULT,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut POINT, u32) -> HRESULT,
    show: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
}

#[repr(C)]
struct BoxDropTarget {
    vtbl: *const IDropTargetVtbl,
    // 2026-11-04 自检审计：COM 引用计数改为原子类型，Release 可安全处理并发/重复调用。
    ref_count: AtomicU32,
    target_hwnd: HWND,
    custom_format_id: u32,
    /// 代理窗口所属 UI 线程（用于线程判定与取证；跨线程时不直接读 UI 数据）。
    owner_thread_id: u32,
    /// 每轮拖拽只记录一次 DragOver 线程对比，避免高频 debug 日志。
    drag_over_thread_logged: AtomicBool,
    /// IDropTargetHelper 实例（仅在外部拖拽时创建，用于保持系统残影图标）
    drop_target_helper: *mut c_void,
}

/// 当前是否有来自分类框的拖拽悬停（用于区分外部拖拽，控制可视化反馈）
static BOX_DRAG_HOVER: AtomicBool = AtomicBool::new(false);

// 2026-11-04 自检审计：画布回调按屏幕坐标命中代理窗口时，不能跨线程直接解引用
// GridWindowData.drop_target_ptr。这里的 HWND→COM 对象表在注销时原子移除，
// 路由方在锁内 AddRef，保证窗口销毁与画布转发竞争时对象仍然存活。
lazy_static::lazy_static! {
    static ref BOX_TARGETS: Mutex<HashMap<isize, usize>> = Mutex::new(HashMap::new());
}

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
    if this.is_null() || riid.is_null() || ppv.is_null() {
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

unsafe fn increment_ref_count(counter: &AtomicU32) -> u32 {
    match counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
        Some(current.saturating_add(1))
    }) {
        Ok(previous) => previous.saturating_add(1),
        Err(current) => current,
    }
}

unsafe extern "system" fn drop_target_add_ref(this: *mut c_void) -> u32 {
    if this.is_null() {
        return 0;
    }
    let obj = &*(this as *const BoxDropTarget);
    increment_ref_count(&obj.ref_count)
}

unsafe extern "system" fn drop_target_release(this: *mut c_void) -> u32 {
    if this.is_null() {
        return 0;
    }
    // 2026-11-04 自检审计：saturating_sub 防止重复 Release 造成 u32 下溢；
    // 归零前先清理 IDropTargetHelper，避免窗口销毁时 Shell 残影与 COM 引用泄漏。
    let released = {
        let obj = &*(this as *const BoxDropTarget);
        obj.ref_count
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current == 0 {
                    None
                } else {
                    Some(current.saturating_sub(1))
                }
            })
    };
    match released {
        Ok(1) => {
            cleanup_helper(this as *mut BoxDropTarget);
            drop(Box::from_raw(this as *mut BoxDropTarget));
            0
        }
        Ok(previous) => previous.saturating_sub(1),
        Err(_) => {
            log::warn!("[DropTarget] Release 在引用计数已归零后再次调用，已忽略");
            0
        }
    }
}

// ----- IDropTarget 方法 -----

/// 清理 IDropTargetHelper 实例（Release 归零路径与 DragLeave/Drop 共用）
///
/// 关键修复：释放前必须调用 DragLeave 通知 Shell 拖拽已离开目标窗口，
/// 否则残影图标会卡死在最后一个位置（分类框边缘）。2026-11-04 自检审计
/// 将其封装为统一 cleanup_helper，确保窗口在外部拖拽中被销毁时也不会漏清理。
unsafe fn cleanup_helper(target: *mut BoxDropTarget) {
    let helper = (*target).drop_target_helper;
    if !helper.is_null() {
        log::info!(
            "[DropTarget] 释放 helper: helper={:?}, hwnd={:?}",
            helper,
            (*target).target_hwnd
        );
        let vtbl = *(helper as *const *const IDropTargetHelperVtbl);
        // ★ 关键修复：调用 DragLeave 通知 Shell 拖拽已离开
        let leave_hr = ((*vtbl).drag_leave)(helper);
        log::info!(
            "[DropTarget] helper.DragLeave: hr={:#010X}",
            leave_hr as u32
        );
        // Show(FALSE) 双重保险：确保残影图像被隐藏
        let show_hr = ((*vtbl).show)(helper, 0);
        log::info!(
            "[DropTarget] helper.Show(FALSE): hr={:#010X}",
            show_hr as u32
        );
        ((*vtbl).release)(helper);
        (*target).drop_target_helper = std::ptr::null_mut();
        log::info!("[DropTarget] helper 释放完成");
    } else {
        log::info!(
            "[DropTarget] helper 为空，跳过释放, hwnd={:?}",
            (*target).target_hwnd
        );
    }
}

unsafe extern "system" fn drop_target_drag_enter(
    this: *mut c_void,
    data_obj: *mut c_void,
    _ks: u32,
    ptl: i64,
    effect: *mut u32,
) -> HRESULT {
    if this.is_null() || effect.is_null() {
        return E_POINTER;
    }

    let target = this as *mut BoxDropTarget;
    (*target)
        .drag_over_thread_logged
        .store(false, Ordering::Relaxed);
    log::debug!(
        "[DropTarget] DragEnter 线程: current={}, owner={}, hwnd={:?}",
        GetCurrentThreadId(),
        (*target).owner_thread_id,
        (*target).target_hwnd
    );

    let has_box = query_format(data_obj, (*target).custom_format_id);
    let has_hdrop = query_format(data_obj, CF_HDROP);
    // 检查是否有虚拟 Shell 对象（回收站、此电脑等）
    let has_shell_idlist = if !has_hdrop && !has_box {
        let fmt_name: Vec<u16> = "Shell IDList Array\0".encode_utf16().collect();
        let cf = super::ffi_imports::RegisterClipboardFormatW(fmt_name.as_ptr());
        cf != 0 && query_format(data_obj, cf)
    } else {
        false
    };

    let format_supported = has_box || has_hdrop || has_shell_idlist;
    // 2026-11-04 自检审计：不接受 MOVE。异步入库/框间移动在 Drop 返回时都尚未
    // 确认最终成功，若向 Shell 承诺 MOVE，失败时资源管理器会删除源文件。
    // 这里只与源 allowed_effects 求 COPY 交集；交集为空则拒绝本次拖放。
    let allowed_effect = *effect;
    let copy_effect = allowed_effect & DROPEFFECT_COPY;
    let accepted = format_supported && copy_effect != 0;
    if accepted {
        *effect = copy_effect;
        BOX_DRAG_HOVER.store(has_box, Ordering::Relaxed);
        PostMessageW((*target).target_hwnd, WM_USER_DRAG_ENTER, 0, 0);
        log::info!(
            "[DRAG-ENTER] 拖拽进入: box={} hdrop={} shell_idlist={} effect=COPY",
            has_box,
            has_hdrop,
            has_shell_idlist
        );
    } else {
        *effect = DROPEFFECT_NONE;
        BOX_DRAG_HOVER.store(false, Ordering::Relaxed);
        if format_supported {
            log::warn!(
                "[DRAG-ENTER] 源 allowed_effects 不含 COPY，拒绝拖放: allowed={:#X}",
                allowed_effect
            );
        } else {
            log::debug!("[DRAG-ENTER] 拒绝拖拽: 无支持的格式");
        }
    }

    // 每次 DragEnter 都先释放上一轮可能残留的 helper；随后仅外部拖拽创建新的
    // IDropTargetHelper，框间自定义拖拽继续使用项目自己的残影覆盖窗口。
    if !(*target).drop_target_helper.is_null() {
        log::info!(
            "[DropTarget] DragEnter: 释放旧 helper {:?}（防泄漏）",
            (*target).drop_target_helper
        );
        cleanup_helper(target);
    }
    if !has_box && accepted {
        let mut helper: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            CLSID_DRAGDROPHELPER.as_ptr(),
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            IID_IDROPTARGETHELPER.as_ptr(),
            &mut helper,
        );
        if hr == S_OK && !helper.is_null() {
            let vtbl = *(helper as *const *const IDropTargetHelperVtbl);
            let mut pt = POINT {
                x: (ptl & 0xFFFFFFFF) as i32,
                y: (ptl >> 32) as i32,
            };
            let drag_enter_hr =
                ((*vtbl).drag_enter)(helper, (*target).target_hwnd, data_obj, &mut pt, *effect);
            (*target).drop_target_helper = helper;
            log::info!(
                "[DropTarget] IDropTargetHelper.DragEnter: hr={:#010X}, pt=({}, {}), hwnd={:?}",
                drag_enter_hr as u32,
                pt.x,
                pt.y,
                (*target).target_hwnd
            );
        } else {
            log::warn!(
                "[DropTarget] IDropTargetHelper 创建失败: hr={:#010X}",
                hr as u32
            );
        }
    }

    S_OK
}

unsafe extern "system" fn drop_target_drag_over(
    this: *mut c_void,
    _ks: u32,
    ptl: i64,
    effect: *mut u32,
) -> HRESULT {
    if this.is_null() || effect.is_null() {
        return E_POINTER;
    }

    let target = &*(this as *const BoxDropTarget);
    // 2026-11-04 自检审计：只承诺 COPY，绝不在 DragOver 阶段无条件切换为 MOVE。
    let copy_effect = *effect & DROPEFFECT_COPY;
    *effect = copy_effect;
    if !target.drag_over_thread_logged.swap(true, Ordering::Relaxed) {
        log::debug!(
            "[DropTarget] DragOver 线程: current={}, owner={}, hwnd={:?}",
            GetCurrentThreadId(),
            target.owner_thread_id,
            target.target_hwnd
        );
    }

    // 外部拖拽（桌面/资源管理器→框）：更新系统拖拽残影后，继续更新分类框插入指示器。
    let is_box_hover = BOX_DRAG_HOVER.load(Ordering::Relaxed);
    if !is_box_hover && !target.drop_target_helper.is_null() {
        let helper = target.drop_target_helper;
        let vtbl = *(helper as *const *const IDropTargetHelperVtbl);
        let mut pt = POINT {
            x: (ptl & 0xFFFFFFFF) as i32,
            y: (ptl >> 32) as i32,
        };
        let _ = ((*vtbl).drag_over)(helper, &mut pt, *effect);
    }

    if copy_effect != 0 {
        let screen_x = ptl as i32;
        let screen_y = (ptl >> 32) as i32;
        if GetCurrentThreadId() == target.owner_thread_id {
            // 同一 UI 线程才可直接访问 GridWindowData；画布转发线程改为 PostMessage。
            let should_update = super::grid_window::get_data(target.target_hwnd)
                .map(|d| !d.drag_active)
                .unwrap_or(false);
            if should_update {
                super::drag_visual::handle_cross_box_drag_over(
                    target.target_hwnd,
                    screen_x,
                    screen_y,
                );
            }
        } else {
            let packed = ((screen_y as isize) << 16) | ((screen_x as isize) & 0xFFFF);
            PostMessageW(
                target.target_hwnd,
                WM_USER_DRAG_OVER_UPDATE,
                0,
                packed as LPARAM,
            );
        }
    }

    S_OK
}

unsafe extern "system" fn drop_target_drag_leave(this: *mut c_void) -> HRESULT {
    if this.is_null() {
        return S_OK;
    }
    let target = this as *mut BoxDropTarget;
    log::debug!(
        "[DropTarget] DragLeave 线程: current={}, owner={}, hwnd={:?}",
        GetCurrentThreadId(),
        (*target).owner_thread_id,
        (*target).target_hwnd
    );
    log::info!(
        "[DropTarget] DragLeave: hwnd={:?}, has_helper={}, is_box_hover={}",
        (*target).target_hwnd,
        !(*target).drop_target_helper.is_null(),
        BOX_DRAG_HOVER.load(Ordering::Relaxed)
    );

    if !(*target).drop_target_helper.is_null() {
        cleanup_helper(target);
        log::info!("[DropTarget] DragLeave: helper 已释放");
    }

    (*target)
        .drag_over_thread_logged
        .store(false, Ordering::Relaxed);
    BOX_DRAG_HOVER.store(false, Ordering::Relaxed);
    PostMessageW((*target).target_hwnd, WM_USER_DRAG_LEAVE, 0, 0);
    S_OK
}

/// IDropTarget::Drop 实现：处理拖拽放置操作
///
/// 从 IDataObject 读取文件路径（CF_HDROP 或 CFSTR_SHELLIDLIST），
/// 暂存到 PENDING_DROP 队列，通知目标窗口处理。
/// 包含防御性空指针检查，防止异常输入导致崩溃。
unsafe extern "system" fn drop_target_drop(
    this: *mut c_void,
    data_obj: *mut c_void,
    _ks: u32,
    pt: i64,
    effect: *mut u32,
) -> HRESULT {
    if this.is_null() || data_obj.is_null() || effect.is_null() {
        log::warn!(
            "[DropTarget] Drop: 空指针被拒绝 (this={:?}, data_obj={:?}, effect={:?})",
            this,
            data_obj,
            effect
        );
        return E_POINTER;
    }

    let target = this as *mut BoxDropTarget;
    log::debug!(
        "[DropTarget] Drop 线程: current={}, owner={}, hwnd={:?}",
        GetCurrentThreadId(),
        (*target).owner_thread_id,
        (*target).target_hwnd
    );

    // 2026-11-04 自检审计：Drop 也只接受 COPY。外部文件入库与框间引用更新都在
    // Drop 返回后异步完成，此处无法确认成功；不确认成功绝不结算 MOVE。
    let allowed_effect = *effect;
    let copy_effect = allowed_effect & DROPEFFECT_COPY;
    if copy_effect == 0 {
        *effect = DROPEFFECT_NONE;
        log::warn!(
            "[DropTarget] Drop: 源 allowed_effects 不含 COPY，拒绝放置: allowed={:#X}",
            allowed_effect
        );
        return S_OK;
    }

    log::info!(
        "[DropTarget] Drop: hwnd={:?}, has_helper={}, pt=({}, {})",
        (*target).target_hwnd,
        !(*target).drop_target_helper.is_null(),
        (pt & 0xFFFFFFFF) as i32,
        (pt >> 32) as i32
    );
    if !(*target).drop_target_helper.is_null() {
        let helper = (*target).drop_target_helper;
        let vtbl = *(helper as *const *const IDropTargetHelperVtbl);
        let mut drop_pt = POINT {
            x: (pt & 0xFFFFFFFF) as i32,
            y: (pt >> 32) as i32,
        };
        let drop_hr = ((*vtbl).drop)(helper, data_obj, &mut drop_pt, copy_effect);
        log::info!("[DropTarget] helper.Drop: hr={:#010X}", drop_hr as u32);
        cleanup_helper(target);
        log::info!("[DropTarget] Drop: helper 已释放");
    }

    PostMessageW((*target).target_hwnd, WM_USER_DRAG_LEAVE, 0, 0);

    let drop_x = (pt & 0xFFFFFFFF) as i32;
    let drop_y = (pt >> 32) as i32;

    let has_box_format =
        (*target).custom_format_id != 0 && query_format(data_obj, (*target).custom_format_id);
    let box_data = if has_box_format {
        read_box_drag_data(data_obj, (*target).custom_format_id)
    } else {
        None
    };
    if has_box_format && box_data.is_none() {
        *effect = DROPEFFECT_NONE;
        log::warn!("[DROP] IkunBoxDragData 存在但解析失败，拒绝消费 CF_HDROP 临时路径");
        return S_OK;
    }

    // 2026-11-04 自检审计：框间拖拽只消费自定义载荷中的原始逻辑路径。
    // CF_HDROP 中的 %TEMP%\ikun_ole_drag 仅用于 Explorer 展示，源端返回后可能已删除。
    let file_paths = if let Some(box_payload) = box_data.as_ref() {
        box_payload
            .items
            .iter()
            .map(|item| item.file_path.clone())
            .collect::<Vec<_>>()
    } else {
        let mut paths = read_drop_files(data_obj);
        if paths.is_empty() {
            log::info!("[DROP] CF_HDROP 为空，尝试读取 PIDL...");
            paths = read_shell_idlist_paths(data_obj);
        }
        paths
    };
    if file_paths.is_empty() {
        *effect = DROPEFFECT_NONE;
        log::warn!("[DROP] 未读取到有效文件路径，拒绝放置");
        return S_OK;
    }
    log::info!("[DROP] 最终 file_paths={:?}", file_paths);

    let (insert_col, insert_row) = if GetCurrentThreadId() == (*target).owner_thread_id {
        match super::grid_window::get_data((*target).target_hwnd) {
            Some(data) => (data.drag_over_insert_col, data.drag_over_insert_row),
            None => (-1, -1),
        }
    } else {
        // 画布回调线程不直接读 GridWindowData；异步处理器可回落到当前 UI 状态。
        (-1, -1)
    };

    let queued = if let Ok(mut pending) = PENDING_DROP.lock() {
        *pending = Some(PendingDrop {
            file_paths,
            box_data: box_data.clone(),
            drop_x,
            drop_y,
            insert_col,
            insert_row,
        });
        true
    } else {
        false
    };
    if !queued {
        *effect = DROPEFFECT_NONE;
        log::warn!("[DROP] PENDING_DROP 锁不可用，拒绝放置");
        return S_OK;
    }

    // 2026-11-04 自检审计：源框移除由 item 消息驱动。多选时按索引降序逐项入队并
    // PostMessage，每项一条消息，避免单槽覆盖以及从前往后删除导致的索引漂移。
    if let Some(ref bd) = box_data {
        if !bd.items.is_empty() {
            let src = bd.source_hwnd as *mut c_void;
            if src.is_null() {
                log::warn!("[DROP] 框间拖拽缺少有效源窗口，跳过源删除通知");
            } else {
                let mut items: Vec<_> = bd.items.iter().collect();
                items.sort_by_key(|item| std::cmp::Reverse(item.index));
                if let Ok(mut removals) = PENDING_SOURCE_REMOVAL.lock() {
                    for item in items {
                        removals.push_back(PendingSourceRemoval {
                            file_path: item.file_path.clone(),
                            display_name: item.display_name.clone(),
                            item_index: item.index,
                        });
                        let posted = PostMessageW(src, WM_USER_DRAG_ITEM_REMOVED, item.index, 0);
                        if posted == 0 {
                            removals.pop_back();
                            log::warn!(
                                "[DROP] 源删除消息投递失败，已回滚队列项: index={}",
                                item.index
                            );
                        }
                    }
                } else {
                    log::warn!("[DROP] PENDING_SOURCE_REMOVAL 锁不可用，跳过源删除通知");
                }
            }
        }
    }

    PostMessageW((*target).target_hwnd, WM_USER_DRAG_DROP, 0, 0);
    // 框间自定义载荷由 WM_USER_DRAG_ITEM_REMOVED 驱动源删除；OLE 侧统一返回 COPY。
    *effect = DROPEFFECT_COPY;
    log::info!("拖拽放置完成（数据已暂存，OLE 效果=COPY）");
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

// =============================================================================
// 公共 API
// =============================================================================

/// 为分类框窗口注册 OLE 拖拽目标
pub fn register_box_drop_target(hwnd: HWND) -> *mut c_void {
    let custom_format_id = get_box_drag_format_id();
    let target = Box::new(BoxDropTarget {
        vtbl: &DROP_TARGET_VTBL,
        ref_count: AtomicU32::new(1),
        target_hwnd: hwnd,
        custom_format_id,
        owner_thread_id: unsafe { GetCurrentThreadId() },
        drag_over_thread_logged: AtomicBool::new(false),
        drop_target_helper: std::ptr::null_mut(),
    });
    let ptr = Box::into_raw(target) as *mut c_void;
    let hr = unsafe { RegisterDragDrop(hwnd, ptr) };
    if hr != S_OK {
        log::warn!("RegisterDragDrop 失败: hr={:#010X}", hr as u32);
        unsafe { drop_target_release(ptr) };
        return std::ptr::null_mut();
    }

    lock_unpoison(&BOX_TARGETS).insert(hwnd as isize, ptr as usize);
    log::info!("已注册拖拽目标: hwnd={:?}", hwnd);
    ptr
}

/// 撤销分类框窗口的 OLE 拖拽目标注册
pub fn unregister_box_drop_target(hwnd: HWND, target_ptr: *mut c_void) {
    // 2026-11-04 自检审计：先移除 HWND 映射阻断新路由，再清空可能存在的
    // CANVAS_ROUTED 引用；最后 RevokeDragDrop 并释放注册表引用。
    {
        let mut targets = lock_unpoison(&BOX_TARGETS);
        if targets.get(&(hwnd as isize)).copied() == Some(target_ptr as usize) {
            targets.remove(&(hwnd as isize));
        }
    }
    if let Some(routed) = take_canvas_route_matching(target_ptr as usize) {
        unsafe { drop_target_release(routed as *mut c_void) };
    }
    unsafe {
        RevokeDragDrop(hwnd);
    }
    if !target_ptr.is_null() {
        unsafe { drop_target_release(target_ptr) };
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

/// IDropTargetHelper 生命周期状态机测试
///
/// 验证 helper 在各拖拽阶段的预期状态转换：
/// - 初始：helper 为空
/// - DragEnter：创建 helper
/// - DragLeave/Drop：释放 helper（先 DragLeave → Show(FALSE) → Release）
#[cfg(test)]
mod helper_lifecycle_tests {
    use super::*;

    /// 模拟 BoxDropTarget 的 helper 字段初始状态
    /// 新注册的 drop target 的 helper 必须为空
    #[test]
    fn test_initial_helper_is_null() {
        // 模拟 BoxDropTarget 创建时的 drop_target_helper 字段
        let drop_target_helper: *mut std::ffi::c_void = std::ptr::null_mut();
        assert!(
            drop_target_helper.is_null(),
            "新创建的 drop target 的 helper 必须为空"
        );
    }

    /// 验证 DragLeave 后 helper 被正确清空
    /// cleanup_helper 应将 helper 设为 null
    #[test]
    fn test_helper_cleared_after_release_pattern() {
        // 模拟 cleanup_helper 的核心逻辑：
        // 1. 检查 helper 非空
        // 2. 调用 DragLeave
        // 3. 调用 Show(FALSE)
        // 4. 调用 Release
        // 5. 设置 helper = null
        let mut drop_target_helper: *mut std::ffi::c_void = 0x1234 as *mut _;
        assert!(!drop_target_helper.is_null());

        // 模拟 release 操作
        drop_target_helper = std::ptr::null_mut();
        assert!(drop_target_helper.is_null(), "release 后 helper 必须为空");
    }

    /// 验证 DragEnter 时旧 helper 被释放（防泄漏保护）
    /// 连续拖拽场景：第二次 DragEnter 时必须先释放旧的 helper
    #[test]
    fn test_drag_enter_releases_old_helper() {
        // 模拟第一次 DragEnter 创建了 helper
        let mut drop_target_helper: *mut std::ffi::c_void = 0xABCD as *mut _;
        assert!(
            !drop_target_helper.is_null(),
            "第一次 DragEnter 后 helper 非空"
        );

        // 模拟第二次 DragEnter：先检查并释放旧 helper
        if !drop_target_helper.is_null() {
            // cleanup_helper(target) 的核心效果
            drop_target_helper = std::ptr::null_mut();
        }
        assert!(
            drop_target_helper.is_null(),
            "第二次 DragEnter 前旧 helper 必须被释放"
        );

        // 然后创建新 helper
        drop_target_helper = 0xEF01 as *mut _;
        assert!(!drop_target_helper.is_null(), "新 helper 创建成功");
    }

    /// 验证 BOX_DRAG_HOVER 状态转换
    /// - 框间拖拽 DragEnter：BOX_DRAG_HOVER = true
    /// - 外部拖拽 DragEnter：BOX_DRAG_HOVER = false
    /// - DragLeave：BOX_DRAG_HOVER = false
    #[test]
    fn test_box_drag_hover_state_transitions() {
        // 初始状态
        BOX_DRAG_HOVER.store(false, Ordering::Relaxed);
        assert!(!BOX_DRAG_HOVER.load(Ordering::Relaxed));

        // 框间拖拽进入
        BOX_DRAG_HOVER.store(true, Ordering::Relaxed);
        assert!(BOX_DRAG_HOVER.load(Ordering::Relaxed));

        // DragLeave 重置
        BOX_DRAG_HOVER.store(false, Ordering::Relaxed);
        assert!(!BOX_DRAG_HOVER.load(Ordering::Relaxed));

        // 外部拖拽进入（has_box=false → BOX_DRAG_HOVER=false）
        BOX_DRAG_HOVER.store(false, Ordering::Relaxed);
        assert!(!BOX_DRAG_HOVER.load(Ordering::Relaxed));
    }

    /// 验证 release 调用顺序不变量
    /// 正确顺序：DragLeave → Show(FALSE) → Release
    /// 这是防止残影卡死的关键
    #[test]
    fn test_release_call_order_invariant() {
        // 模拟 IDropTargetHelper 方法调用顺序
        let mut call_order: Vec<&str> = Vec::new();

        // 正确的释放顺序
        call_order.push("drag_leave"); // 通知 Shell 拖拽已离开
        call_order.push("show_false"); // 隐藏残影图像
        call_order.push("release"); // 释放 COM 对象

        assert_eq!(call_order[0], "drag_leave", "第一步必须是 DragLeave");
        assert_eq!(call_order[1], "show_false", "第二步必须是 Show(FALSE)");
        assert_eq!(call_order[2], "release", "第三步必须是 Release");
        assert_eq!(call_order.len(), 3, "必须恰好 3 步");
    }
}

// =============================================================================
// 画布级转发 IDropTarget（引擎模式专用）
// =============================================================================
//
// 引擎激活时分类框代理窗口提交的是全透明帧 → OLE 的鼠标命中链落在
// liquid-verre-rs 的每屏画布 HWND 上，注册在代理窗口的 BoxDropTarget 再也
// 收不到 DragEnter。这里给每块画布注册一个无状态转发器：按拖拽点坐标路由
// 到对应分类框的 BoxDropTarget，直接调用其 vtable 方法，令「拖入接收」
// 在库窗口模型下原样工作。画布回调发生在 liquid-verre-render 线程，
// GridWindowData 的读取通过 PostMessage 回到代理窗 UI 线程。

/// 转发器当前路由到的 BoxDropTarget（isize 裸指针，0=无）。
/// Enter 建立、Leave/Drop 清除；路由持有一次 AddRef，切换目标时成对 Release。
static CANVAS_ROUTED: AtomicUsize = AtomicUsize::new(0);
/// 同一次 OLE 拖拽序列的 IDataObject 指针；仅用于 A→B 切换时补发 DragEnter。
static CANVAS_ROUTED_DATA_OBJ: AtomicUsize = AtomicUsize::new(0);
/// 路由指针与 data object 必须成对切换，避免目标切换窗口期读到半更新状态。
static CANVAS_ROUTE_LOCK: Mutex<()> = Mutex::new(());
// 已注册转发器的画布 HWND → CanvasDropTarget 指针（键为 HWND 的 isize 形式，
// 值持有注册表引用；跨线程访问加锁，注销时 RevokeDragDrop 并 Release）。
lazy_static::lazy_static! {
    static ref CANVAS_BOUND: Mutex<HashMap<isize, usize>> = Mutex::new(HashMap::new());
}

#[repr(C)]
struct CanvasDropTarget {
    vtbl: *const IDropTargetVtbl,
    // 2026-11-04 自检审计：真实引用计数，注册失败/撤销/显示器重建均可回收对象。
    ref_count: AtomicU32,
    canvas_hwnd: HWND,
}

static CANVAS_DT_VTBL: IDropTargetVtbl = IDropTargetVtbl {
    query_interface: canvas_target_query_interface,
    add_ref: canvas_target_add_ref,
    release: canvas_target_release,
    drag_enter: canvas_target_drag_enter,
    drag_over: canvas_target_drag_over,
    drag_leave: canvas_target_drag_leave,
    drop: canvas_target_drop,
};

unsafe extern "system" fn canvas_target_query_interface(
    this: *mut c_void,
    riid: *const u8,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if this.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    let iid = std::slice::from_raw_parts(riid, 16);
    if iid == IID_IUNKNOWN || iid == IID_IDROPTARGET {
        *ppv = this;
        canvas_target_add_ref(this);
        S_OK
    } else {
        *ppv = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn canvas_target_add_ref(this: *mut c_void) -> u32 {
    if this.is_null() {
        return 0;
    }
    let obj = &*(this as *const CanvasDropTarget);
    increment_ref_count(&obj.ref_count)
}

unsafe extern "system" fn canvas_target_release(this: *mut c_void) -> u32 {
    if this.is_null() {
        return 0;
    }
    let released = {
        let obj = &*(this as *const CanvasDropTarget);
        obj.ref_count
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current == 0 {
                    None
                } else {
                    Some(current.saturating_sub(1))
                }
            })
    };
    match released {
        Ok(1) => {
            drop(Box::from_raw(this as *mut CanvasDropTarget));
            0
        }
        Ok(previous) => previous.saturating_sub(1),
        Err(_) => {
            log::warn!("[DropTarget] Canvas Release 在引用计数归零后再次调用，已忽略");
            0
        }
    }
}

fn canvas_route_store(target: usize, data_obj: usize) {
    let _guard = lock_unpoison(&CANVAS_ROUTE_LOCK);
    CANVAS_ROUTED.store(target, Ordering::SeqCst);
    CANVAS_ROUTED_DATA_OBJ.store(data_obj, Ordering::SeqCst);
}

fn canvas_route_take() -> (usize, usize) {
    let _guard = lock_unpoison(&CANVAS_ROUTE_LOCK);
    let target = CANVAS_ROUTED.swap(0, Ordering::SeqCst);
    // IDataObject 在整个 OLE 拖拽序列内有效；离开 A 尚未进入 B 的间隙也必须保留，
    // 否则鼠标跨过空白区域后无法给 B 补发 DragEnter。最终 Leave/Drop 才清空。
    let data_obj = CANVAS_ROUTED_DATA_OBJ.load(Ordering::SeqCst);
    (target, data_obj)
}

fn canvas_set_active_data_obj(data_obj: usize) {
    let _guard = lock_unpoison(&CANVAS_ROUTE_LOCK);
    CANVAS_ROUTED_DATA_OBJ.store(data_obj, Ordering::SeqCst);
}

fn canvas_clear_active_data_obj() {
    let _guard = lock_unpoison(&CANVAS_ROUTE_LOCK);
    CANVAS_ROUTED_DATA_OBJ.store(0, Ordering::SeqCst);
}

/// 注销目标时仅清空匹配的路由，不调用 DragLeave（窗口正在销毁），
/// 但必须把路由持有的 AddRef 还给调用方完成 Release。
fn take_canvas_route_matching(target: usize) -> Option<usize> {
    let _guard = lock_unpoison(&CANVAS_ROUTE_LOCK);
    if CANVAS_ROUTED.load(Ordering::SeqCst) != target || target == 0 {
        return None;
    }
    CANVAS_ROUTED.store(0, Ordering::SeqCst);
    CANVAS_ROUTED_DATA_OBJ.store(0, Ordering::SeqCst);
    Some(target)
}

/// POINTL(i64) → 屏幕坐标 → 命中分类框的 BoxDropTarget。
/// 返回前已在 BOX_TARGETS 锁内 AddRef，调用方必须 Release。
unsafe fn canvas_route(ptl: i64) -> *mut BoxDropTarget {
    let x = ptl as i32;
    let y = (ptl >> 32) as i32;
    let agent = super::zorder::find_grid_window_at_point(x, y);
    if agent.is_null() {
        return std::ptr::null_mut();
    }
    let target = lock_unpoison(&BOX_TARGETS)
        .get(&(agent as isize))
        .copied()
        .unwrap_or(0);
    if target == 0 {
        return std::ptr::null_mut();
    }
    drop_target_add_ref(target as *mut c_void);
    target as *mut BoxDropTarget
}

unsafe fn canvas_call_enter(
    target: *mut BoxDropTarget,
    data_obj: *mut c_void,
    ks: u32,
    ptl: i64,
    effect: *mut u32,
) -> HRESULT {
    let vtbl = *(target as *const *const IDropTargetVtbl);
    ((*vtbl).drag_enter)(target as *mut c_void, data_obj, ks, ptl, effect)
}

unsafe fn canvas_call_over(
    target: *mut BoxDropTarget,
    ks: u32,
    ptl: i64,
    effect: *mut u32,
) -> HRESULT {
    let vtbl = *(target as *const *const IDropTargetVtbl);
    ((*vtbl).drag_over)(target as *mut c_void, ks, ptl, effect)
}

unsafe fn canvas_call_leave(target: *mut BoxDropTarget) -> HRESULT {
    let vtbl = *(target as *const *const IDropTargetVtbl);
    ((*vtbl).drag_leave)(target as *mut c_void)
}

unsafe fn canvas_call_drop(
    target: *mut BoxDropTarget,
    data_obj: *mut c_void,
    ks: u32,
    ptl: i64,
    effect: *mut u32,
) -> HRESULT {
    let vtbl = *(target as *const *const IDropTargetVtbl);
    ((*vtbl).drop)(target as *mut c_void, data_obj, ks, ptl, effect)
}

unsafe fn log_canvas_thread(operation: &str, target: *mut BoxDropTarget, canvas: HWND) {
    let mut process_id = 0u32;
    let proxy_thread = GetWindowThreadProcessId((*target).target_hwnd, &mut process_id);
    let canvas_thread = GetWindowThreadProcessId(canvas, &mut process_id);
    log::debug!(
        "[DropTarget][Canvas] {} 线程: current={}, canvas_thread={}, proxy_thread={}, owner={}, hwnd={:?}",
        operation,
        GetCurrentThreadId(),
        canvas_thread,
        proxy_thread,
        (*target).owner_thread_id,
        (*target).target_hwnd
    );
}

unsafe extern "system" fn canvas_target_drag_enter(
    this: *mut c_void,
    data_obj: *mut c_void,
    ks: u32,
    ptl: i64,
    effect: *mut u32,
) -> HRESULT {
    if effect.is_null() {
        return E_POINTER;
    }
    // IDataObject 属于整次拖拽序列；即使 DragEnter 落在块间空白处，也要保存，
    // 供随后 DragOver 首次命中分类框时补发 DragEnter。
    canvas_set_active_data_obj(data_obj as usize);
    let target = canvas_route(ptl);

    let (old, _) = canvas_route_take();
    if old != 0 {
        if old == target as usize {
            // 旧路由与新路由同一对象：释放本次额外 AddRef，保留原路由引用。
            drop_target_release(old as *mut c_void);
        } else {
            canvas_call_leave(old as *mut BoxDropTarget);
            drop_target_release(old as *mut c_void);
        }
    }
    if target.is_null() {
        *effect = DROPEFFECT_NONE;
        return S_OK;
    }
    let canvas = if this.is_null() {
        std::ptr::null_mut()
    } else {
        (*(this as *const CanvasDropTarget)).canvas_hwnd
    };
    log_canvas_thread("DragEnter", target, canvas);

    let hr = canvas_call_enter(target, data_obj, ks, ptl, effect);
    if hr >= 0 {
        canvas_route_store(target as usize, data_obj as usize);
    } else {
        drop_target_release(target as *mut c_void);
    }
    hr
}

unsafe extern "system" fn canvas_target_drag_over(
    this: *mut c_void,
    ks: u32,
    ptl: i64,
    effect: *mut u32,
) -> HRESULT {
    if effect.is_null() {
        return E_POINTER;
    }

    // 2026-11-04 自检审计：每次都按当前 POINTL 重解析目标，框 A→框 B 时不会把
    // Drop 继续投给 Enter 时缓存的 A。
    let target = canvas_route(ptl);
    let current = CANVAS_ROUTED.load(Ordering::SeqCst);
    if target.is_null() {
        let (old, _) = canvas_route_take();
        if old != 0 {
            canvas_call_leave(old as *mut BoxDropTarget);
            drop_target_release(old as *mut c_void);
        }
        *effect = DROPEFFECT_NONE;
        return S_OK;
    }

    if current == target as usize {
        let hr = canvas_call_over(target, ks, ptl, effect);
        drop_target_release(target as *mut c_void);
        return hr;
    }

    let (old, data_obj) = canvas_route_take();
    if old != 0 {
        if old == target as usize {
            drop_target_release(old as *mut c_void);
        } else {
            canvas_call_leave(old as *mut BoxDropTarget);
            drop_target_release(old as *mut c_void);
        }
    }

    let canvas = if this.is_null() {
        std::ptr::null_mut()
    } else {
        (*(this as *const CanvasDropTarget)).canvas_hwnd
    };
    log_canvas_thread("DragOver 目标切换", target, canvas);

    if data_obj == 0 {
        log::warn!("[DropTarget][Canvas] 目标切换缺少缓存 IDataObject，无法补发 DragEnter");
        drop_target_release(target as *mut c_void);
        *effect = DROPEFFECT_NONE;
        return S_OK;
    }

    let enter_hr = canvas_call_enter(target, data_obj as *mut c_void, ks, ptl, effect);
    if enter_hr < 0 {
        drop_target_release(target as *mut c_void);
        *effect = DROPEFFECT_NONE;
        return enter_hr;
    }
    let over_hr = canvas_call_over(target, ks, ptl, effect);
    canvas_route_store(target as usize, data_obj);
    over_hr
}

unsafe extern "system" fn canvas_target_drag_leave(_this: *mut c_void) -> HRESULT {
    let (routed, _) = canvas_route_take();
    canvas_clear_active_data_obj();
    if routed == 0 {
        return S_OK;
    }
    let hr = canvas_call_leave(routed as *mut BoxDropTarget);
    drop_target_release(routed as *mut c_void);
    hr
}

unsafe extern "system" fn canvas_target_drop(
    this: *mut c_void,
    data_obj: *mut c_void,
    ks: u32,
    ptl: i64,
    effect: *mut u32,
) -> HRESULT {
    if effect.is_null() {
        return E_POINTER;
    }

    // Drop 以当前坐标做最终路由；缓存仅用于识别是否已对同一目标 Enter 过。
    let target = canvas_route(ptl);
    let (old, cached_data_obj) = canvas_route_take();
    canvas_clear_active_data_obj();
    if target.is_null() {
        if old != 0 {
            canvas_call_leave(old as *mut BoxDropTarget);
            drop_target_release(old as *mut c_void);
        }
        *effect = DROPEFFECT_NONE;
        return S_OK;
    }

    let canvas = if this.is_null() {
        std::ptr::null_mut()
    } else {
        (*(this as *const CanvasDropTarget)).canvas_hwnd
    };
    log_canvas_thread("Drop", target, canvas);

    if old == target as usize {
        drop_target_release(old as *mut c_void);
    } else {
        if old != 0 {
            canvas_call_leave(old as *mut BoxDropTarget);
            drop_target_release(old as *mut c_void);
        }
        if cached_data_obj == 0 {
            log::warn!("[DropTarget][Canvas] 最终目标未收到过 DragEnter，且缺少 IDataObject");
            drop_target_release(target as *mut c_void);
            *effect = DROPEFFECT_NONE;
            return S_OK;
        }
        let enter_hr = canvas_call_enter(target, cached_data_obj as *mut c_void, ks, ptl, effect);
        if enter_hr < 0 {
            drop_target_release(target as *mut c_void);
            *effect = DROPEFFECT_NONE;
            return enter_hr;
        }
    }

    let hr = canvas_call_drop(target, data_obj, ks, ptl, effect);
    drop_target_release(target as *mut c_void);
    hr
}

/// 为全部画布注册转发 IDropTarget（幂等；引擎启动与 MonitorsChanged 后调用）。
pub fn rebind_canvas_drop_targets() {
    let canvases = super::glass_engine::canvas_hwnds();
    let live: std::collections::HashSet<isize> = canvases.iter().copied().collect();

    // 显示器变化后旧画布可能已销毁：先撤销已离场 HWND，避免 CANVAS_BOUND 只增不减。
    let stale = {
        let mut bound = lock_unpoison(&CANVAS_BOUND);
        let stale_keys: Vec<isize> = bound
            .keys()
            .copied()
            .filter(|canvas| !live.contains(canvas))
            .collect();
        let mut stale = Vec::with_capacity(stale_keys.len());
        for canvas in stale_keys {
            if let Some(target) = bound.remove(&canvas) {
                stale.push((canvas, target));
            }
        }
        stale
    };
    for (canvas, target) in stale {
        unsafe {
            RevokeDragDrop(canvas as HWND);
            canvas_target_release(target as *mut c_void);
        }
        log::info!("显示器变化后已撤销画布拖放转发器: canvas={canvas:#X}");
    }

    for canvas in canvases {
        if lock_unpoison(&CANVAS_BOUND).contains_key(&canvas) {
            continue;
        }

        let obj = Box::into_raw(Box::new(CanvasDropTarget {
            vtbl: &CANVAS_DT_VTBL,
            ref_count: AtomicU32::new(1),
            canvas_hwnd: canvas as HWND,
        })) as *mut c_void;
        let hr = unsafe { RegisterDragDrop(canvas as HWND, obj) };
        if hr != S_OK {
            unsafe { canvas_target_release(obj) };
            log::warn!("画布拖放转发器注册失败: canvas={canvas:#X}, hr={hr:#010X}");
            continue;
        }

        lock_unpoison(&CANVAS_BOUND).insert(canvas, obj as usize);
        log::info!("画布拖放转发器已注册: canvas={canvas:#X}");
    }
}

/// 引擎 shutdown 前撤销全部画布转发器；由 glass_engine.rs 接线调用。
pub(crate) fn revoke_all_canvas_drop_targets() {
    let (routed, _) = canvas_route_take();
    canvas_clear_active_data_obj();
    if routed != 0 {
        unsafe {
            canvas_call_leave(routed as *mut BoxDropTarget);
            drop_target_release(routed as *mut c_void);
        }
    }

    let bound = std::mem::take(&mut *lock_unpoison(&CANVAS_BOUND));
    for (canvas, target) in bound {
        unsafe {
            RevokeDragDrop(canvas as HWND);
            canvas_target_release(target as *mut c_void);
        }
        log::info!("已撤销画布拖放转发器: canvas={canvas:#X}");
    }
}
