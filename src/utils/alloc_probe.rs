//! Rust 堆占用探针（全局分配器计数，只读诊断）
//!
//! # 为什么需要它
//!
//! `mem_probe` 把进程内存拆成了 GPU 纹理 / DIB / 回读缓冲 / 图标缓存等
//! 若干可归因的分量，但实测这些分量全部恒定不动时，进程私有字节仍以
//! ~1.3MB/分 的速度爬升。剩下的可能只有两类：**Rust 堆**，或者
//! D2D / D3D11 / 显卡用户态驱动的私有分配。这两者在任务管理器里
//! 是同一个数字，必须从分配器这一侧切开。
//!
//! 本模块用一个包在 `System` 外面的计数分配器给出三个关键量：
//!
//! - **活跃字节**：已分配未释放的总量。它涨 = Rust 侧真的在漏。
//! - **峰值字节**：活跃字节的历史高点。活跃远低于峰值 = 只是碎片/
//!   堆已提交容量没还回去，不是泄漏。
//! - **累计申请量**：从启动到现在一共向分配器要过多少字节。
//!   它除以运行时间就是**分配churn速率** —— 高频申请/归还会让
//!   Windows 堆的已提交容量单调上涨，这正是「每个分量都不涨、
//!   总内存却慢慢爬」的典型成因。
//!
//! 计数用 `Relaxed` 原子操作，x86 上每次分配多花约几十纳秒，
//! 对渲染帧率没有可测量影响。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// 当前已分配未释放的字节数
static LIVE_BYTES: AtomicI64 = AtomicI64::new(0);
/// 当前已分配未释放的块数
static LIVE_BLOCKS: AtomicI64 = AtomicI64::new(0);
/// 活跃字节的历史峰值
static PEAK_BYTES: AtomicI64 = AtomicI64::new(0);
/// 启动以来累计向分配器申请过的字节数（churn 指标，只增不减）
static TOTAL_BYTES: AtomicU64 = AtomicU64::new(0);
/// 启动以来累计分配次数
static TOTAL_COUNT: AtomicU64 = AtomicU64::new(0);

/// 堆占用快照。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocSnapshot {
    /// 已分配未释放的字节数
    pub live_bytes: u64,
    /// 已分配未释放的块数
    pub live_blocks: u64,
    /// 活跃字节历史峰值
    pub peak_bytes: u64,
    /// 累计申请字节数
    pub total_bytes: u64,
    /// 累计分配次数
    pub total_count: u64,
}

/// 取一次堆快照。
pub fn snapshot() -> AllocSnapshot {
    AllocSnapshot {
        live_bytes: LIVE_BYTES.load(Ordering::Relaxed).max(0) as u64,
        live_blocks: LIVE_BLOCKS.load(Ordering::Relaxed).max(0) as u64,
        peak_bytes: PEAK_BYTES.load(Ordering::Relaxed).max(0) as u64,
        total_bytes: TOTAL_BYTES.load(Ordering::Relaxed),
        total_count: TOTAL_COUNT.load(Ordering::Relaxed),
    }
}

/// 记账：活跃字节增加 `delta`（可为负），并把峰值抬高到新的活跃值。
fn account(delta: i64, churn: u64) {
    let live = LIVE_BYTES.fetch_add(delta, Ordering::Relaxed) + delta;
    LIVE_BLOCKS.fetch_add(if delta >= 0 { 1 } else { -1 }, Ordering::Relaxed);
    if churn > 0 {
        TOTAL_BYTES.fetch_add(churn, Ordering::Relaxed);
        TOTAL_COUNT.fetch_add(1, Ordering::Relaxed);
    }
    PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
}

/// 只计数、不改变分配行为的透传分配器。
pub struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            account(layout.size() as i64, layout.size() as u64);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        account(-(layout.size() as i64), 0);
        System.dealloc(ptr, layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            account(layout.size() as i64, layout.size() as u64);
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // realloc 可能原地扩展，也可能另分配一块；两种情况下
        // 「活跃字节」的净变化都是 new_size - layout.size()，
        // 块数不变，因此这里不动 LIVE_BLOCKS。
        let new_ptr = System.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            let delta = new_size as i64 - layout.size() as i64;
            LIVE_BYTES.fetch_add(delta, Ordering::Relaxed);
            let live = LIVE_BYTES.load(Ordering::Relaxed);
            PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
            if new_size > layout.size() {
                TOTAL_BYTES.fetch_add((new_size - layout.size()) as u64, Ordering::Relaxed);
                TOTAL_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全局计数器是进程内共享的：并行跑的其他测试也在分配/释放，
    /// 单次采样的差值会被干扰。用有限重试把干扰窗口过掉，
    /// 避免把并发误报成记账错误。
    fn balanced_attempt() -> u8 {
        let before = snapshot();
        let boxed = Box::new([7u8; 4096]);
        // 指针不逃逸时 release 优化会把 alloc/dealloc 折叠掉，
        // 计数器一动不动；black_box 把分配钉在原地。
        std::hint::black_box(&boxed);
        let during = snapshot();
        let mut mask = 0u8;
        if !(during.live_bytes >= before.live_bytes + 4096) {
            mask |= 1;
        }
        if !(during.total_bytes > before.total_bytes) {
            mask |= 2;
        }
        drop(boxed);
        let after = snapshot();
        if after.live_bytes != before.live_bytes {
            mask |= 4;
        }
        if after.live_blocks != before.live_blocks {
            mask |= 8;
        }
        mask
    }

    /// 分配一块并立刻释放，活跃字节必须回到原值 —— 否则记账有偏差，
    /// 报出来的「Rust 堆活跃」就是错的，会把优化方向带偏。
    #[test]
    fn alloc_dealloc_is_balanced() {
        let mut counts = [0u32; 4];
        for _ in 0..500 {
            let mask = balanced_attempt();
            if mask == 0 {
                return;
            }
            for bit in 0..4 {
                if mask & (1 << bit) != 0 {
                    counts[bit] += 1;
                }
            }
            std::hint::spin_loop();
        }
        panic!(
            "记账验证失败 500 次: 位1(live未涨)={} 位2(total未涨)={} 位3(live未回落)={} 位4(blocks未回落)={}",
            counts[0], counts[1], counts[2], counts[3]
        );
    }

    fn realloc_attempt() -> bool {
        let before = snapshot();
        let mut v: Vec<u64> = Vec::with_capacity(16);
        for i in 0..4096u64 {
            v.push(i);
        }
        std::hint::black_box(&v);
        let during = snapshot();
        let grew = during.live_bytes > before.live_bytes;
        drop(v);
        let after = snapshot();
        grew && after.live_bytes == before.live_bytes && after.live_blocks == before.live_blocks
    }

    /// Vec 扩容走 realloc，活跃字节必须跟着变，且不能重复计块数。
    #[test]
    fn realloc_moves_live_bytes_without_double_counting_blocks() {
        for _ in 0..500 {
            if realloc_attempt() {
                return;
            }
            std::hint::spin_loop();
        }
        panic!("多次重试仍被并发测试干扰，无法验证 realloc 记账");
    }

    /// 峰值必须始终不小于当前活跃值。
    ///
    /// 并行测试可能在两次原子读之间释放内存，重试消除窗口。
    #[test]
    fn peak_is_never_below_live() {
        for _ in 0..500 {
            let s = snapshot();
            if s.peak_bytes >= s.live_bytes {
                return;
            }
            std::hint::spin_loop();
        }
        panic!("多次重试仍观察到峰值低于活跃值");
    }
}
