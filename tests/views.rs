//! # Views — Coherence & Synchronization
//!
//! View 是 location 到 timestamp 的映射，表示线程对 message 的确认状态。有三种 view：
//!
//! | View | 机制 | 作用 |
//! |------|------|------|
//! | **Per-thread view** | 表示线程对 message 的确认，读写操作更新当前线程的 view | 保证 per-location coherence（RR/RW/WR/WW）|
//! | **Per-message view** | Release store 生成 message view；Acquire load 合并 message view | 实现 Release/Acquire 同步 |
//! | **Global view** | fence(SC) 同步 thread view 与 global view | 实现 SC fence 跨线程同步 |

use relaxed_memory_concurrency::test::loom;
use relaxed_memory_concurrency::test::loom::sync::Arc;
use relaxed_memory_concurrency::test::loom::sync::atomic::{AtomicUsize, Ordering, fence};
use relaxed_memory_concurrency::test::loom::thread;

// ═══════════════════════════════════════════════════════════════════════════════
//  Per-thread View → Coherence
// ═══════════════════════════════════════════════════════════════════════════════

/// **RR coherence**：两次读同一位置，前一次读到新值，后一次不能读到旧值。
///
/// 对应文档：`X=1 || r1=X; r2=X [r1=1, r2=0 impossible]`
#[test]
fn rr_coherence() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));

        let x_ = Arc::clone(&x);
        let t1 = thread::spawn(move || {
            x_.store(1, Ordering::Relaxed);
        });

        let x_ = Arc::clone(&x);
        let t2 = thread::spawn(move || {
            if x_.load(Ordering::Relaxed) == 1 {
                assert_eq!(x_.load(Ordering::Relaxed), 1);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}

/// **RW coherence**：读后写，读到的值一定在写之前。
///
/// 对应文档：`r=X; X=1 [r=0]`
///
/// 先读 X 得到 0（初始值），再写 X=1。读到的值不受后续写的影响。
#[test]
fn rw_coherence() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));

        thread::spawn(move || {
            assert_eq!(x.load(Ordering::Relaxed), 0);
            x.store(1, Ordering::Relaxed);
        })
        .join()
        .unwrap();
    });
}

/// **WR coherence**：写后读，读到的值一定是刚写的。
///
/// 对应文档：`X=1; r=X [r=1]`
#[test]
fn wr_coherence() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));

        thread::spawn(move || {
            x.store(1, Ordering::Relaxed);
            assert_eq!(x.load(Ordering::Relaxed), 1);
        })
        .join()
        .unwrap();
    });
}

/// **WW coherence**：写后写，最终结果一定是最后一个写。
///
/// 对应文档：`X=1; X=2 [X=2 at the end]`
#[test]
fn ww_coherence() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));

        let x_ = Arc::clone(&x);
        thread::spawn(move || {
            x_.store(1, Ordering::Relaxed);
            x_.store(2, Ordering::Relaxed);
        })
        .join()
        .unwrap();

        assert_eq!(x.load(Ordering::Relaxed), 2);
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Per-message View → Release/Acquire Synchronization
// ═══════════════════════════════════════════════════════════════════════════════

/// **Release/Acquire 同步**：Release store 会生成一个 message view（记录 release
/// 时刻线程的完整视图）；Acquire load 将该 message view 合并到当前线程的 view 中。
///
/// 对应文档：
/// ```rust
/// X = 1;                        ||   if Y.load(acquire) == 1:
/// Y.store(1, release);          ||       assert!(X == 1);  // 一定成功
/// ```
///
/// 如果线程 2 看到 Y=1（Acquire），则此前线程 1 对 X=1 的写入也必然可见。
#[test]
fn release_acquire_sync() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));

        let (x_, y_) = (Arc::clone(&x), Arc::clone(&y));
        let t1 = thread::spawn(move || {
            x_.store(1, Ordering::Relaxed);
            y_.store(1, Ordering::Release);
        });

        let (x_, y_) = (Arc::clone(&x), Arc::clone(&y));
        let t2 = thread::spawn(move || {
            if y_.load(Ordering::Acquire) == 1 {
                assert_eq!(x_.load(Ordering::Relaxed), 1);
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
///
/// 如果线程 2 看到 Y=1（relaxed），则此前线程 1 对 X=1 的写入也必然可见
#[test]
fn sc_fence_sync() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));

        let (x_, y_) = (Arc::clone(&x), Arc::clone(&y));
        let t1 = thread::spawn(move || {
            x_.store(1, Ordering::Relaxed);
            fence(Ordering::SeqCst);
            y_.store(1, Ordering::Relaxed);
        });

        let (x_, y_) = (Arc::clone(&x), Arc::clone(&y));
        let t2 = thread::spawn(move || {
            if y_.load(Ordering::Relaxed) == 1 {
                fence(Ordering::SeqCst);
                assert_eq!(x_.load(Ordering::Relaxed), 1);
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
/// ```rust
/// X = 1;                        ||   if Y.load() == 1:
/// Y.store(1);                   ||       assert!(X == 1);  
/// ```
///
/// 线程 2 看到 Y=1 后读 X，此时 X 的值可能是 0（旧值）也可能是 1，因此断言可能会失败。
#[test]
#[should_panic]
fn relaxed_no_sync() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));

        let (x_, y_) = (Arc::clone(&x), Arc::clone(&y));
        let t1 = thread::spawn(move || {
            x_.store(1, Ordering::Relaxed);
            y_.store(1, Ordering::Relaxed);
        });

        let (x_, y_) = (Arc::clone(&x), Arc::clone(&y));
        let t2 = thread::spawn(move || {
            if y_.load(Ordering::Relaxed) == 1 {
                assert_eq!(x_.load(Ordering::Relaxed), 1);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}
