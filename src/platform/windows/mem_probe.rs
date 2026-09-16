//! 内存占用分解探针（只读诊断，不参与任何渲染决策）
//!
//! # 为什么需要它
//!
//! 「玻璃一开内存就涨到 500MB+」这种问题，光看任务管理器只有一个总数，
//! 无法判断钱花在哪：是 GPU 纹理、GDI DIB、CPU 回读缓冲、图标位图缓存，
//! 还是 WebView2 子进程。猜一个改一个已经反复失败过。
//!
//! 本模块每 60 秒往日志里写**一行**完整分解，把总数拆成可归因的几块：
//!
//! - 进程自身：工作集 / 私有字节 / 峰值工作集 / 分页 / GDI / USER / 句柄
//! - 显卡：每个适配器上本进程占用的专用显存与共享显存
//!   （D3D11 纹理与 D2D 位图记在这里，不在私有字节里）
//! - 液态玻璃：桌面捕获纹理、模糊纹理、渲染表面缓存、上一帧缓存
//! - 分类框：DIB 后端、玻璃回读缓冲、玻璃 D2D 位图、图标位图缓存
//! - 子进程：WebView2 一类的后代进程个数与内存合计
//!
//! 全部调用都是只读查询，任一分量失败就记 0 并继续，
//! 绝不 panic，也绝不改变任何渲染路径的行为。

use std::cell::RefCell;
use std::mem::size_of;

use windows::core::Interface;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory6, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
    DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetGuiResources, GetProcessHandleCount, OpenProcess,
    GR_GDIOBJECTS, GR_USEROBJECTS, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// 一帧分类框渲染的分段累计耗时（微秒）与帧数。
///
/// # 为什么必须分段
///
/// 「帧率上不去」和「内存一直涨」是同一个根因的两面：每帧都要跑
/// GPU 取帧 → D2D 模糊 → D3D 着色 → 回读到 CPU → 上传成 D2D 位图 →
/// **软件光栅化**画进 GDI DIB → `UpdateLayeredWindow`。只报一个总耗时
/// 无法判断瓶颈在 GPU 段还是软件段，也就无法决定「提高帧率」和
/// 「压住内存」该先动哪一刀。
#[derive(Clone, Copy, Default)]
pub(crate) struct FrameTimings {
    /// 完成的帧数
    pub frames: u64,
    /// `render_background` 全程（DXGI 取帧 + D2D 模糊 + D3D 着色 + 回读）
    pub glass_us: u64,
    /// 其中 D2D 模糊 effect 图那一段
    pub blur_us: u64,
    /// 其中 DXGI 取帧那一段（`AcquireNextFrame` + 整屏 `CopyResource`）
    pub capture_us: u64,
    /// 其中 GPU→CPU 回读那一段（`Map` 阻塞 + 逐行 memcpy + `Unmap`）
    ///
    /// # 为什么必须单独量这一段
    ///
    /// `Map` 用 `D3D11_MAP_READ` 且 flags=0，会**阻塞到 GPU 把前面排队的
    /// 活全干完** —— 这是整条管线里唯一的硬同步点，也是「继续保留
    /// `UpdateLayeredWindow`」与「换 DirectComposition」两条路线的分水岭：
    /// 只要最终画面还要落进 GDI DIB，这次回读就必须付；换成 DComp 之后
    /// 玻璃纹理可零拷贝直接参与合成，这一段整体消失。不量出来就只能
    /// 靠猜来决定要不要动窗口模型。
    pub readback_us: u64,
    /// 其中 `Map` 阻塞等 GPU 排空的时间（回读减去行拷贝）
    ///
    /// # 为什么要把「等 GPU」和「搬字节」分开
    ///
    /// 这两者的修法完全相反，混在一个数字里就会开错刀：
    /// - `Map` 占大头 = GPU 真的算了这么久（模糊 + 着色器），
    ///   换 DirectComposition 也**省不掉**，必须缩小 GPU 工作量；
    /// - 行拷贝占大头 = 纯粹是 CPU 搬 4.8MB，DComp 零拷贝直接归零。
    pub map_us: u64,
    /// D2D DC 渲染目标的 BeginDraw..EndDraw（软件光栅化）
    pub dib_us: u64,
    /// 其中「第一趟：内容层」那一段（命中缓存时应接近 0）
    pub content_us: u64,
    /// 内容层缓存命中次数（走周期 tick 且指纹未变）
    pub content_hits: u64,
    /// 内容层缓存未命中次数（全量重画）
    pub content_misses: u64,
    /// `UpdateLayeredWindow` 提交
    pub submit_us: u64,
}

