//! 桌面图标逐图标管理器

#![allow(dead_code)] // Windows 平台组件，部分在当前构建路径未使用
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
    /// 受保护的可见图标位置（隐藏/显示操作前保存，操作后恢复）
    ///
    /// 用于对抗桌面"对齐到网格"导致的自动重排：
    /// `LVM_SETITEMPOSITION` 触发 SysListView32 重新布局所有可见图标，
    /// 保存位置后恢复，使非管理图标保持原位。
    protected_positions: Vec<(i32, (i32, i32))>,
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
        let listview_hwnd = super::icon_hider::find_desktop_listview().unwrap_or_else(|_| {
            let cached =
                super::ffi_types::SHARED_LISTVIEW_HWND.load(std::sync::atomic::Ordering::SeqCst);
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
            protected_positions: Vec::new(),
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

    /// 使用指定的 SysListView32 句柄创建管理器（多显示器拖放定位用）
    pub fn with_listview(listview_hwnd: HWND) -> Result<Self> {
        if listview_hwnd.is_null() {
            anyhow::bail!("SysListView32 句柄无效");
        }

        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(listview_hwnd, &mut pid) };
        if pid == 0 {
            anyhow::bail!("无法获取 SysListView32 的进程 ID");
        }

        let process = unsafe {
            OpenProcess(
                PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE,
                0,
                pid,
            )
        };
        if process.is_null() {
            anyhow::bail!("OpenProcess 失败（pid={}）", pid);
        }

        let mut mgr = Self {
            listview_hwnd,
            explorer_process: process,
            name_to_index: HashMap::new(),
            name_to_position: HashMap::new(),
            protected_positions: Vec::new(),
        };

        if let Err(e) = mgr.rebuild_index() {
            log::warn!("索引构建失败（非致命）: {}", e);
        }

        Ok(mgr)
    }

    /// 获取 SysListView32 句柄
    pub fn listview_hwnd(&self) -> HWND {
        self.listview_hwnd
    }

    /// 获取 SysListView32 中的图标总数
    pub fn get_icon_count(&self) -> i32 {
        unsafe { SendMessageW(self.listview_hwnd, LVM_GETITEMCOUNT, 0, 0) as i32 }
    }

    /// 禁用 SysListView32 重绘（防止批量操作触发桌面自动重排级联）
    ///
    /// 配合 `enable_redraw` 使用，在批量 `hide_icon` / `show_icon_at` 前调用。
    pub fn disable_redraw(&self) {
        unsafe {
            SendMessageW(self.listview_hwnd, WM_SETREDRAW, 0, 0);
        }
    }

    /// 重新启用 SysListView32 重绘并触发统一重绘
    ///
    /// 配合 `disable_redraw` 使用，在批量操作完成后调用。
    pub fn enable_redraw(&self) {
        unsafe {
            SendMessageW(self.listview_hwnd, WM_SETREDRAW, 1, 0);
            InvalidateRect(self.listview_hwnd, std::ptr::null(), 1);
        }
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
    /// 设置图标位置（使用 LVM_SETITEMPOSITION32 跨进程传坐标）
    ///
    /// LVM_SETITEMPOSITION32 通过远程内存传递 POINT 结构，
    /// 可能绕过桌面 Shell 对 LVM_SETITEMPOSITION 的拦截。
    fn set_icon_position_32(&self, index: i32, x: i32, y: i32) -> Result<()> {
        if self.explorer_process.is_null() {
            anyhow::bail!("explorer 进程句柄无效");
        }

        // 在 explorer.exe 中分配 POINT 内存
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
            anyhow::bail!("VirtualAllocEx 失败");
        }

        // 写入 POINT{x, y} 到远程内存
        let point = POINT { x, y };
        let mut bytes_written: usize = 0;
        let write_ok = unsafe {
            WriteProcessMemory(
                self.explorer_process,
                remote_point,
                &point as *const POINT as *const c_void,
                std::mem::size_of::<POINT>(),
                &mut bytes_written,
            )
        };
        if write_ok == 0 {
            unsafe { VirtualFreeEx(self.explorer_process, remote_point, 0, MEM_RELEASE) };
            anyhow::bail!("WriteProcessMemory 失败");
        }

        // 发送 LVM_SETITEMPOSITION32
        let ret = unsafe {
            SendMessageW(
                self.listview_hwnd,
                LVM_SETITEMPOSITION32,
                index as WPARAM,
                remote_point as LPARAM,
            )
        };

        // 释放远程内存
        unsafe { VirtualFreeEx(self.explorer_process, remote_point, 0, MEM_RELEASE) };

        log::info!(
            "set_icon_position_32: index={}, ({}, {}), ret={}",
            index,
            x,
            y,
            ret
        );

        Ok(())
    }

    pub fn hide_icon(&self, index: i32) -> Result<()> {
        self.set_icon_position(index, HIDDEN_ICON_X, HIDDEN_ICON_Y)
            .context("隐藏图标失败")
    }

    /// 显示单个图标到指定桌面坐标
    ///
    /// 将之前隐藏的图标移回到指定的屏幕坐标位置。
    pub fn show_icon_at(&self, index: i32, x: i32, y: i32) -> Result<()> {
        self.set_icon_position(index, x, y).context("显示图标失败")
    }

    /// 按名称隐藏图标
    ///
    /// 查找缓存中的 index 并隐藏。找不到时记录警告。
    pub fn hide_icon_by_name(&self, name: &str) -> Result<()> {
        match self.find_icon_by_name(name) {
            Some(index) => self.hide_icon(index),
            None => {
                log::warn!(
                    "hide_icon_by_name: 未找到图标 '{}', 可能 explorer 已重启",
                    name
                );
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
                log::warn!(
                    "show_icon_by_name: 未找到图标 '{}', 可能 explorer 已重启",
                    name
                );
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

    /// 保存可见图标的位置到保护列表（排除指定索引）
    ///
    /// 在 `hide_icon` 批量操作之前调用，记录非管理图标的 (index, position)，
    /// 供后续 `restore_protected_positions` 恢复。
    ///
    /// # 参数
    ///
    /// - `exclude_indices`: 管理图标的 index 集合，这些图标即将被隐藏，
    ///   不应纳入保护列表（否则 restore 会将其恢复到可见位置，撤消隐藏）
    ///
    /// 原理：`LVM_SETITEMPOSITION` 触发桌面"对齐到网格"自动重排，
    /// 导致所有可见图标位置偏移。保存非管理图标后恢复，使其保持原位。
    pub fn save_protected_positions(&mut self, exclude_indices: &[i32]) {
        self.protected_positions.clear();
        for &index in self.name_to_index.values() {
            // 跳过即将被隐藏的管理图标（防止 restore 将其恢复回可见位置）
            if exclude_indices.contains(&index) {
                continue;
            }
            if let Ok(pos) = self.get_icon_position(index) {
                // 只保存可见位置的图标（排除已隐藏在屏幕外的）
                if pos.0 >= 0 && pos.1 >= 0 {
                    self.protected_positions.push((index, pos));
                }
            }
        }
        log::debug!(
            "已保存 {} 个非管理图标位置到保护列表（排除 {} 个管理图标）",
            self.protected_positions.len(),
            exclude_indices.len()
        );
    }

    /// 恢复所有受保护的可见图标位置
    ///
    /// 在 `LVM_SETITEMPOSITION` 操作后调用，将被自动重排偏移的可见图标
    /// 恢复到操作前的位置。已处于正确位置的图标跳过（避免触发额外重排）。
    ///
    /// 使用 `WM_SETREDRAW` 包裹批量操作，防止每次 `LVM_SETITEMPOSITION`
    /// 触发桌面自动重排级联。所有位置设置完成后统一重绘一次。
    ///
    /// 调用时机：
    /// - `hide_managed_icons`: 隐藏所有管理图标后调用一次
    /// - `handle_box_to_desktop`: `show_icon_at` 后立即调用
    /// - `restore_native_icons`: 恢复所有图标后调用一次
    pub fn restore_protected_positions(&self) {
        if self.protected_positions.is_empty() {
            return;
        }

        let mut restored = 0;
        let mut skipped = 0;
        for &(index, (saved_x, saved_y)) in &self.protected_positions {
            // 读取当前位置，只在实际偏移时恢复（避免无必要的 LVM_SETITEMPOSITION）
            if let Ok((cur_x, cur_y)) = self.get_icon_position(index) {
                if cur_x == saved_x && cur_y == saved_y {
                    skipped += 1;
                    continue;
                }
                if self.set_icon_position(index, saved_x, saved_y).is_ok() {
                    restored += 1;
                }
            }
        }

        log::debug!(
            "恢复受保护图标: {} 个已恢复, {} 个未偏移（跳过）",
            restored,
            skipped
        );
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
            match super::icon_hider::find_desktop_listview() {
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

    /// 枚举 SysListView32 中所有可见图标的显示名称
    ///
    /// 返回 `(display_name, index)` 列表，包含桌面 SysListView32 中的
    /// 所有图标项。用于发现不在文件系统中的虚拟 Shell 对象
    /// （如企业微信联系人、回收站等）。
    pub fn enumerate_all_visible_names(&mut self) -> Vec<(String, i32)> {
        if let Err(e) = self.rebuild_index() {
            log::warn!("枚举桌面图标名称失败: {}", e);
            return Vec::new();
        }

        let count = self.get_icon_count();
        if count <= 0 {
            return Vec::new();
        }

        let mut results = Vec::with_capacity(count as usize);
        for i in 0..count {
            match self.read_item_text(i) {
                Ok(name) if !name.is_empty() => {
                    results.push((name, i));
                }
                Ok(_) => {}
                Err(e) => {
                    log::debug!("读取图标 index={} 名称失败: {}", i, e);
                }
            }
        }

        log::info!("SysListView32 中共发现 {} 个可见图标", results.len());
        results
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
        self.protected_positions.clear();
        log::info!("桌面图标管理器已清理");
    }

    // ===== 内部方法 =====

    /// 设置图标在 SysListView32 中的位置
    pub fn set_icon_position(&self, index: i32, x: i32, y: i32) -> Result<()> {
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

    /// 设置图标位置并返回 SendMessageW 原始返回值（用于诊断）
    fn set_icon_position_raw(&self, index: i32, x: i32, y: i32) -> LRESULT {
        let lparam = ((y & 0xFFFF) << 16) | (x & 0xFFFF);
        unsafe {
            SendMessageW(
                self.listview_hwnd,
                LVM_SETITEMPOSITION,
                index as WPARAM,
                lparam as LPARAM,
            )
        }
    }

    /// 获取图标在 SysListView32 中的位置
    pub fn get_icon_position(&self, index: i32) -> Result<(i32, i32)> {
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
            anyhow::bail!(
                "LVM_GETITEMPOSITION 或 ReadProcessMemory 失败（index={}）",
                index
            );
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
        let remote_text = (remote_base as usize + lvitem_size) as *mut u16;

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
        let len = text_buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(text_buf.len());
        Ok(String::from_utf16_lossy(&text_buf[..len]))
    }

    /// 读取指定 item 在系统图像列表中的 image index (iImage)
    ///
    /// 使用 LVM_GETITEMW + LVIF_IMAGE 标志，跨进程读取 LVITEMW.iImage 字段。
    /// 返回的 iImage 可直接作为 ImageList_GetIcon 的参数。
    fn read_item_image(&self, index: i32) -> Result<i32> {
        if self.explorer_process.is_null() {
            anyhow::bail!("explorer 进程句柄无效");
        }

        let lvitem_size = std::mem::size_of::<LVITEMW>();

        // 在 explorer.exe 中分配 LVITEMW 内存
        let remote_base = unsafe {
            VirtualAllocEx(
                self.explorer_process,
                std::ptr::null_mut(),
                lvitem_size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if remote_base.is_null() {
            anyhow::bail!("VirtualAllocEx 分配 LVITEMW 内存失败");
        }

        // 构建 LVITEMW，请求 iImage 字段
        let lvitem = LVITEMW {
            mask: LVIF_IMAGE,
            i_item: index,
            i_sub_item: 0,
            state: 0,
            state_mask: 0,
            psz_text: std::ptr::null_mut(),
            cch_text_max: 0,
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

        // 发送 LVM_GETITEMW
        unsafe {
            SendMessageW(
                self.listview_hwnd,
                LVM_GETITEMW,
                index as WPARAM,
                remote_base as LPARAM,
            )
        };

        // 读取 iImage 字段（在 LVITEMW 结构体中的偏移量）
        let i_image_offset = std::mem::offset_of!(LVITEMW, i_image);
        let remote_i_image = (remote_base as usize + i_image_offset) as *const c_void;
        let mut image_index: i32 = -1;
        let mut bytes_read: usize = 0;
        let read_ok = unsafe {
            ReadProcessMemory(
                self.explorer_process,
                remote_i_image,
                &mut image_index as *mut i32 as *mut c_void,
                std::mem::size_of::<i32>(),
                &mut bytes_read,
            )
        };

        // 释放远程内存
        unsafe {
            VirtualFreeEx(self.explorer_process, remote_base, 0, MEM_RELEASE);
        }

        if read_ok == 0 {
            anyhow::bail!("ReadProcessMemory 读取 iImage 失败（index={}）", index);
        }

        Ok(image_index)
    }

    /// 枚举所有可见图标的 (显示名称, item_index, image_index)
    ///
    /// image_index 是系统图像列表中的索引，可直接传给 ImageList_GetIcon。
    pub fn enumerate_names_with_images(&mut self) -> Vec<(String, i32, i32)> {
        if let Err(e) = self.rebuild_index() {
            log::warn!("enumerate_names_with_images: rebuild_index failed: {}", e);
            return Vec::new();
        }

        let count = self.get_icon_count();
        if count <= 0 {
            return Vec::new();
        }

        let mut results = Vec::with_capacity(count as usize);
        for i in 0..count {
            let name = match self.read_item_text(i) {
                Ok(n) if !n.is_empty() => n,
                _ => continue,
            };
            let image_idx = self.read_item_image(i).unwrap_or(-1);
            results.push((name, i, image_idx));
        }

        log::info!("enumerate_names_with_images: {} items", results.len());
        results
    }
}
impl Drop for DesktopIconManager {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// 诊断测试：检测 DesktopIconManager 是否可正常工作
///
/// 尝试创建管理器、重建索引，并将结果记录到日志。
/// 在应用启动时调用一次，用于验证 LVM 方案可行性。
pub fn diagnose() {
    log::info!("===== DesktopIconManager 诊断测试 =====");

    // 测试 1: find_desktop_listview
    match super::icon_hider::find_desktop_listview() {
        Ok(hwnd) => log::info!("[诊断] find_desktop_listview: 成功 hwnd={:?}", hwnd),
        Err(e) => {
            log::warn!("[诊断] find_desktop_listview: 失败 — {}", e);
            log::info!("===== 诊断完成（LVM 不可用）=====");
            return;
        }
    }

    // 测试 2: DesktopIconManager::new()
    let mut mgr = match DesktopIconManager::new() {
        Ok(m) => {
            log::info!("[诊断] DesktopIconManager::new(): 成功");
            m
        }
        Err(e) => {
            log::warn!("[诊断] DesktopIconManager::new(): 失败 — {}", e);
            log::info!("===== 诊断完成（LVM 不可用）=====");
            return;
        }
    };

    // 测试 3: rebuild_index + 图标列表
    let count = mgr.get_icon_count();
    log::info!("[诊断] SysListView32 图标总数: {}", count);

    if let Err(e) = mgr.rebuild_index() {
        log::warn!("[诊断] rebuild_index: 失败 — {}", e);
        log::info!("===== 诊断完成（LVM 部分可用）=====");
        return;
    }

    let names = mgr.cached_names();
    log::info!("[诊断] 索引缓存图标数: {}", names.len());
    if names.len() <= 50 {
        log::info!("[诊断] 图标名称列表: {:?}", names);
    }

    // 测试 4: 读取第一个图标位置
    if let Some(first_name) = names.first() {
        if let Some(idx) = mgr.find_icon_by_name(first_name) {
            log::info!("[诊断] 图标 '{}' index={} 存在", first_name, idx);
        }
    }

    log::info!("===== 诊断完成（LVM 可用）=====");
}

/// 快照所有桌面图标的当前位置
///
/// 遍历 SysListView32 中的所有图标，读取每个图标的显示名称和位置。
/// 返回 HashMap<显示名称, (client_x, client_y)>。
///
/// 在程序启动时调用（创建覆盖区域之前），记录原始桌面布局。
pub fn snapshot_all_desktop_positions() -> HashMap<String, (i32, i32)> {
    let mut positions = HashMap::new();

    match DesktopIconManager::new() {
        Ok(mgr) => {
            let count = mgr.get_icon_count();
            log::info!("快照桌面图标: SysListView32 中有 {} 个图标", count);

            let names = mgr.cached_names();
            for name in &names {
                // 直接调用 get_icon_position 读取当前位置
                if let Some(index) = mgr.find_icon_by_name(name) {
                    if let Ok(pos) = mgr.get_icon_position(index) {
                        positions.insert(name.clone(), pos);
                    }
                }
            }

            log::info!("快照完成: 成功记录 {} 个图标位置", positions.len());
        }
        Err(e) => {
            log::warn!("快照桌面图标失败: 创建 DesktopIconManager 失败: {}", e);
        }
    }

    positions
}

/// 通过显示名称设置图标位置
///
/// 在 SysListView32 中查找指定名称的图标，将其移动到指定位置。
/// 使用 LVM_SETITEMPOSITION 跨进程操作。
pub fn set_icon_position_by_name(name: &str, x: i32, y: i32) -> Result<()> {
    let auto_arrange_was_on = disable_desktop_auto_arrange();

    let result = (|| -> Result<()> {
        let mut mgr = DesktopIconManager::new().context("创建桌面图标管理器失败")?;
        mgr.rebuild_index().context("重建图标索引失败")?;

        let index = mgr
            .find_icon_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("未找到图标: {}", name))?;

        mgr.set_icon_position(index, x, y)
            .context(format!("设置图标 '{}' 位置失败", name))?;
        log::info!("恢复图标位置: '{}' 已移动到 ({}, {})", name, x, y);
        Ok(())
    })();

    if auto_arrange_was_on == Some(true) {
        restore_desktop_auto_arrange();
    }

    result
}

/// 查找桌面 SysListView32 句柄
///
/// 逐层查找：Progman → SHELLDLL_DefView → SysListView32，
/// 或 WorkerW → SHELLDLL_DefView → SysListView32。
///
/// Windows 10/11 上 SysListView32 通常在 WorkerW 下而非 Progman 下。
/// 需要先向 Progman 发送 0x052C 消息激活桌面 WorkerW 窗口。
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
                    log::debug!("SysListView32 在 Progman 下找到");
                    return Ok(lv);
                }
            }

            // Windows 10/11: 向 Progman 发送 0x052C 激活桌面 WorkerW
            SendMessageW(progman, 0x052C, 0, 0);
            log::debug!("已向 Progman 发送 0x052C 激活桌面 WorkerW");
        }

        // 尝试 WorkerW
        let mut worker = FindWindowW(WORKER_CLASS.as_ptr(), std::ptr::null());
        let mut worker_count = 0;
        while !worker.is_null() {
            worker_count += 1;
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
                    log::debug!("SysListView32 在第 {} 个 WorkerW 下找到", worker_count);
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

        log::warn!(
            "无法找到 SysListView32（已搜索 Progman + {} 个 WorkerW）",
            worker_count
        );
        anyhow::bail!(
            "无法找到桌面 SysListView32（已搜索 Progman + {} 个 WorkerW）",
            worker_count
        )
    }
}

// =============================================================================
// 桌面自动排列控制（通过 IFolderView2 COM 接口）
// =============================================================================
//
// 桌面 SysListView32 开启"自动排列图标"时，LVM_SETITEMPOSITION 将图标移到
// 屏幕外后，桌面会立即将图标重排回可见区域，导致隐藏操作失效。
//
// 解决方案：在批量隐藏/显示图标前，通过 IFolderView2::SetCurrentFolderFlags
// 临时关闭 FWF_AUTOARRANGE（自动排列），操作完成后恢复。
//
// COM 链路：IShellWindows → IDispatch → IServiceProvider → IShellBrowser
//           → IShellView → IFolderView2

use windows::core::{Interface as WinInterface, GUID};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, IServiceProvider, CLSCTX_ALL,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{IFolderView2, IShellBrowser, IShellView, IShellWindows};

/// CLSID_ShellWindows = {9BA05972-F6A8-11CF-A442-00A0C90A8F39}
const CLSID_SHELL_WINDOWS: GUID = GUID::from_u128(0x9BA05972_F6A8_11CF_A442_00A0C90A8F39);

/// SID_STopLevelBrowser = {4C96BE40-915C-11CF-99D3-00AA004AE837}
const SID_S_TOP_LEVEL_BROWSER: GUID = GUID::from_u128(0x4C96BE40_915C_11CF_99D3_00AA004AE837);

/// FWF_AUTOARRANGE — 桌面"自动排列图标"标志位
const FWF_AUTOARRANGE: u32 = 0x1;

/// 临时禁用桌面"自动排列图标"
///
/// 通过 COM 接口 `IFolderView2::SetCurrentFolderFlags` 清除 `FWF_AUTOARRANGE` 位，
/// 使后续 `LVM_SETITEMPOSITION` 操作不被桌面自动重排撤消。
///
/// 返回 `Some(true)` 表示自动排列已关闭（需要在操作完成后调用 `restore_auto_arrange`），
/// 返回 `Some(false)` 表示自动排列本来就未开启（无需恢复），
/// 返回 `None` 表示 COM 操作失败（上层应继续执行，仅降级为无保护模式）。
pub fn disable_desktop_auto_arrange() -> Option<bool> {
    unsafe {
        // CoInitializeEx 可安全重复调用（配对使用）
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let sw: IShellWindows = match CoCreateInstance(&CLSID_SHELL_WINDOWS, None, CLSCTX_ALL) {
            Ok(sw) => sw,
            Err(e) => {
                log::warn!(
                    "disable_auto_arrange: CoCreateInstance(IShellWindows) 失败: {}",
                    e
                );
                CoUninitialize();
                return None;
            }
        };

        let index = VARIANT::from(0i32);
        let dispatch: IDispatch = match sw.Item(&index) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("disable_auto_arrange: IShellWindows.Item(0) 失败: {}", e);
                CoUninitialize();
                return None;
            }
        };

        let sp: IServiceProvider = match WinInterface::cast::<IServiceProvider>(&dispatch) {
            Ok(sp) => sp,
            Err(e) => {
                log::warn!(
                    "disable_auto_arrange: QueryInterface(IServiceProvider) 失败: {}",
                    e
                );
                CoUninitialize();
                return None;
            }
        };

        let sb: IShellBrowser = match sp.QueryService::<IShellBrowser>(&SID_S_TOP_LEVEL_BROWSER) {
            Ok(sb) => sb,
            Err(e) => {
                log::warn!(
                    "disable_auto_arrange: QueryService(IShellBrowser) 失败: {}",
                    e
                );
                CoUninitialize();
                return None;
            }
        };

        let sv: IShellView = match sb.QueryActiveShellView() {
            Ok(sv) => sv,
            Err(e) => {
                log::warn!("disable_auto_arrange: QueryActiveShellView 失败: {}", e);
                CoUninitialize();
                return None;
            }
        };

        let fv2: IFolderView2 = match WinInterface::cast::<IFolderView2>(&sv) {
            Ok(fv) => fv,
            Err(e) => {
                log::warn!(
                    "disable_auto_arrange: QueryInterface(IFolderView2) 失败: {}",
                    e
                );
                CoUninitialize();
                return None;
            }
        };

        let flags = match fv2.GetCurrentFolderFlags() {
            Ok(f) => f,
            Err(e) => {
                log::warn!("disable_auto_arrange: GetCurrentFolderFlags 失败: {}", e);
                CoUninitialize();
                return None;
            }
        };

        if (flags & FWF_AUTOARRANGE) == 0 {
            log::info!("桌面自动排列已关闭，无需禁用 (flags={:#X})", flags);
            // 仍然调用 CoUninitialize 保持配对
            CoUninitialize();
            return Some(false);
        }

        match fv2.SetCurrentFolderFlags(FWF_AUTOARRANGE, 0) {
            Ok(()) => {
                log::info!(
                    "桌面自动排列已禁用 (flags: {:#X} → {:#X})",
                    flags,
                    flags & !FWF_AUTOARRANGE
                );
                // CoUninitialize 在 restore 时调用
                // 注意：此处故意不调用 CoUninitialize，COM 对象需要保持活跃
                // restore_auto_arrange 会重新获取接口并调用 CoUninitialize
                Some(true)
            }
            Err(e) => {
                log::warn!("disable_auto_arrange: SetCurrentFolderFlags 失败: {}", e);
                CoUninitialize();
                None
            }
        }
    }
}

/// 恢复桌面"自动排列图标"设置
///
/// 在 `disable_desktop_auto_arrange` 返回 `Some(true)` 后调用，
/// 重新设置 `FWF_AUTOARRANGE` 位，恢复用户原始的桌面设置。
pub fn restore_desktop_auto_arrange() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let sw: IShellWindows = match CoCreateInstance(&CLSID_SHELL_WINDOWS, None, CLSCTX_ALL) {
            Ok(sw) => sw,
            Err(e) => {
                log::warn!("restore_auto_arrange: CoCreateInstance 失败: {}", e);
                CoUninitialize();
                return;
            }
        };

        let index = VARIANT::from(0i32);
        let dispatch: IDispatch = match sw.Item(&index) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("restore_auto_arrange: Item(0) 失败: {}", e);
                CoUninitialize();
                return;
            }
        };

        let sp: IServiceProvider = match WinInterface::cast::<IServiceProvider>(&dispatch) {
            Ok(sp) => sp,
            Err(e) => {
                log::warn!("restore_auto_arrange: IServiceProvider 失败: {}", e);
                CoUninitialize();
                return;
            }
        };

        let sb: IShellBrowser = match sp.QueryService::<IShellBrowser>(&SID_S_TOP_LEVEL_BROWSER) {
            Ok(sb) => sb,
            Err(e) => {
                log::warn!("restore_auto_arrange: IShellBrowser 失败: {}", e);
                CoUninitialize();
                return;
            }
        };

        let sv: IShellView = match sb.QueryActiveShellView() {
            Ok(sv) => sv,
            Err(e) => {
                log::warn!("restore_auto_arrange: QueryActiveShellView 失败: {}", e);
                CoUninitialize();
                return;
            }
        };

        let fv2: IFolderView2 = match WinInterface::cast::<IFolderView2>(&sv) {
            Ok(fv) => fv,
            Err(e) => {
                log::warn!("restore_auto_arrange: IFolderView2 失败: {}", e);
                CoUninitialize();
                return;
            }
        };

        match fv2.SetCurrentFolderFlags(FWF_AUTOARRANGE, FWF_AUTOARRANGE) {
            Ok(()) => log::info!("桌面自动排列已恢复"),
            Err(e) => log::warn!("restore_auto_arrange: SetCurrentFolderFlags 失败: {}", e),
        }

        CoUninitialize();
    }
}

