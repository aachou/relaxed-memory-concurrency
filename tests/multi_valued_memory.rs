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

/// 验证在 `Relaxed` 语义下 `r1 = 0 && r2 = 0` **可达**（load hoisting）。
///
/// 两个线程各写一个位置再读另一个位置。通过 witness 捕获目标状态：
/// loom 探索所有调度后 witness 被设置，则证明该行为确实存在。
#[test]
fn test_load_hoisting() {
    let reached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let r = reached.clone();

    loom::model(move || {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(usize::MAX));
        let r2 = Arc::new(AtomicUsize::new(usize::MAX));

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

        if r1.load(Ordering::Relaxed) == 0 && r2.load(Ordering::Relaxed) == 0 {
            r.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });

    assert!(
        reached.load(std::sync::atomic::Ordering::SeqCst),
        "r1=0 && r2=0 must be reachable under Relaxed (load hoisting)"
    );
}