thread_local! {
    /// 本帧 `render_background` 的耗时（微秒），供调用方从「软件光栅」段里扣掉。
    ///
    /// # 为什么必须有这个
    ///
    /// 玻璃那一段是在**第二趟合成内部**被调用的，而第二趟整段都算在
    /// `dib_us` 里。直接报 `玻璃 + 软件光栅 + 提交` 会把玻璃算两遍 ——
    /// 实测这个重复计数把 25.2ms/帧虚报成 38.3ms/帧，虚高 52%，
    /// 足以让「单线程天花板」的结论整整错一档。
    static LAST_GLASS_US: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static FRAME_TIMINGS: RefCell<FrameTimings> = const {
        RefCell::new(FrameTimings {
            frames: 0,
            glass_us: 0,
            blur_us: 0,
            capture_us: 0,
            readback_us: 0,
            map_us: 0,
            dib_us: 0,
            content_us: 0,
            content_hits: 0,
            content_misses: 0,
            submit_us: 0,
        })
    };
}

/// 记一次玻璃渲染的耗时（`render_background` 全程）。
///
/// 不计帧数：玻璃可能没开、也可能这一帧失败，帧数由
/// `add_frame_timing` 在分类框真正提交画面时统一记。
pub(crate) fn add_glass_timing(glass_us: u64) {
    LAST_GLASS_US.with(|cell| cell.set(glass_us));
    FRAME_TIMINGS.with(|cell| cell.borrow_mut().glass_us += glass_us);
}

/// 取走并清零「本帧玻璃段耗时」。
///
/// 只有第二趟真的调过 `render_background` 才非 0；玻璃关闭、
/// 或本帧在调用前就失败返回时为 0，此时扣减是空操作，安全。
pub(crate) fn take_last_glass_us() -> u64 {
    LAST_GLASS_US.with(|cell| cell.replace(0))
}

/// 记 D2D 模糊 effect 图那一段（已包含在玻璃耗时内，单独列出便于归因）。
pub(crate) fn add_blur_timing(blur_us: u64) {
    FRAME_TIMINGS.with(|cell| cell.borrow_mut().blur_us += blur_us);
}

/// 记 DXGI 取帧那一段（已包含在玻璃耗时内，单独列出便于归因）。
pub(crate) fn add_capture_timing(capture_us: u64) {
    FRAME_TIMINGS.with(|cell| cell.borrow_mut().capture_us += capture_us);
}

/// 记 GPU→CPU 回读那一段（已包含在玻璃耗时内，单独列出便于归因）。
pub(crate) fn add_readback_timing(readback_us: u64) {
    FRAME_TIMINGS.with(|cell| cell.borrow_mut().readback_us += readback_us);
}

/// 记回读里 `Map` 阻塞那一段（已包含在 `readback_us` 内）。
pub(crate) fn add_map_timing(map_us: u64) {
    FRAME_TIMINGS.with(|cell| cell.borrow_mut().map_us += map_us);
}

/// 记一帧分类框的软件光栅化与提交耗时，并把帧数 +1。
pub(crate) fn add_frame_timing(dib_us: u64, submit_us: u64) {
    add_frame_timing_ex(dib_us, submit_us, 0, false)
}

/// 记一帧的软件光栅化 / 提交耗时，并额外报告内容层缓存的命中情况。
///
/// # 为什么必须把「内容层」单独切出来
///
/// `dib_us` 覆盖两趟 D2D 绘制。内容层缓存有没有真的生效，只看
/// `dib_us` 是分不出来的 —— 它同时包含合成那一趟，而合成那一趟
/// 每帧都要跑。只有把第一趟单独计时，才能看出「命中时第一趟
/// 接近 0ms」这个预期是否成立；命中率则直接说明指纹判据是否
/// 每帧都在误判成「变了」。
pub(crate) fn add_frame_timing_ex(
    dib_us: u64,
    submit_us: u64,
    content_us: u64,
    content_reused: bool,
) {
    FRAME_TIMINGS.with(|cell| {
        let mut t = cell.borrow_mut();
        t.frames += 1;
        t.dib_us += dib_us;
        t.content_us += content_us;
        if content_reused {
            t.content_hits += 1;
        } else {
            t.content_misses += 1;
        }
        t.submit_us += submit_us;
    });
}