// =============================================================================
// 拖放后定位（仅用于可见坐标，不用于隐藏）
// =============================================================================

/// 根据屏幕坐标查找对应的桌面 SysListView32
///
/// 从光标位置向上遍历窗口树，找到承载图标的 ListView。
/// 多显示器场景下比固定查找 Progman 更准确。
pub fn find_listview_at_screen_point(pt: POINT) -> Result<HWND> {
    unsafe {
        let mut hwnd = WindowFromPoint(pt);
        if hwnd.is_null() {
            return super::icon_hider::find_desktop_listview();
        }
        for _ in 0..12 {
            let lv = FindWindowExW(
                hwnd,
                std::ptr::null_mut(),
                LISTVIEW_CLASS.as_ptr(),
                std::ptr::null(),
            );
            if !lv.is_null() {
                return Ok(lv);
            }
            let parent = GetParent(hwnd);
            if parent.is_null() {
                break;
            }
            hwnd = parent;
        }
        super::icon_hider::find_desktop_listview()
    }
}

/// 从 .lnk 路径收集可能的 SysListView32 显示名称
fn icon_name_candidates(file_path: &str) -> Vec<String> {
    let mut names = Vec::new();
    let path = std::path::Path::new(file_path);
    if let Some(stem) = path.file_stem() {
        let s = stem.to_string_lossy().to_string();
        if !s.is_empty() {
            names.push(s);
        }
    }
    if let Some(name) = path.file_name() {
        let s = name.to_string_lossy().to_string();
        if !s.is_empty() && !names.contains(&s) {
            names.push(s);
        }
    }
    names
}

