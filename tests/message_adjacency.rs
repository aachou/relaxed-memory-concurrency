//! # Message Adjacency — Read-Modify-Write
//!
//! 对于 fetch-add 这类 RMW(ReadModifyWrite) 指令，新 message 必须邻接到被读取的 message 右侧。
//! 这防止了 RMW 读取到 old value。
//!
//! ## 对应文档
//!
//! ```rust
//! r1 = X.fetch_add(1)       ||        r2 = X.fetch_add(1)
//! ```
//!
//! 不允许 `r1 = r2 = 0`。

use relaxed_memory_concurrency::test::loom;
use relaxed_memory_concurrency::test::loom::sync::Arc;
use relaxed_memory_concurrency::test::loom::sync::atomic::{AtomicUsize, Ordering};
use relaxed_memory_concurrency::test::loom::thread;

#[test]
fn message_adjacency_rmw_2_threads() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));

        let x_ = Arc::clone(&x);
        let t1 = thread::spawn(move || x_.fetch_add(1, Ordering::Relaxed));

        let x_ = Arc::clone(&x);
        let t2 = thread::spawn(move || x_.fetch_add(1, Ordering::Relaxed));

        let mut reached = vec![t1.join().unwrap(), t2.join().unwrap()];
        reached.sort();
        assert_eq!(reached, vec![0, 1]);
    });
}

#[test]
fn message_adjacency_rmw_3_threads() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));

        let x_ = Arc::clone(&x);
        let t1 = thread::spawn(move || x_.fetch_add(1, Ordering::Relaxed));

        let x_ = Arc::clone(&x);
        let t2 = thread::spawn(move || x_.fetch_add(1, Ordering::Relaxed));

        let x_ = Arc::clone(&x);
        let t3 = thread::spawn(move || x_.fetch_add(1, Ordering::Relaxed));

        let mut reached = vec![t1.join().unwrap(), t2.join().unwrap(), t3.join().unwrap()];
        reached.sort();
        assert_eq!(reached, vec![0, 1, 2]);
    });
}