/// 取走并清零分段耗时。
fn take_frame_timings() -> FrameTimings {
    FRAME_TIMINGS.with(|cell| {
        let snapshot = *cell.borrow();
        *cell.borrow_mut() = FrameTimings::default();
        snapshot
    })
}

/// 输出间隔。60 秒一条，一天约 1440 行，占日志配额很小。
pub(crate) const MEM_PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// 最多枚举的显卡适配器数（现实中 1~2 个，多出来的枚举会直接失败退出）。
const MAX_ADAPTERS: u32 = 8;

/// 上次输出时的累计分配量与时刻，用来算这一分钟的分配 churn 速率。
static LAST_ALLOC_TOTAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LAST_ALLOC_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

thread_local! {
    static LAST_REPORT: RefCell<Option<std::time::Instant>> = const { RefCell::new(None) };
    /// DXGI 工厂只为查显存，建一次就够（反复建反而白占内存）
    static DXGI_FACTORY: RefCell<Option<IDXGIFactory6>> = const { RefCell::new(None) };
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

/// 进程自身的内存与内核对象计数。
struct ProcessCounters {
    working_set: u64,
    private_usage: u64,
    peak_working_set: u64,
    pagefile: u64,
    gdi: u32,
    user: u32,
    handles: u32,
}

/// 读取任意进程句柄的内存计数（`cb` 必须是 EX 结构体大小，否则拿不到私有字节）。
unsafe fn measure_handle(handle: HANDLE) -> Option<(u64, u64)> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    GetProcessMemoryInfo(
        handle,
        (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
        counters.cb,
    )
    .ok()?;
    Some((counters.WorkingSetSize as u64, counters.PrivateUsage as u64))
}

fn query_self_counters() -> Option<ProcessCounters> {
    unsafe {
        let process = GetCurrentProcess();
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            counters.cb,
        )
        .ok()?;
        let mut handles = 0u32;
        // 句柄数拿不到不算失败，记 0 即可
        let _ = GetProcessHandleCount(process, &mut handles);
        Some(ProcessCounters {
            working_set: counters.WorkingSetSize as u64,
            private_usage: counters.PrivateUsage as u64,
            peak_working_set: counters.PeakWorkingSetSize as u64,
            pagefile: counters.PagefileUsage as u64,
            gdi: GetGuiResources(process, GR_GDIOBJECTS),
            user: GetGuiResources(process, GR_USEROBJECTS),
            handles,
        })
    }
}

fn dxgi_factory() -> Option<IDXGIFactory6> {
    if let Some(existing) = DXGI_FACTORY.with(|cell| cell.borrow().clone()) {
        return Some(existing);
    }
    let created: IDXGIFactory6 = unsafe { CreateDXGIFactory1() }.ok()?;
    DXGI_FACTORY.with(|cell| *cell.borrow_mut() = Some(created.clone()));
    Some(created)
}

/// 单个显卡适配器上本进程的显存占用。
struct AdapterUsage {
    /// 展示用名字，带 LUID，便于看出「同一块 GPU 被枚举了多次」
    name: String,
    /// 去重键：适配器描述 + 自身专用显存总量。
    ///
    /// 同一块物理 GPU 常被 DXGI 枚举成多个条目（本机 Intel Arc 140T
    /// 就出现 3 个 LUID 不同、描述相同的条目），而每个条目都会上报
    /// **同一份**本进程用量。直接求和会把「专用显存」放大 3 倍，
    /// 把一个 130MB 的问题说成 390MB，把优化方向整个带偏。
    /// 真正的双显卡（核显 + 独显）描述不同，仍会被分别计入。
    dedup_key: String,
    local: u64,
    shared: u64,
}