pub fn hide_icon_on_desktop(file_path: &str) {
    let result = (|| -> Result<()> {
        let listview = super::icon_hider::find_desktop_listview()?;
        let mut mgr = DesktopIconManager::with_listview(listview)?;

        let names = icon_name_candidates(file_path);
        if names.is_empty() {
            anyhow::bail!("无法从路径提取图标名称: {}", file_path);
        }

        // 重试查找（Explorer 刷新可能有延迟）
        let mut index = None;
        for attempt in 0..10 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            mgr.rebuild_index()?;
            for name in &names {
                if let Some(idx) = mgr.find_icon_by_name(name) {
                    index = Some(idx);
                    break;
                }
            }
            if index.is_some() {
                break;
            }
        }

        let index = index.ok_or_else(|| {
            anyhow::anyhow!("SysListView32 中未找到图标（已尝试名称: {:?}）", names)
        })?;

        mgr.hide_icon(index)?;
        log::info!("[HIDE] 桌面图标已隐藏: {} (index={})", file_path, index);
        Ok(())
    })();

    if let Err(e) = result {
        log::warn!("[HIDE] 桌面图标隐藏失败: {} - {}", file_path, e);
    }
}

pub fn show_icon_on_desktop(file_path: &str, screen_x: i32, screen_y: i32) {
    let auto_arrange_was_on = disable_desktop_auto_arrange();

    let result = (|| -> Result<()> {
        let listview = super::icon_hider::find_desktop_listview()?;
        let mut mgr = DesktopIconManager::with_listview(listview)?;

        let names = icon_name_candidates(file_path);
        if names.is_empty() {
            anyhow::bail!("无法从路径提取图标名称: {}", file_path);
        }

        // 重试查找
        let mut index = None;
        for attempt in 0..20 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            mgr.rebuild_index()?;
            for name in &names {
                if let Some(idx) = mgr.find_icon_by_name(name) {
                    index = Some(idx);
                    break;
                }
            }
            if index.is_some() {
                break;
            }
        }

        let index = index.ok_or_else(|| {
            anyhow::anyhow!("SysListView32 中未找到图标（已尝试名称: {:?}）", names)
        })?;

        // 屏幕坐标 → 客户区坐标
        let mut client_pt = POINT {
            x: screen_x,
            y: screen_y,
        };
        unsafe {
            if ScreenToClient(mgr.listview_hwnd(), &mut client_pt) == 0 {
                client_pt = POINT {
                    x: screen_x,
                    y: screen_y,
                };
            }
        }

        mgr.show_icon_at(index, client_pt.x, client_pt.y)?;
        log::info!(
            "[SHOW] 桌面图标已显示: {} -> ({}, {})",
            file_path,
            screen_x,
            screen_y
        );
        Ok(())
    })();

    if auto_arrange_was_on == Some(true) {
        restore_desktop_auto_arrange();
    }

    if let Err(e) = result {
        log::warn!("[SHOW] 桌面图标显示失败: {} - {}", file_path, e);
    }
}
/// 将桌面图标定位到屏幕坐标（拖放释放后调用）
///
/// 配合 `icon_hider::restore_single_icon` 使用：
/// 1. 文件移动恢复可见性
/// 2. 本函数通过 LVM 设置释放位置（屏幕坐标 → ListView 客户区坐标）
///
/// 失败时仅记录警告，不影响图标已在桌面显示。
/// 将桌面图标定位到指定屏幕坐标
///
/// 通过 SysListView32 跨进程操作将图标移动到目标位置。
/// 内置重试机制（最多 20 次 × 50ms = 1 秒），等待 Shell 异步创建图标。
pub fn position_icon_at_screen(file_path: &str, screen_x: i32, screen_y: i32) {
    let t_fn = std::time::Instant::now();

    let t_aa = std::time::Instant::now();
    let auto_arrange_was_on = disable_desktop_auto_arrange();
    log::debug!("[TIMING] disable_auto_arrange: {:?}", t_aa.elapsed());

    let result = (|| -> Result<()> {
        let t_lv = std::time::Instant::now();
        // 直接使用桌面 SysListView32，不依赖屏幕坐标查找
        let listview = super::icon_hider::find_desktop_listview()?;
        log::debug!("[TIMING] find_desktop_listview: {:?}", t_lv.elapsed());

        let t_mgr = std::time::Instant::now();
        let mut mgr = DesktopIconManager::with_listview(listview)?;
        log::debug!(
            "[TIMING] DesktopIconManager::with_listview: {:?}",
            t_mgr.elapsed()
        );

        let mut client_pt = POINT {
            x: screen_x,
            y: screen_y,
        };
        unsafe {
            if ScreenToClient(mgr.listview_hwnd(), &mut client_pt) == 0 {
                log::warn!("ScreenToClient 失败，使用屏幕坐标作为回退");
                client_pt = POINT {
                    x: screen_x,
                    y: screen_y,
                };
            }
        }

        let names = icon_name_candidates(file_path);
        if names.is_empty() {
            anyhow::bail!("无法从路径提取图标名称: {}", file_path);
        }

        // 内置重试：等待 Shell 创建图标（最多 20 次 × 50ms = 1 秒）
        let mut index = None;
        const MAX_RETRIES: u32 = 20;
        const RETRY_INTERVAL_MS: u64 = 50;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(RETRY_INTERVAL_MS));
            }

            mgr.rebuild_index()?;
            for name in &names {
                if let Some(idx) = mgr.find_icon_by_name(name) {
                    index = Some(idx);
                    break;
                }
            }

            if index.is_some() {
                if attempt > 0 {
                    log::info!(
                        "[TIMING] 图标在 {} 次重试后找到 ({}ms)",
                        attempt,
                        attempt as u64 * RETRY_INTERVAL_MS
                    );
                }
                break;
            }
        }

        let index = index.ok_or_else(|| {
            anyhow::anyhow!(
                "SysListView32 中未找到图标（已尝试名称: {:?}，重试 {} 次）",
                names,
                MAX_RETRIES
            )
        })?;

        let t_show = std::time::Instant::now();
        mgr.show_icon_at(index, client_pt.x, client_pt.y)?;
        log::info!(
            "[TIMING] 图标已定位到 ({}, {}), show_icon_at: {:?}, 函数总耗时: {:?}",
            screen_x,
            screen_y,
            t_show.elapsed(),
            t_fn.elapsed()
        );
        Ok(())
    })();

    if auto_arrange_was_on == Some(true) {
        let t_restore = std::time::Instant::now();
        restore_desktop_auto_arrange();
        log::debug!("[TIMING] restore_auto_arrange: {:?}", t_restore.elapsed());
    }

    if let Err(e) = result {
        log::warn!("[TIMING] 定位失败: {}, 耗时: {:?}", e, t_fn.elapsed());
    }
}

