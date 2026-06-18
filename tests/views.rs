//! # Views — Coherence & Synchronization
//!
//! **View** is a mapping from location to timestamp, modelling a thread's
//! confirmed state of messages.  Three kinds of view constrain behaviour:
//!
//! | View | 机制 | 作用 |
//! |------|------|------|
//! | **Per-thread view** | 读写操作更新当前线程的 view | 保证 per-location coherence（RR/RW/WR/WW）|
//! | **Per-message view** | Release 写生成 message view；Acquire 读合并 message view | 实现 Release/Acquire 同步 |
//! | **Global view** | `fence(SC)` 同步 thread view 与 global view | 实现 SC fence 跨线程同步 |
//!
//! ## 对应文档
//!
//! - Per-thread View: lines 183–207
//! - Per-message View: lines 208–237
//! - Global View: lines 239–273

use loom::sync::atomic::{AtomicUsize, Ordering, fence};
use loom::sync::Arc;
use loom::thread;

// ═══════════════════════════════════════════════════════════════════════════════
//  Per-thread View → Coherence
// ═══════════════════════════════════════════════════════════════════════════════

/// **RR coherence**：两次读同一位置，前一次读到新值，后一次不能读到旧值。
///
/// 对应文档：`X=1 || r1=X; r2=X [r1=1, r2=0 impossible]`
///
/// 若线程2 第一次读到 X=1（来自线程1 的写入），其 per-thread view 已更新到
/// X 的最新 timestamp，第二次读不可能回退到 X=0。
#[test]
fn rr_coherence() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let t1 = thread::spawn(move || {
            x1.store(1, Ordering::Relaxed);
        });

        let x2 = x.clone();
        let r1c = r1.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            let v1 = x2.load(Ordering::Acquire);
            r1c.store(v1, Ordering::Relaxed);
            let v2 = x2.load(Ordering::Acquire);
            r2c.store(v2, Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        let v1 = r1.load(Ordering::Relaxed);
        let v2 = r2.load(Ordering::Relaxed);
        assert!(v1 != 1 || v2 == 1, "RR coherence: if first read sees 1, second must also see 1");
    });
}

/// **RW coherence**：读后写，读到的值一定在写之前。
///
/// 对应文档：`r=X; X=1 [r=0]`
///
/// 先读 X 得到 42（初始值），再写 X=100。读到的值不受后续写的影响。
#[test]
fn rw_coherence() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(42));
        let r = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let rc = r.clone();
        let t = thread::spawn(move || {
            let val = x1.load(Ordering::Relaxed);
            rc.store(val, Ordering::Relaxed);
            x1.store(100, Ordering::Relaxed);
        });

        t.join().unwrap();

        let val = r.load(Ordering::Relaxed);
        assert_eq!(val, 42, "RW coherence: read sees value before the write");
    });
}

/// **WR coherence**：写后读，读到的值一定是刚写的。
///
/// 对应文档：`X=1; r=X [r=1]`
///
/// 写入 X=42 后立即读同一个位置，per-thread view 保证了读到刚写入的值。
#[test]
fn wr_coherence() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let t = thread::spawn(move || {
            x1.store(42, Ordering::Relaxed);
            let val = x1.load(Ordering::Relaxed);
            assert_eq!(val, 42, "WR coherence: write then read sees the written value");
        });

        t.join().unwrap();
    });
}

/// **WW coherence**：写后写，最终结果一定是最后一个写。
///
/// 对应文档：`X=1; X=2 [X=2 at the end]`
///
/// 连续写入 X=1 和 X=2，per-thread view 保证最终 X=2（最后一次写入）。
#[test]
fn ww_coherence() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let t = thread::spawn(move || {
            x1.store(1, Ordering::Relaxed);
            x1.store(2, Ordering::Relaxed);
        });

        t.join().unwrap();

        assert_eq!(x.load(Ordering::Relaxed), 2, "WW coherence: final value is the last store");
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Per-message View → Release/Acquire Synchronization
// ═══════════════════════════════════════════════════════════════════════════════