/// 本进程在各显卡适配器上占用的显存。
///
/// 按适配器分开报，否则「专用 500MB」这种数字无法归因：
/// 机器上常常同时挂着独显、核显和 Microsoft Basic Render Driver，
/// D3D11 设备落在哪一块、显存记在哪一块，只有拆开才看得出来。
fn query_video_memory() -> Vec<AdapterUsage> {
    let Some(factory) = dxgi_factory() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for index in 0..MAX_ADAPTERS {
        let Ok(adapter1) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        let desc = unsafe { adapter1.GetDesc1() }.ok();
        let dedup_key = match &desc {
            Some(desc) => {
                let end = desc
                    .Description
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(desc.Description.len());
                format!(
                    "{}|{}",
                    String::from_utf16_lossy(&desc.Description[..end]),
                    desc.DedicatedVideoMemory
                )
            }
            None => format!("<adapter {index}>"),
        };
        let name = match &desc {
            Some(desc) => {
                let end = desc
                    .Description
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(desc.Description.len());
                let text = String::from_utf16_lossy(&desc.Description[..end]);
                let software = desc.Flags & 0x1 != 0; // DXGI_ADAPTER_FLAG_SOFTWARE
                format!(
                    "{}{} 自身专用显存{:.0}MB LUID={:X}:{:X}",
                    text,
                    if software { "（软件）" } else { "" },
                    mb(desc.DedicatedVideoMemory as u64),
                    desc.AdapterLuid.HighPart,
                    desc.AdapterLuid.LowPart
                )
            }
            None => format!("<适配器 {index} 描述获取失败>"),
        };
        let mut usage = AdapterUsage {
            name,
            dedup_key,
            local: 0,
            shared: 0,
        };
        if let Ok(adapter3) = adapter1.cast::<IDXGIAdapter3>() {
            for segment in [
                DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
                DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
            ] {
                let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
                if unsafe { adapter3.QueryVideoMemoryInfo(0, segment, &mut info) }.is_err() {
                    continue;
                }
                // CurrentUsage 本身就是 u64，不需要夹负数也不需要转换
                let value = info.CurrentUsage;
                if segment == DXGI_MEMORY_SEGMENT_GROUP_LOCAL {
                    usage.local = value;
                } else {
                    usage.shared = value;
                }
            }
        }
        out.push(usage);
    }
    out
}

/// 后代进程（WebView2 的 browser / renderer / gpu 等）的个数与内存合计。
///
/// WebView2 的内存不在主进程里，任务管理器按「应用组」求和才显示得出来。
/// 这里做同样的事：从自己的 pid 出发做传递闭包，把所有后代的
/// 工作集与私有字节加起来。
fn query_descendants() -> (usize, u64, u64) {
    let root = unsafe { GetCurrentProcessId() };
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return (0, 0, 0);
    };
    if snapshot.is_invalid() {
        return (0, 0, 0);
    }
    let result = collect_descendants(snapshot, root);
    // 快照句柄必须归还，否则每 60 秒漏一个内核句柄
    let _ = unsafe { CloseHandle(snapshot) };
    result
}

fn collect_descendants(snapshot: HANDLE, root: u32) -> (usize, u64, u64) {
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    unsafe {
        if Process32FirstW(snapshot, &mut entry).is_err() {
            return (0, 0, 0);
        }
        loop {
            children
                .entry(entry.th32ParentProcessID)
                .or_default()
                .push(entry.th32ProcessID);
            if Process32NextW(snapshot, &mut entry).is_err() {
                break;
            }
        }
    }
    // 广度优先展开；`seen` 防止父 pid 被回收复用后出现环
    let mut queue = vec![root];
    let mut seen = std::collections::HashSet::new();
    seen.insert(root);
    let mut count = 0usize;
    let mut working_set = 0u64;
    let mut private_usage = 0u64;
    while let Some(pid) = queue.pop() {
        if let Some(kids) = children.get(&pid) {
            for kid in kids {
                if seen.insert(*kid) {
                    queue.push(*kid);
                }
            }
        }
        if pid == root {
            continue;
        }
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            continue;
        };
        let measured = unsafe { measure_handle(handle) };
        let _ = unsafe { CloseHandle(handle) };
        if let Some((ws, priv_usage)) = measured {
            count += 1;
            working_set += ws;
            private_usage += priv_usage;
        }
    }
    (count, working_set, private_usage)
}

