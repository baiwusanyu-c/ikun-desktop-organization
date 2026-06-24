//! 桌面图标逐图标管理器
//!
//! 通过跨进程操作 explorer.exe 的 SysListView32 控件，
//! 实现单个桌面图标的隐藏（移到屏幕外）和显示（移回指定坐标）。
//!
//! # 技术原理
//!
//! 1. 通过 `FindWindowExW` 逐层查找获取 SysListView32 句柄
//! 2. 通过 `OpenProcess` 获取 explorer.exe 进程句柄
//! 3. 使用 `VirtualAllocEx` / `WriteProcessMemory` 在 explorer.exe 分配内存
//! 4. 通过 `SendMessage(LVM_*)` 跨进程读写图标信息和位置
//! 5. 维护 `name → index` 映射缓存，用于按文件名查找图标
//!
//! # 优雅降级
//!
//! 所有操作返回 `Result`，失败时记录警告日志但不阻塞应用。
//! 跨进程操作失败时，上层可退化为仅逻辑移动（不操作 SysListView32）。

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::ffi::c_void;

use super::ffi_imports::*;
use super::ffi_types::*;

// ===== 窗口类名常量（UTF-16）=====
static PROG_CLASS: &[u16] = &[80, 114, 111, 103, 109, 97, 109, 0]; // "Progman"
static WORKER_CLASS: &[u16] = &[87, 111, 114, 107, 101, 114, 87, 0]; // "WorkerW"
static DEFVIEW_CLASS: &[u16] = &[
    83, 72, 69, 76, 76, 68, 76, 76, 95, 68, 101, 102, 86, 105, 101, 119, 0,
]; // "SHELLDLL_DefView"
static LISTVIEW_CLASS: &[u16] = &[
    83, 121, 115, 76, 105, 115, 116, 86, 105, 101, 119, 51, 50, 0,
]; // "SysListView32"

/// 跨进程读取文本时的缓冲区大小（UTF-16 字符数）
const TEXT_BUFFER_CHARS: usize = 512;

/// 桌面图标逐图标管理器
///
/// 维护 SysListView32 句柄和 explorer.exe 进程句柄，
/// 提供按名称查找、隐藏、显示单个图标的能力。
pub struct DesktopIconManager {
    /// SysListView32 窗口句柄
    listview_hwnd: HWND,
    /// explorer.exe 进程句柄（拥有 SysListView32 的进程）
    explorer_process: HANDLE,
    /// 文件名 → ListView index 的缓存映射
    name_to_index: HashMap<String, i32>,
    /// 文件名 → 上次已知桌面坐标（用于恢复时定位）
    name_to_position: HashMap<String, (i32, i32)>,
}

// Safety: HWND 和 HANDLE 在同一线程使用，Send 安全
unsafe impl Send for DesktopIconManager {}

impl DesktopIconManager {
    /// 创建桌面图标管理器
    ///
    /// 查找 SysListView32、打开 explorer.exe 进程、构建索引缓存。
    /// 若 `find_desktop_listview()` 失败（如窗口已被隐藏），
    /// 则使用 `SHARED_LISTVIEW_HWND` 中缓存的 HWND（由 icon_hider 保存）。
    /// 失败时返回错误（上层应优雅降级）。
    pub fn new() -> Result<Self> {
        let listview_hwnd = find_desktop_listview().unwrap_or_else(|_| {
            // SysListView32 可能被 icon_hider 隐藏，使用缓存的 HWND
            let cached = super::ffi_types::SHARED_LISTVIEW_HWND
                .load(std::sync::atomic::Ordering::SeqCst);
            if cached != 0 {
                log::info!("使用缓存的 SysListView32 HWND: {:#X}", cached);
                cached as HWND
            } else {
                std::ptr::null_mut()
            }
        });

        if listview_hwnd.is_null() {
            anyhow::bail!("无法找到桌面 SysListView32 窗口（包括缓存）");
        }

        // 获取 SysListView32 所在进程 ID
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(listview_hwnd, &mut pid) };
        if pid == 0 {
            anyhow::bail!("无法获取 SysListView32 的进程 ID");
        }

        // 打开进程（需要 VM 读写权限用于跨进程 ListView 操作）
        let process = unsafe {
            OpenProcess(
                PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE,
                0,
                pid,
            )
        };
        if process.is_null() {
            anyhow::bail!("OpenProcess 失败（pid={}），可能需要管理员权限", pid);
        }