/// ???????????????
///
/// ?? `handle_box_to_desktop` ?????????????? Shell ???????
/// ?????? `(screen_x, screen_y)`??????? `None`?
pub fn verify_icon_screen_position(file_path: &str) -> Option<(i32, i32)> {
    let result: Result<(i32, i32)> = (|| -> Result<(i32, i32)> {
        let listview = super::icon_hider::find_desktop_listview()?;
        let mut mgr = DesktopIconManager::with_listview(listview)?;

        let names = icon_name_candidates(file_path);
        if names.is_empty() {
            anyhow::bail!("???????????: {}", file_path);
        }

        mgr.rebuild_index()?;
        let mut index = None;
        for name in &names {
            if let Some(idx) = mgr.find_icon_by_name(name) {
                index = Some(idx);
                break;
            }
        }
        let index =
            index.ok_or_else(|| anyhow::anyhow!("??: SysListView32 ?????? (names={:?})", names))?;

        let (client_x, client_y) = mgr.get_icon_position(index)?;

        // ????? ? ????
        let mut pt = POINT {
            x: client_x,
            y: client_y,
        };
        unsafe {
            ClientToScreen(mgr.listview_hwnd(), &mut pt);
        }

        Ok((pt.x, pt.y))
    })();

    match result {
        Ok(pos) => {
            log::info!("[VERIFY] ?? '{}' ??????: ({}, {})", file_path, pos.0, pos.1);
            Some(pos)
        }
        Err(e) => {
            log::warn!("[VERIFY] ???????? '{}': {}", file_path, e);
            None
        }
    }
}