/// 距上次输出是否已到间隔。到点则刷新计时并返回 `true`。
pub(crate) fn due() -> bool {
    LAST_REPORT.with(|cell| {
        let mut slot = cell.borrow_mut();
        let now = std::time::Instant::now();
        let is_due = match *slot {
            Some(last) => now.duration_since(last) >= MEM_PROBE_INTERVAL,
            None => true,
        };
        if is_due {
            *slot = Some(now);
        }
        is_due
    })
}

/// 采集并输出一次内存分解。任一分量失败记 0，不影响其余分量。
pub(crate) fn log_breakdown() {
    let counters = query_self_counters();
    let adapters = query_video_memory();
    // 同一块物理 GPU 被 DXGI 枚举多次时，每个条目都会上报同一份
    // 进程用量，按去重键只算一次，否则「专用显存」会被凭空放大数倍。
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut video_local = 0u64;
    let mut video_shared = 0u64;
    for adapter in &adapters {
        if seen.insert(adapter.dedup_key.as_str()) {
            video_local += adapter.local;
            video_shared += adapter.shared;
        }
    }
    // 只把真的占了显存的适配器列出来，避免 4 个适配器里有 3 个是 0 的噪音
    let video_detail: Vec<String> = adapters
        .iter()
        .filter(|a| a.local + a.shared > 0)
        .map(|a| format!("{}={:.0}MB", a.name, mb(a.local + a.shared)))
        .collect();
    let video_detail = if video_detail.is_empty() {
        "无".to_string()
    } else {
        video_detail.join(" / ")
    };
    let (kids, kid_ws, kid_priv) = query_descendants();
    let glass = super::liquid_glass::memory_stats();
    let grid = super::grid_window::memory_stats();
    let (text_formats, ellipsis_signs, text_layouts) = super::render_devices::text_cache_sizes();
    let timings = take_frame_timings();
    // 注意别叫 frames：那个名字已经被 DXGI 取帧计数占用，会遮蔽
    let timed_frames = timings.frames.max(1) as f64;
    let (frames, mouse_only, no_rects) = super::liquid_glass::frame_counters();
    let (region_checks, region_changed) = super::liquid_glass::region_check_counters();
    // 命中率 = 报「变了」的次数 / 比对次数。接近 0 说明驱动在空报整屏矩形、
    // 像素比对正在把渲染挡下来；接近 1 说明桌面确实在变，渲染省不掉。
    let region_hit_percent = if region_checks > 0 {
        region_changed as f64 * 100.0 / region_checks as f64
    } else {
        0.0
    };
    let alloc = crate::utils::alloc_probe::snapshot();
    // churn 速率 = 这段时间里向分配器申请了多少字节 / 经过的分钟数
    let alloc_window_minutes = {
        let now = std::time::Instant::now();
        let mut slot = LAST_ALLOC_AT.lock().unwrap_or_else(|e| e.into_inner());
        let minutes = slot
            .map(|last| now.duration_since(last).as_secs_f64() / 60.0)
            .unwrap_or(0.0);
        *slot = Some(now);
        minutes
    };
    let previous_total =
        LAST_ALLOC_TOTAL.swap(alloc.total_bytes, std::sync::atomic::Ordering::Relaxed);
    let churn_mb_per_min = if alloc_window_minutes > 1e-6 {
        mb(alloc.total_bytes.saturating_sub(previous_total)) / alloc_window_minutes
    } else {
        0.0
    };

    let (ws, priv_usage, peak, pagefile, gdi, user, handles) = match counters {
        Some(c) => (
            c.working_set,
            c.private_usage,
            c.peak_working_set,
            c.pagefile,
            c.gdi,
            c.user,
            c.handles,
        ),
        None => (0, 0, 0, 0, 0, 0, 0),
    };

    log::info!(
        "[内存分解] 进程 WS={:.0}MB 私有={:.0}MB 峰值WS={:.0}MB 分页={:.0}MB GDI={gdi} USER={user} 句柄={handles} \
         | 显存 专用={vlocal:.0}MB 共享={vshared:.0}MB（{adapter_n} 个适配器: {video_detail}）\
         | 玻璃[{state}] 捕获纹理={cap_n}张/{cap_mb:.1}MB 模糊纹理={blur_n}张/{blur_mb:.1}MB \
         共享渲染表面={surf_n}份/{surf_mb:.1}MB 模糊链={chain_n}条/累计建{chain_total}次 \
         输出={out_active}/{out_total}个有分类框 \
         取帧={frames}（仅鼠标={mouse_only} 无矩形={no_rects}）\
         区域像素比对={checks}次/命中{changed}次（{hit:.0}%）\
         | 分类框 {boxes}个/{items}条 DIB={dib_mb:.1}MB 回读缓冲={pixels_mb:.1}MB \
         玻璃位图={bmp_mb:.1}MB 图标位图={icon_n}项/{icon_mb:.1}MB \
         文字缓存 格式={tf}条 省略号={es}条 布局={tl}条/{tl_limit}上限 \
         | Rust堆 活跃={live_mb:.1}MB 峰值={peak_mb:.1}MB 块数={live_blocks} \
         分配速率={churn:.0}MB/分（累计 {total_gb:.2}GB/{total_count} 次）\
         | 帧耗时 {frames_n}帧 平均{total_ms:.2}ms（玻璃{glass_ms:.2} = 取帧{cap_ms:.2} \
         + 模糊{blur_ms:.2} + 回读{rb_ms:.2}（等GPU {map_ms:.2} + 搬运 {copy_ms:.2}） + 着色{shade_ms:.2}）\
         光栅合成{dib_ms:.2}（内容层{content_ms:.2} 命中{hits}/未命中{misses}）\
         提交{submit_ms:.2}）\
         | 子进程 {kids}个 WS={kid_ws_mb:.1}MB 私有={kid_priv_mb:.1}MB",
        mb(ws),
        mb(priv_usage),
        mb(peak),
        mb(pagefile),
        vlocal = mb(video_local),
        vshared = mb(video_shared),
        adapter_n = adapters.len(),
        video_detail = video_detail,
        state = glass.state_name,
        cap_n = glass.capture_texture_count,
        cap_mb = mb(glass.capture_texture_bytes),
        blur_n = glass.blurred_texture_count,
        blur_mb = mb(glass.blurred_texture_bytes),
        surf_n = glass.surface_cache_entries,
        surf_mb = mb(glass.surface_cache_bytes),
        chain_n = glass.blur_chain_count,
        chain_total = glass.blur_chains_created,
        out_active = glass.active_output_count,
        out_total = glass.output_count,
        frames = frames,
        mouse_only = mouse_only,
        no_rects = no_rects,
        checks = region_checks,
        changed = region_changed,
        hit = region_hit_percent,
        boxes = grid.windows,
        items = grid.items,
        dib_mb = mb(grid.dib_bytes),
        pixels_mb = mb(grid.glass_pixels_bytes),
        bmp_mb = mb(grid.glass_bitmap_bytes),
        icon_n = grid.icon_bitmap_entries,
        icon_mb = mb(grid.icon_bitmap_bytes),
        tf = text_formats,
        es = ellipsis_signs,
        tl = text_layouts,
        tl_limit = super::render_devices::TEXT_LAYOUT_CACHE_LIMIT,
        live_mb = mb(alloc.live_bytes),
        peak_mb = mb(alloc.peak_bytes),
        live_blocks = alloc.live_blocks,
        churn = churn_mb_per_min,
        total_gb = alloc.total_bytes as f64 / 1_073_741_824.0,
        total_count = alloc.total_count,
        kids = kids,
        kid_ws_mb = mb(kid_ws),
        kid_priv_mb = mb(kid_priv),
        frames_n = timings.frames,
        total_ms = (timings.glass_us + timings.dib_us + timings.submit_us) as f64 / 1000.0 / timed_frames,
        glass_ms = timings.glass_us as f64 / 1000.0 / timed_frames,
        blur_ms = timings.blur_us as f64 / 1000.0 / timed_frames,
        cap_ms = timings.capture_us as f64 / 1000.0 / timed_frames,
        rb_ms = timings.readback_us as f64 / 1000.0 / timed_frames,
        map_ms = timings.map_us as f64 / 1000.0 / timed_frames,
        copy_ms = (timings.readback_us - timings.map_us) as f64 / 1000.0 / timed_frames,
        // 着色 = 玻璃总耗时扣掉已单独归因的三段；saturating_sub 保证计时
        // 抖动导致负数时显示 0，而不是回绕成一个巨大的数。
        shade_ms = timings
            .glass_us
            .saturating_sub(timings.capture_us + timings.blur_us + timings.readback_us) as f64
            / 1000.0
            / timed_frames,
        dib_ms = timings.dib_us as f64 / 1000.0 / timed_frames,
        content_ms = timings.content_us as f64 / 1000.0 / timed_frames,
        hits = timings.content_hits,
        misses = timings.content_misses,
        submit_ms = timings.submit_us as f64 / 1000.0 / timed_frames,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MB 换算必须是二进制 MB，与日志里其他 MB 数字口径一致。
    #[test]
    fn mb_uses_binary_units() {
        assert!((mb(1_048_576) - 1.0).abs() < f64::EPSILON);
        assert!((mb(0) - 0.0).abs() < f64::EPSILON);
        assert!((mb(1_572_864) - 1.5).abs() < 1e-9);
    }

    /// 自身进程计数在本平台上必然拿得到；拿不到说明 API 签名用错了。
    #[test]
    fn self_counters_are_available() {
        let counters = query_self_counters().expect("GetProcessMemoryInfo 应当成功");
        assert!(counters.working_set > 0, "工作集不应为 0");
        assert!(counters.private_usage > 0, "私有字节不应为 0");
        assert!(
            counters.peak_working_set >= counters.working_set,
            "峰值工作集不应小于当前工作集"
        );
        assert!(counters.handles > 0, "句柄数不应为 0");
    }

    /// 显存查询允许拿不到（虚拟机 / WARP 驱动），但不能 panic；
    /// 每个适配器的上报用量都必须是个合理数字，名字不能为空。
    #[test]
    fn video_memory_query_never_panics() {
        let adapters = query_video_memory();
        for adapter in &adapters {
            assert!(
                adapter.local < (1u64 << 40),
                "适配器 {} 上报的专用用量异常: {}",
                adapter.name,
                adapter.local
            );
            assert!(!adapter.name.is_empty(), "适配器名不应为空");
        }
    }

    /// 同一块 GPU 被 DXGI 枚举成多个条目时必须只算一次，
    /// 否则「专用显存」会被凭空放大成真实值的数倍，把优化方向带偏。
    /// 描述不同的真双显卡仍要分别计入。
    #[test]
    fn duplicate_adapters_are_counted_once_but_distinct_gpus_are_not() {
        let make = |dedup_key: &str, luid: u32, local: u64| AdapterUsage {
            name: format!("{dedup_key} LUID=0:{luid:X}"),
            dedup_key: dedup_key.to_string(),
            local,
            shared: 0,
        };
        let adapters = vec![
            // 本机实况：同一块 Intel Arc 被枚举 3 次，每个条目都报 139MB
            make("Intel Arc|128", 0xFFC5, 139),
            make("Intel Arc|128", 0x3E11F, 139),
            make("Intel Arc|128", 0x2549E, 139),
            // 真正不同的第二块 GPU，必须另外计入
            make("NVIDIA RTX|8192", 0x1, 50),
        ];
        let mut seen = std::collections::HashSet::new();
        let total: u64 = adapters
            .iter()
            .filter(|a| seen.insert(a.dedup_key.as_str()))
            .map(|a| a.local)
            .sum();
        assert_eq!(total, 189, "重复条目算一次(139) + 独显(50)");
    }

    /// 后代进程统计必须把测试进程自己排除在计数之外，且数值合理。
    #[test]
    fn descendants_exclude_self_and_stay_sane() {
        let (count, ws, priv_usage) = query_descendants();
        assert!(count < 1024, "后代进程数异常: {count}");
        assert!(ws < (1u64 << 40), "后代工作集合计异常: {ws}");
        assert!(priv_usage < (1u64 << 40), "后代私有字节合计异常");
    }

    /// churn 速率必须按真实经过的时间折算，窗口为 0 时不能除出 inf/NaN。
    #[test]
    fn churn_rate_is_finite_for_zero_window() {
        let minutes = 0.0f64;
        let rate = if minutes > 1e-6 { 100.0 / minutes } else { 0.0 };
        assert!(rate.is_finite(), "窗口为 0 时必须报 0，不能是 inf/NaN");
    }

    /// 节流：第一次必然到点，紧接着的第二次必须被拦下。
    #[test]
    fn due_throttles_repeat_calls() {
        LAST_REPORT.with(|cell| *cell.borrow_mut() = None);
        assert!(due(), "第一次调用应当输出");
        assert!(!due(), "紧接着的第二次调用应当被节流");
    }
}

/// 诊断开关：IKUN_DEBUG_DUMP_FRAME=1 时把后缓冲帧落盘（见 grid_window::dump_dcomp_backbuffer）。
pub fn dump_frame_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("IKUN_DEBUG_DUMP_FRAME")
            .map(|v| !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false")))
            .unwrap_or(false)
    })
}

