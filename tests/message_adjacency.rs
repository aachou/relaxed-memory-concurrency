//! # Message Adjacency — Read-Modify-Write
//!
//! For RMW operations (e.g. `fetch_add`), the new message must be placed
//! **adjacent** to the message being read.  This prevents two RMWs from both
//! reading the same old value and losing one update.
//!
//! ## 对应文档
//!
//! ```rust
//! r1 = X.fetch_add(1)       ||        r2 = X.fetch_add(1)
//! ```
//!
//! `r1 = r2 = 0` 是不可能的——每个新 message 必须邻接到被读 message 的右侧，
//! 所以第二个 fetch_add 无法读到与第一个相同的初始值。

use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::Arc;
use loom::thread;

/// 两个线程同时对 X 做 `fetch_add(1)`，验证不会出现两者都读到 0。
///
/// 根据 Message Adjacency：第一个 RMW 读 0 写 1，第二个 RMW 只能邻接到 1
/// 的右侧读 1 写 2。因此 `r1 + r2 == 1` 必然成立。
#[test]
fn test_message_adjacency_rmw_2_threads() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            r1c.store(x1.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        });

        let x2 = x.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            r2c.store(x2.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        let v1 = r1.load(Ordering::Relaxed);
        let v2 = r2.load(Ordering::Relaxed);
        assert!(v1 + v2 == 1, "message adjacency forbids both reading 0: got ({}, {})", v1, v2);
    });
}

/// 三个线程分别 `fetch_add(1)`，验证每个线程读到唯一值，最终 X = 3。
///
/// Message Adjacency 保证了 RMW 链式串联：读到 0、1、2 各一次，不会重复。
/// 最终 X 的值等于线程数，没有丢失任何一次加法。
#[test]
fn test_message_adjacency_rmw_3_threads() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));
        let r3 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            r1c.store(x1.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        });

        let x2 = x.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            r2c.store(x2.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        });

        let x3 = x.clone();
        let r3c = r3.clone();
        let t3 = thread::spawn(move || {
            r3c.store(x3.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();
        t3.join().unwrap();

        let v1 = r1.load(Ordering::Relaxed);
        let v2 = r2.load(Ordering::Relaxed);
        let v3 = r3.load(Ordering::Relaxed);

        let mut seen = vec![v1, v2, v3];
        seen.sort();
        assert_eq!(seen, vec![0, 1, 2], "each thread reads a unique value: adjacency chains RMWs");
        assert_eq!(x.load(Ordering::Relaxed), 3, "final value equals number of RMWs");
    });
}