/// **Release/Acquire 同步**：Release 写会生成一个 message view（记录 release
/// 时刻线程的完整视图）；Acquire 读将该 message view 合并到当前线程的 view 中。
///
/// 对应文档：
/// ```rust
/// X = 1;                        ||   if Y.load(acquire) == 1:
/// Y.store(1, release);          ||       assert!(X == 1);  // 一定成功
/// ```
///
/// 如果线程2 看到 Y=1（Acquire），则此前线程1 对 X=1 的写入必然也可见。
#[test]
fn release_acquire_sync() {
    loom::model(|| {
        let data = Arc::new(AtomicUsize::new(0));
        let flag = Arc::new(AtomicUsize::new(0));

        let d1 = data.clone();
        let f1 = flag.clone();
        let t1 = thread::spawn(move || {
            d1.store(42, Ordering::Relaxed);
            f1.store(1, Ordering::Release);
        });

        let d2 = data.clone();
        let f2 = flag.clone();
        let t2 = thread::spawn(move || {
            if f2.load(Ordering::Acquire) == 1 {
                let val = d2.load(Ordering::Relaxed);
                assert_eq!(val, 42, "per-message view: Acquire sees data from prior Release");
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Global View → SC Fence Synchronization
// ═══════════════════════════════════════════════════════════════════════════════

/// **SC Fence 同步**：`fence(SC)` 会将当前线程的 view 与 global view 合并为
/// 两者的最新值，使得即使使用 `Relaxed` 操作也能跨线程传递消息。
///
/// 对应文档：
/// ```rust
/// X = 1;              ||   if Y.load(relaxed) == 1 {
/// fence(SC);          ||       fence(SC);
/// Y.store(1, relaxed);||       assert!(X == 1);
///                     ||   }
/// ```
#[test]
fn sc_fence_sync() {
    loom::model(|| {
        let data = Arc::new(AtomicUsize::new(0));
        let flag = Arc::new(AtomicUsize::new(0));

        let d1 = data.clone();
        let f1 = flag.clone();
        let t1 = thread::spawn(move || {
            d1.store(42, Ordering::Relaxed);
            fence(Ordering::SeqCst);
            f1.store(1, Ordering::Relaxed);
        });

        let d2 = data.clone();
        let f2 = flag.clone();
        let t2 = thread::spawn(move || {
            if f2.load(Ordering::Relaxed) == 1 {
                fence(Ordering::SeqCst);
                let val = d2.load(Ordering::Relaxed);
                assert_eq!(val, 42, "global view: SC fence synchronises data across threads");
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
//  对照：不使用任何同步机制
// ═══════════════════════════════════════════════════════════════════════════════

/// 对照测试：全部使用 `Relaxed`，不做任何同步。
///
/// 线程2 看到 flag=1 后读 data，此时 data 的值可能是 0（旧值）也可能是 42。
/// 因为没有 Release/Acquire 或 SC fence，per-message view 和 global view
/// 都不会被更新，所以不存在 happens-before 关系——读旧值是合法行为。
#[test]
fn relaxed_no_sync() {
    loom::model(|| {
        let data = Arc::new(AtomicUsize::new(0));
        let flag = Arc::new(AtomicUsize::new(0));

        let d1 = data.clone();
        let f1 = flag.clone();
        let t1 = thread::spawn(move || {
            d1.store(42, Ordering::Relaxed);
            f1.store(1, Ordering::Relaxed);
        });

        let d2 = data.clone();
        let f2 = flag.clone();
        let t2 = thread::spawn(move || {
            if f2.load(Ordering::Relaxed) == 1 {
                let _val = d2.load(Ordering::Relaxed);
                // Without synchronization (no Release/Acquire or SC fence),
                // this thread may read stale data (val=0). This is allowed
                // under the relaxed memory model.
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}