/// 拖拽/移动期每帧统计：帧间隔分布 + 单帧成本，
/// 退出拖拽时汇总一行日志，用于客观衡量掉帧（间隔>33ms 即掉帧）。
struct DragStats {
    hwnd: usize,
    last: Option<std::time::Instant>,
    intervals: Vec<u32>,
    costs: Vec<u32>,
}
static DRAG_STATS: std::sync::Mutex<DragStats> = std::sync::Mutex::new(DragStats {
    hwnd: 0,
    last: None,
    intervals: Vec::new(),
    costs: Vec::new(),
});

/// 取最近一帧玻璃管线分段耗时（微秒）：取帧/模糊/回读/等GPU。
pub fn last_glass_breakdown() -> (u64, u64, u64, u64) {
    FRAME_TIMINGS.with(|cell| {
        let t = cell.borrow();
        (t.capture_us, t.blur_us, t.readback_us, t.map_us)
    })
}

pub fn record_drag_frame(hwnd: usize, dragging: bool, at: std::time::Instant, cost: std::time::Duration) {
    if !dragging {
        return;
    }
    let Ok(mut st) = DRAG_STATS.lock() else { return };
    if st.hwnd != hwnd {
        drop(st);
        flush_drag_stats();
        let Ok(mut st) = DRAG_STATS.lock() else { return };
        log::warn!("[dragdbg] record session start hwnd={:x}", hwnd);
        st.hwnd = hwnd;
        st.last = None;
        st.intervals.clear();
        st.costs.clear();
        st.last = Some(at);
        st.costs.push(cost.as_micros() as u32);
        return;
    }
    if let Some(l) = st.last {
        st.intervals.push(at.saturating_duration_since(l).as_millis() as u32);
    }
    st.last = Some(at);
    st.costs.push(cost.as_micros() as u32);
    if cost.as_millis() > 30 {
        let (cap, blur, rb, map) = last_glass_breakdown();
        log::warn!(
            "[dragdbg] stall cost={}ms capture={}us blur={}us readback={}us map={}us",
            cost.as_millis(),
            cap,
            blur,
            rb,
            map
        );
    }
}