        let mut mgr = Self {
            listview_hwnd,
            explorer_process: process,
            name_to_index: HashMap::new(),
            name_to_position: HashMap::new(),
        };

        // 构建初始索引
        if let Err(e) = mgr.rebuild_index() {
            log::warn!("初始索引构建失败（非致命）: {}", e);
        }

        log::info!(
            "桌面图标管理器已初始化: {} 个图标已索引",
            mgr.name_to_index.len()
        );
        Ok(mgr)
    }

    /// 获取 SysListView32 中的图标总数
    pub fn get_icon_count(&self) -> i32 {
        unsafe { SendMessageW(self.listview_hwnd, LVM_GETITEMCOUNT, 0, 0) as i32 }
    }

    /// 按文件名查找图标的 ListView index
    ///
    /// 首先查缓存，缓存未命中时返回 None。
    /// 如果 explorer 重启导致 index 变化，需调用 `rebuild_index` 刷新。
    pub fn find_icon_by_name(&self, name: &str) -> Option<i32> {
        self.name_to_index.get(name).copied()
    }

    /// 隐藏单个图标（移到屏幕外）
    ///
    /// 将指定 index 的桌面图标移动到 `HIDDEN_ICON_X, HIDDEN_ICON_Y`，
    /// 使其在桌面上不可见但仍在 SysListView32 中存在。
    pub fn hide_icon(&self, index: i32) -> Result<()> {
        self.set_icon_position(index, HIDDEN_ICON_X, HIDDEN_ICON_Y)
            .context("隐藏图标失败")
    }

    /// 显示单个图标到指定桌面坐标
    ///
    /// 将之前隐藏的图标移回到指定的屏幕坐标位置。
    pub fn show_icon_at(&self, index: i32, x: i32, y: i32) -> Result<()> {
        self.set_icon_position(index, x, y)
            .context("显示图标失败")
    }

    /// 按名称隐藏图标
    ///
    /// 查找缓存中的 index 并隐藏。找不到时记录警告。
    pub fn hide_icon_by_name(&self, name: &str) -> Result<()> {
        match self.find_icon_by_name(name) {
            Some(index) => self.hide_icon(index),
            None => {
                log::warn!("hide_icon_by_name: 未找到图标 '{}', 可能 explorer 已重启", name);
                anyhow::bail!("未找到图标 '{}'", name)
            }
        }
    }

    /// 按名称显示图标到指定位置
    ///
    /// 查找缓存中的 index 并显示。找不到时记录警告。
    pub fn show_icon_by_name(&self, name: &str, x: i32, y: i32) -> Result<()> {
        match self.find_icon_by_name(name) {
            Some(index) => self.show_icon_at(index, x, y),
            None => {
                log::warn!("show_icon_by_name: 未找到图标 '{}', 可能 explorer 已重启", name);
                anyhow::bail!("未找到图标 '{}'", name)
            }
        }
    }

    /// 批量隐藏图标
    ///
    /// 逐个隐藏，单个失败不影响其他图标。返回成功隐藏的数量。
    pub fn hide_icons_by_names(&self, names: &[String]) -> usize {
        let mut success_count = 0;
        for name in names {
            match self.hide_icon_by_name(name) {
                Ok(()) => success_count += 1,
                Err(e) => log::warn!("批量隐藏: 跳过 '{}': {}", name, e),
            }
        }
        success_count
    }

    /// 批量恢复图标到桌面
    ///
    /// 使用缓存的上次已知坐标，无坐标则使用 (100, 100) 作为回退位置。
    /// 返回成功恢复的数量。
    pub fn show_icons_by_names(&self, names: &[String]) -> usize {
        let mut success_count = 0;
        for name in names {
            let (x, y) = self
                .name_to_position
                .get(name)
                .copied()
                .unwrap_or((100, 100));
            match self.show_icon_by_name(name, x, y) {
                Ok(()) => success_count += 1,
                Err(e) => log::warn!("批量恢复: 跳过 '{}': {}", name, e),
            }
        }
        success_count
    }

    /// 保存图标的当前桌面坐标（用于后续恢复）
    pub fn save_icon_position(&mut self, name: &str) {
        if let Some(index) = self.find_icon_by_name(name) {
            if let Ok((x, y)) = self.get_icon_position(index) {
                self.name_to_position.insert(name.to_string(), (x, y));
            }
        }
    }

    /// 重建 name → index 映射缓存
    ///
    /// 遍历 SysListView32 中所有图标，读取文本名称并建立映射。
    /// 应在以下时机调用：
    /// - 初始化时
    /// - explorer.exe 重启后
    /// - 发现缓存 index 失效时
    pub fn rebuild_index(&mut self) -> Result<()> {
        // 检查 SysListView32 是否仍然有效
        if unsafe { IsWindow(self.listview_hwnd) } == 0 {
            // 窗口已失效（explorer 重启），尝试重新查找
            match find_desktop_listview() {
                Ok(hwnd) => {
                    self.listview_hwnd = hwnd;
                    // 重新打开进程
                    let mut pid: u32 = 0;
                    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
                    if !self.explorer_process.is_null() {
                        unsafe { CloseHandle(self.explorer_process) };
                    }
                    self.explorer_process = unsafe {
                        OpenProcess(
                            PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE,
                            0,
                            pid,
                        )
                    };
                    if self.explorer_process.is_null() {
                        anyhow::bail!("重新打开 explorer 进程失败");
                    }
                    log::info!("SysListView32 已重新定位（explorer 重启后）");
                }
                Err(e) => {
                    anyhow::bail!("SysListView32 窗口已失效且无法重新查找: {}", e);
                }
            }
        }

        let count = self.get_icon_count();
        if count <= 0 {
            log::warn!("SysListView32 中没有图标（count={}）", count);
            self.name_to_index.clear();
            return Ok(());
        }

        let mut new_map = HashMap::with_capacity(count as usize);

        for i in 0..count {
            match self.read_item_text(i) {
                Ok(name) if !name.is_empty() => {
                    new_map.insert(name, i);
                }
                Ok(_) => {} // 空名称，跳过
                Err(e) => {
                    log::warn!("读取图标 index={} 名称失败: {}", i, e);
                }
            }
        }

        // 同时读取所有图标的位置（用于恢复）
        for (name, &index) in &new_map {
            if let Ok(pos) = self.get_icon_position(index) {
                self.name_to_position.insert(name.clone(), pos);
            }
        }

        log::info!(
            "索引重建完成: {} → {} 个图标",
            self.name_to_index.len(),
            new_map.len()
        );
        self.name_to_index = new_map;
        Ok(())
    }

    /// 获取索引缓存中的所有名称
    pub fn cached_names(&self) -> Vec<String> {
        self.name_to_index.keys().cloned().collect()
    }

    /// 获取保存的图标原始位置（用于恢复时定位）
    pub fn get_saved_position(&self, name: &str) -> Option<(i32, i32)> {
        self.name_to_position.get(name).copied()
    }

    /// 检查 SysListView32 窗口是否仍然有效
    pub fn is_valid(&self) -> bool {
        unsafe { IsWindow(self.listview_hwnd) != 0 }
    }

    /// 释放 explorer 进程句柄
    pub fn cleanup(&mut self) {
        if !self.explorer_process.is_null() {
            unsafe { CloseHandle(self.explorer_process) };
            self.explorer_process = std::ptr::null_mut();
        }
        self.name_to_index.clear();
        self.name_to_position.clear();
        log::info!("桌面图标管理器已清理");
    }

    // ===== 内部方法 =====

    /// 设置图标在 SysListView32 中的位置
    fn set_icon_position(&self, index: i32, x: i32, y: i32) -> Result<()> {
        if unsafe { IsWindow(self.listview_hwnd) } == 0 {
            anyhow::bail!("SysListView32 窗口已失效");
        }
        let lparam = ((y & 0xFFFF) << 16) | (x & 0xFFFF);
        unsafe {
            SendMessageW(
                self.listview_hwnd,
                LVM_SETITEMPOSITION,
                index as WPARAM,
                lparam as LPARAM,
            )
        };
        Ok(())
    }

    /// 获取图标在 SysListView32 中的位置
    fn get_icon_position(&self, index: i32) -> Result<(i32, i32)> {
        if self.explorer_process.is_null() {
            anyhow::bail!("explorer 进程句柄无效");
        }

        // 在 explorer.exe 中分配内存用于接收 POINT 结构
        let remote_point = unsafe {
            VirtualAllocEx(
                self.explorer_process,
                std::ptr::null_mut(),
                std::mem::size_of::<POINT>(),
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if remote_point.is_null() {
            anyhow::bail!("VirtualAllocEx 分配 POINT 内存失败");
        }

        // 发送 LVM_GETITEMPOSITION
        let result = unsafe {
            SendMessageW(
                self.listview_hwnd,
                LVM_GETITEMPOSITION,
                index as WPARAM,
                remote_point as LPARAM,
            )
        };

        let mut point = POINT { x: 0, y: 0 };
        let mut bytes_read: usize = 0;
        let read_ok = unsafe {
            ReadProcessMemory(
                self.explorer_process,
                remote_point,
                &mut point as *mut POINT as *mut c_void,
                std::mem::size_of::<POINT>(),
                &mut bytes_read,
            )
        };

        // 释放远程内存
        unsafe {
            VirtualFreeEx(self.explorer_process, remote_point, 0, MEM_RELEASE);
        }

        if result == 0 || read_ok == 0 {
            anyhow::bail!("LVM_GETITEMPOSITION 或 ReadProcessMemory 失败（index={}）", index);
        }

        Ok((point.x, point.y))
    }

    /// 跨进程读取 ListView 项目的文本名称
    ///
    /// 使用 VirtualAllocEx 在 explorer.exe 中分配 LVITEMW + 文本缓冲区，
    /// 通过 LVM_GETITEMTEXTW 读取项目名称。
    fn read_item_text(&self, index: i32) -> Result<String> {
        if self.explorer_process.is_null() {
            anyhow::bail!("explorer 进程句柄无效");
        }

        let lvitem_size = std::mem::size_of::<LVITEMW>();
        let text_bytes = TEXT_BUFFER_CHARS * 2; // UTF-16: 每字符 2 字节
        let total_size = lvitem_size + text_bytes;

        // 在 explorer.exe 中分配连续内存：[LVITEMW][text_buffer]
        let remote_base = unsafe {
            VirtualAllocEx(
                self.explorer_process,
                std::ptr::null_mut(),
                total_size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if remote_base.is_null() {
            anyhow::bail!("VirtualAllocEx 分配 LVITEMW 内存失败");
        }

        // 文本缓冲区在远程内存中的地址
        let remote_text =
            (remote_base as usize + lvitem_size) as *mut u16;

        // 构建本地 LVITEMW
        let lvitem = LVITEMW {
            mask: LVIF_TEXT,
            i_item: index,
            i_sub_item: 0,
            state: 0,
            state_mask: 0,
            psz_text: remote_text,
            cch_text_max: TEXT_BUFFER_CHARS as i32,
            i_image: 0,
            l_param: 0,
            i_indent: 0,
        };

        // 写入 LVITEMW 到远程内存
        let mut bytes_written: usize = 0;
        let write_ok = unsafe {
            WriteProcessMemory(
                self.explorer_process,
                remote_base,
                &lvitem as *const LVITEMW as *const c_void,
                lvitem_size,
                &mut bytes_written,
            )
        };

        if write_ok == 0 {
            unsafe { VirtualFreeEx(self.explorer_process, remote_base, 0, MEM_RELEASE) };
            anyhow::bail!("WriteProcessMemory 写入 LVITEMW 失败");
        }

        // 发送 LVM_GETITEMTEXTW
        unsafe {
            SendMessageW(
                self.listview_hwnd,
                LVM_GETITEMTEXTW,
                index as WPARAM,
                remote_base as LPARAM,
            )
        };

        // 读取文本缓冲区
        let mut text_buf = vec![0u16; TEXT_BUFFER_CHARS];
        let mut bytes_read: usize = 0;
        let read_ok = unsafe {
            ReadProcessMemory(
                self.explorer_process,
                remote_text as *const c_void,
                text_buf.as_mut_ptr() as *mut c_void,
                text_bytes,
                &mut bytes_read,
            )
        };

        // 释放远程内存
        unsafe {
            VirtualFreeEx(self.explorer_process, remote_base, 0, MEM_RELEASE);
        }

        if read_ok == 0 {
            anyhow::bail!("ReadProcessMemory 读取文本失败（index={}）", index);
        }

        // 找到 null terminator 并转换为 String
        let len = text_buf.iter().position(|&c| c == 0).unwrap_or(text_buf.len());
        Ok(String::from_utf16_lossy(&text_buf[..len]))
    }
}

impl Drop for DesktopIconManager {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// 查找桌面 SysListView32 句柄
///
/// 逐层查找：Progman → SHELLDLL_DefView → SysListView32，
/// 或 WorkerW → SHELLDLL_DefView → SysListView32。
fn find_desktop_listview() -> Result<HWND> {
    unsafe {
        // 尝试 Progman
        let progman = FindWindowW(PROG_CLASS.as_ptr(), std::ptr::null());
        if !progman.is_null() {
            let defview = FindWindowExW(
                progman,
                std::ptr::null_mut(),
                DEFVIEW_CLASS.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    LISTVIEW_CLASS.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Ok(lv);
                }
            }
        }

        // 尝试 WorkerW
        let mut worker = FindWindowW(WORKER_CLASS.as_ptr(), std::ptr::null());
        while !worker.is_null() {
            let defview = FindWindowExW(
                worker,
                std::ptr::null_mut(),
                DEFVIEW_CLASS.as_ptr(),
                std::ptr::null(),
            );
            if !defview.is_null() {
                let lv = FindWindowExW(
                    defview,
                    std::ptr::null_mut(),
                    LISTVIEW_CLASS.as_ptr(),
                    std::ptr::null(),
                );
                if !lv.is_null() {
                    return Ok(lv);
                }
            }
            worker = FindWindowExW(
                std::ptr::null_mut(),
                worker,
                WORKER_CLASS.as_ptr(),
                std::ptr::null(),
            );
        }

        anyhow::bail!("无法找到 SysListView32")
    }
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lvitemw_size_64bit() {
        // 64-bit Windows 上 LVITEMW（不含 iGroupId 等扩展字段）应为 56 字节
        let size = std::mem::size_of::<LVITEMW>();
        if cfg!(target_pointer_width = "64") {
            assert_eq!(size, 56, "LVITEMW 在 64-bit 平台应为 56 字节，实际 {}", size);
        }
    }

    #[test]
    fn test_hidden_icon_constants() {
        // 隐藏坐标应在屏幕可视区域之外
        assert!(HIDDEN_ICON_X < 0, "隐藏 X 坐标应为负值");
        assert!(HIDDEN_ICON_Y < 0, "隐藏 Y 坐标应为负值");
    }

    #[test]
    fn test_drag_threshold() {
        assert_eq!(DRAG_THRESHOLD, 5, "拖拽启动阈值应为 5 像素");
    }

    #[test]
    fn test_drag_item_info_clone() {
        let info = DragItemInfo {
            index: 3,
            file_path: "C:\\test.lnk".to_string(),
            display_name: "测试".to_string(),
            icon_handle: std::ptr::null_mut(),
        };
        let cloned = info.clone();
        assert_eq!(cloned.index, 3);
        assert_eq!(cloned.file_path, "C:\\test.lnk");
        assert_eq!(cloned.display_name, "测试");
    }

    #[test]
    fn test_custom_message_values() {
        // 确保自定义消息不与系统消息冲突
        assert!(WM_USER_DRAG_ITEM_REMOVED > WM_APP);
        assert!(WM_USER_DRAG_ITEM_ADDED > WM_APP);
        assert_ne!(WM_USER_DRAG_ITEM_REMOVED, WM_USER_DRAG_ITEM_ADDED);
    }

    #[test]
    fn test_lvm_constants() {
        // 验证 LVM 常量值与 Windows SDK 一致
        assert_eq!(LVM_GETITEMCOUNT, 0x1004);
        assert_eq!(LVM_GETITEMTEXTW, 0x1073);
        assert_eq!(LVM_GETITEMPOSITION, 0x1010);
        assert_eq!(LVM_SETITEMPOSITION, 0x100F);
        assert_eq!(LVIF_TEXT, 0x0001);
    }

    #[test]
    fn test_process_memory_constants() {
        // 验证进程/内存常量与 Windows SDK 一致
        assert_eq!(PROCESS_VM_OPERATION, 0x0008);
        assert_eq!(PROCESS_VM_READ, 0x0010);
        assert_eq!(PROCESS_VM_WRITE, 0x0020);
        assert_eq!(MEM_COMMIT, 0x1000);
        assert_eq!(MEM_RESERVE, 0x2000);
        assert_eq!(MEM_RELEASE, 0x8000);
        assert_eq!(PAGE_READWRITE, 0x04);
    }
}
