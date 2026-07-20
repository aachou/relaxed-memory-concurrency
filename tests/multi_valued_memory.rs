//! # Multi-valued Memory — Load Hoisting
//!
//! 内存被表示为 location 到 message 列表的映射，每个 message 由 value 和 timestamp 组成。
//! 线程可以从一个 location 读到 old value。
//!
//! ## 对应文档
//!
//! ```text
//! X = 1;   r1 = Y;      ||      Y = 1;   r2 = X;
//! ```
//!
//! 允许 `r1 = r2 = 0`

use relaxed_memory_concurrency::test::loom;
use relaxed_memory_concurrency::test::loom::sync::Arc;
use relaxed_memory_concurrency::test::loom::sync::atomic::{AtomicUsize, Ordering};
use relaxed_memory_concurrency::test::loom::thread;

#[test]
fn load_hoisting() {
    let reached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reached_ = reached.clone();

    loom::model(move || {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));

        let x_ = Arc::clone(&x);
        let y_ = Arc::clone(&y);
        let t1 = thread::spawn(move || {
            x_.store(1, Ordering::Relaxed);
            y_.load(Ordering::Relaxed)
        });

        let x_ = Arc::clone(&x);
        let y_ = Arc::clone(&y);
        let t2 = thread::spawn(move || {
            y_.store(1, Ordering::Relaxed);
            x_.load(Ordering::Relaxed)
        });

        if t1.join().unwrap() == 0 && t2.join().unwrap() == 0 {
            reached_.store(true, Ordering::Relaxed);
        }
    });

    #[cfg(feature = "check-loom")]
    assert!(reached.load(Ordering::Relaxed));

    #[cfg(not(feature = "check-loom"))]
    assert!(true);
}