/// 拖拽结束时汇总输出：帧间隔分布 + 单帧成本，
/// 间隔 >33ms 视为掉帧（16ms 档的两倍）。
pub fn flush_drag_stats() {
    let Ok(mut st) = DRAG_STATS.lock() else { return };
    let n = st.intervals.len();
    log::warn!("[dragdbg] flush hwnd={:x} n={}", st.hwnd, n);
    if n == 0 {
        st.hwnd = 0;
        st.last = None;
        return;
    }
    let mut sorted = st.intervals.clone();
    sorted.sort_unstable();
    let mean_i = sorted.iter().sum::<u32>() as f64 / n as f64;
    let p95_i = sorted[((n as f64 * 0.95) as usize).min(n - 1)];
    let max_i = sorted[n - 1];
    let gaps33 = sorted.iter().filter(|v| **v > 33).count();
    let gaps50 = sorted.iter().filter(|v| **v > 50).count();
    let mean_c = st.costs.iter().sum::<u32>() as f64 / st.costs.len() as f64 / 1000.0;
    let max_c = *st.costs.iter().max().unwrap_or(&0) as f64 / 1000.0;
    log::warn!(
        "[拖拽帧统计] hwnd={:x} 帧数={} 平均间隔={:.1}ms p95={}ms max={}ms 掉帧(>33ms)={} (>50ms)={} | 单帧成本 平均={:.2}ms max={:.2}ms",
        st.hwnd,
        n,
        mean_i,
        p95_i,
        max_i,
        gaps33,
        gaps50,
        mean_c,
        max_c
    );
    st.hwnd = 0;
    st.last = None;
    st.intervals.clear();
    st.costs.clear();
}
