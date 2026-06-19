//! # Lock Tests — SpinLock, TicketLock, CLHLock
//!
//! Each lock is tested with two loom model-checking scenarios:
//!
//! | 测试 | 验证内容 | 方法 |
//! |------|---------|------|
//! | `mutual_exclusion` | 互斥性 | 两线程各递增计数器 1 次，最终值 = 2 |
//! | `message_passing` | 临界区数据可见性 | 线程1 写值 + unlock → 线程2 lock + 读值 |

use relaxed_memory_concurrency::spin_lock::SpinLock;
use relaxed_memory_concurrency::ticket_lock::TicketLock;
use relaxed_memory_concurrency::clh_lock::CLHLock;

use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::Arc;
use loom::thread;

macro_rules! lock_tests {
    ($mod_name:ident, $lock_type:ty) => {
        mod $mod_name {
            use super::*;

            /// 两个线程在 lock 保护下各递增计数器一次，验证最终值为 2。
            ///
            /// 如果锁不能保证互斥，则可能出现丢失更新的情况（两个线程同时读 0，
            /// 各写 1，最终值为 1 而不是 2）。Loom 会探索所有线程交错来验证
            /// 互斥性在所有调度下都成立。
            #[test]
            fn mutual_exclusion() {
                loom::model(|| {
                    let lock = Arc::new(<$lock_type>::default());
                    let counter = Arc::new(AtomicUsize::new(0));

                    let l1 = lock.clone();
                    let c1 = counter.clone();
                    let t1 = thread::spawn(move || {
                        let tok = l1.lock();
                        c1.store(c1.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
                        l1.unlock(tok);
                    });

                    let l2 = lock.clone();
                    let c2 = counter.clone();
                    let t2 = thread::spawn(move || {
                        let tok = l2.lock();
                        c2.store(c2.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
                        l2.unlock(tok);
                    });

                    t1.join().unwrap();
                    t2.join().unwrap();

                    assert_eq!(counter.load(Ordering::Acquire), 2,
                        "mutual exclusion: each increment is preserved, no lost updates");
                });
            }

            /// 验证锁的 Release/Acquire 语义能传递临界区内的数据。
            ///
            /// 线程1 先获取锁、写入 data=42、释放锁；线程2 等待 ready 信号后
            /// 获取锁、读取 data。锁的 unlock (Release) 和后续 lock (Acquire)
            /// 之间的 happens-before 关系保证了线程2 必然读到 42。
            #[test]
            fn message_passing() {
                loom::model(|| {
                    let lock = Arc::new(<$lock_type>::default());
                    let data = Arc::new(AtomicUsize::new(0));
                    let ready = Arc::new(AtomicUsize::new(0));

                    let l1 = lock.clone();
                    let d1 = data.clone();
                    let r1 = ready.clone();
                    let t1 = thread::spawn(move || {
                        let tok = l1.lock();
                        d1.store(42, Ordering::Relaxed);
                        l1.unlock(tok);
                        r1.store(1, Ordering::Release);
                    });

                    let l2 = lock.clone();
                    let d2 = data.clone();
                    let r2 = ready.clone();
                    let t2 = thread::spawn(move || {
                        while r2.load(Ordering::Acquire) == 0 {
                            loom::hint::spin_loop();
                        }
                        let tok = l2.lock();
                        let val = d2.load(Ordering::Relaxed);
                        assert_eq!(val, 42,
                            "Release unlock + Acquire lock: critical section write is visible");
                        l2.unlock(tok);
                    });

                    t1.join().unwrap();
                    t2.join().unwrap();
                });
            }
        }
    };
}

lock_tests!(spin_lock, SpinLock);
lock_tests!(ticket_lock, TicketLock);
lock_tests!(clh_lock, CLHLock);
