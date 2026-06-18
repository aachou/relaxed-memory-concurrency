//! # Multi-valued Memory — Load Hoisting
//!
//! Memory is modelled as a mapping from locations to message lists, each message
//! having a value and a timestamp.  A thread can read an **old** value from a
//! location (load hoisting) instead of the most recently written one.
//!
//! ## 对应文档
//!
//! ```text
//! X = 1;   r1 = Y;      ||      Y = 1;   r2 = X;
//! ```
//!
//! Under `Relaxed` ordering, `r1 = r2 = 0` **is** reachable because both threads
//! can store-load reorder (read a stale value from the *other* location before
//! their own store becomes visible externally).

use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::Arc;
use loom::thread;

/// 演示在 `Relaxed` 语义下，一个线程可以读到另一个线程写入之前的旧值。
///
/// 两个线程各写一个位置、再读另一个位置。即使两个线程都执行完了，`r1` 和 `r2`
/// 各自读到 0 （旧值）是完全合法的行为。
#[test]
fn relaxed_can_read_old() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let y1 = y.clone();
        let t1 = thread::spawn(move || {
            x1.store(1, Ordering::Relaxed);
            y1.load(Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let t2 = thread::spawn(move || {
            y2.store(1, Ordering::Relaxed);
            x2.load(Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}

/// Store buffering 模式（IRIW 的变体）：两个线程各自写 X/Y 再读对方的位置。
///
/// 此测试仅验证 loom 能探索完所有调度，不会报错。`r1=0 && r2=0` 在 relaxed
/// 语义下是可达的，体现了 **Multi-valued Memory** 允许读旧值的机制。
#[test]
fn store_buffering_allowed() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let y1 = y.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            x1.store(1, Ordering::Relaxed);
            r1c.store(y1.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            y2.store(1, Ordering::Relaxed);
            r2c.store(x2.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Under relaxed semantics, r1=0 && r2=0 IS reachable (load hoisting).
        // This test merely validates that loom explores all schedules without error.
        let _v1 = r1.load(Ordering::Relaxed);
        let _v2 = r2.load(Ordering::Relaxed);
    });
}