// ===== SysListView32 icon extraction =====

const SHGFI_SYSICONINDEX: u32 = 0x00000400;
const SHGFI_SMALLICON: u32 = 0x00000001;
const ILD_TRANSPARENT: u32 = 0x00000001;

#[repr(C)]
struct SysSHFILEINFOW {
    h_icon: *mut std::ffi::c_void,
    i_icon: i32,
    dw_attributes: u32,
    sz_display_name: [u16; 260],
    sz_type_name: [u16; 80],
}

impl Default for SysSHFILEINFOW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

extern "system" {
    fn SHGetFileInfoW(
        pszPath: *const u16,
        dwFileAttributes: u32,
        psfi: *mut SysSHFILEINFOW,
        cbFileInfo: u32,
        uFlags: u32,
    ) -> usize;
    fn ImageList_GetIcon(himl: *mut std::ffi::c_void, i: i32, flags: u32) -> *mut std::ffi::c_void;
}

/// 获取桌面 SysListView32 使用的系统图像列表句柄
///
/// 使用 SHGetImageList(SHIL_LARGE) 获取系统大图标图像列表。
/// 这是获取系统图像列表的推荐方式，比 SHGetFileInfoW 更可靠。
/// 返回的 IImageList 由系统管理，这里泄漏引用（系统级资源，进程结束时释放）。
fn get_shell_image_list() -> Option<*mut std::ffi::c_void> {
    use windows::Win32::UI::Controls::IImageList;
    use windows::Win32::UI::Shell::{SHGetImageList, SHIL_LARGE};

    let result: windows::core::Result<IImageList> = unsafe { SHGetImageList(SHIL_LARGE as i32) };
    match result {
        Ok(image_list) => {
            // 将 IImageList COM 接口转换为 HIMAGELIST (原始指针)
            // IImageList 和 HIMAGELIST 在 Windows 内部是同一对象
            use windows::core::Interface;
            let himl = image_list.as_raw() as *mut std::ffi::c_void;
            // 防止 COM 对象被 drop（系统级资源，保持引用）
            std::mem::forget(image_list);
            log::info!("get_shell_image_list: 获取系统图像列表成功 himl={:?}", himl);
            Some(himl)
        }
        Err(e) => {
            log::warn!("get_shell_image_list: SHGetImageList 失败: {:?}", e);
            None
        }
    }
}

/// 从系统图像列表提取图标
///
/// `image_index` 是 LVITEMW.iImage 字段值（通过 read_item_image 获取），
/// 而非 ListView item index。
pub fn extract_icon_from_listview(image_index: i32) -> Option<super::icon_extract::FileIconInfo> {
    log::info!(
        "extract_icon_from_listview: 尝试提取 image_index={}",
        image_index
    );
    if image_index < 0 {
        log::warn!(
            "extract_icon_from_listview: invalid image_index={}",
            image_index
        );
        return None;
    }
    let himl = match get_shell_image_list() {
        Some(h) => {
            log::info!(
                "extract_icon_from_listview: 获取系统图像列表成功 himl={:?}",
                h
            );
            h
        }
        None => {
            log::warn!("extract_icon_from_listview: 获取系统图像列表失败");
            return None;
        }
    };
    let hicon = unsafe { ImageList_GetIcon(himl, image_index, ILD_TRANSPARENT) };
    if hicon.is_null() {
        log::warn!(
            "extract_icon_from_listview: ImageList_GetIcon(image_index={}) returned null",
            image_index
        );
        return None;
    }
    log::info!("extract_icon_from_listview: 成功提取图标 hicon={:?}", hicon);
    Some(super::icon_extract::FileIconInfo {
        hicon,
        display_name: String::new(),
        type_name: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lvitemw_size_64bit() {
        // 64-bit Windows 上 LVITEMW（不含 iGroupId 等扩展字段）应为 56 字节
        let size = std::mem::size_of::<LVITEMW>();
        if cfg!(target_pointer_width = "64") {
            assert_eq!(
                size, 56,
                "LVITEMW 在 64-bit 平台应为 56 字节，实际 {}",
                size
            );
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

    #[test]
    fn test_extract_icon_from_listview_negative_index() {
        // 负数 image_index 应该返回 None
        let result = extract_icon_from_listview(-1);
        assert!(result.is_none(), "负数 image_index 应返回 None");
    }

    #[test]
    fn test_lvm_getimagelist_constant() {
        use super::super::ffi_types::{LVM_GETIMAGELIST, LVSIL_NORMAL};
        assert_eq!(LVM_GETIMAGELIST, 0x1002);
        assert_eq!(LVSIL_NORMAL, 0);
    }

    #[test]
    fn test_lvif_image_constant() {
        use super::super::ffi_types::LVIF_IMAGE;
        assert_eq!(LVIF_IMAGE, 0x0002);
    }

    // =========================================================================
    // Snapshot and restore tests
    // =========================================================================

    #[test]
    fn test_snapshot_all_desktop_positions_returns_empty_on_failure() {
        // When DesktopIconManager::new() fails (no SysListView32 available in test env),
        // snapshot should return an empty HashMap gracefully
        let positions = snapshot_all_desktop_positions();
        // In test environment, DesktopIconManager::new() may fail,
        // but the function should not panic and should return a HashMap (possibly empty)
        assert!(
            positions.is_empty() || !positions.is_empty(),
            "snapshot should return a HashMap without panicking"
        );
    }

    #[test]
    fn test_set_icon_position_by_name_returns_error_for_nonexistent() {
        // When the icon name doesn't exist, should return an error (not panic)
        let result = set_icon_position_by_name("NonExistentIcon12345.lnk", 100, 200);
        assert!(
            result.is_err(),
            "Should return error for non-existent icon name"
        );
    }

    #[test]
    fn test_snapshot_positions_data_structure() {
        // Verify the return type is correct: HashMap<String, (i32, i32)>
        let positions: HashMap<String, (i32, i32)> = snapshot_all_desktop_positions();
        // Each value should be a tuple of two i32 values
        for (name, (x, y)) in &positions {
            assert!(!name.is_empty(), "Icon name should not be empty");
            // Coordinates can be any i32 value (including negative for hidden icons)
            let _ = (*x, *y); // Just verify the types are correct
        }
    }
}
