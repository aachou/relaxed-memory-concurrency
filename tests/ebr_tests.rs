use relaxed_memory_concurrency::ebr::Collector;

use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use loom::sync::Arc;
use loom::thread;

// ── 顺序测试（协议验证，无并发交错）──────────────────────────

/// 单线程退役一个对象，推进 epoch 两次后验证对象被释放。
#[test]
fn test_basic_reclamation() {
    loom::model(|| {
        let c = Arc::new(Collector::new(2));

        let c1 = c.clone();
        let t1 = thread::spawn(move || {
            let g = c1.pin(0);
            c1.retire(0, 42);
            c1.unpin(g);
        });
        t1.join().unwrap();

        assert!(c.try_advance(), "first advance: 0 -> 1");
        assert!(!c.is_freed(42), "one epoch later: not yet freed");
        assert!(c.try_advance(), "second advance: 1 -> 2");
        assert!(c.is_freed(42), "two epochs later: freed");
    });
}

/// 三个 epoch 轮转，验证每个 epoch 退役的对象在两次推进后被释放。
#[test]
fn test_full_epoch_rotation() {
    loom::model(|| {
        let c = Arc::new(Collector::new(4));

        let c0 = c.clone();
        let t0 = thread::spawn(move || {
            let g = c0.pin(0);
            c0.retire(0, 10);
            c0.unpin(g);
        });
        t0.join().unwrap();
        assert!(c.try_advance(), "0 -> 1");

        let c1 = c.clone();
        let t1 = thread::spawn(move || {
            let g = c1.pin(1);
            c1.retire(1, 20);
            c1.unpin(g);
        });
        t1.join().unwrap();
        assert!(c.try_advance(), "1 -> 2");
        assert!(c.is_freed(10), "10 retired@0 freed after 2 advances");

        let c2 = c.clone();
        let t2 = thread::spawn(move || {
            let g = c2.pin(2);
            c2.retire(2, 30);
            c2.unpin(g);
        });
        t2.join().unwrap();
        assert!(c.try_advance(), "2 -> 0");
        assert!(c.is_freed(20), "20 retired@1 freed after 2 advances");

        assert!(c.try_advance(), "0 -> 1");
        assert!(c.is_freed(30), "30 retired@2 freed after 2 advances");
    });
}

/// 同一 epoch 退役多个对象，全部同时释放。
#[test]
fn test_multiple_retires_same_epoch() {
    loom::model(|| {
        let c = Arc::new(Collector::new(2));

        let c1 = c.clone();
        let t1 = thread::spawn(move || {
            let g = c1.pin(0);
            c1.retire(0, 1);
            c1.retire(0, 2);
            c1.retire(0, 3);
            c1.unpin(g);
        });
        t1.join().unwrap();

        assert!(c.try_advance());
        assert!(!c.is_freed(1));
        assert!(!c.is_freed(2));
        assert!(!c.is_freed(3));

        assert!(c.try_advance());
        assert!(c.is_freed(1));
        assert!(c.is_freed(2));
        assert!(c.is_freed(3));
    });
}

/// 重复 pin/unpin 不影响正确性。
#[test]
fn test_repeated_pin() {
    loom::model(|| {
        let c = Arc::new(Collector::new(2));

        let c1 = c.clone();
        let t = thread::spawn(move || {
            for _ in 0..3 {
            let g = c1.pin(0);
            c1.retire(0, 99);
            c1.unpin(g);
        }});
        t.join().unwrap();

        assert!(c.try_advance());
        assert!(!c.is_freed(99));
        assert!(c.try_advance());
        assert!(c.is_freed(99));
    });
}

// ── 并发测试 ──────────────────────────

/// 线程在旧 epoch pin 住时阻塞 try_advance。
///
/// 线程 1 pin 在 epoch 0 并保持 pin 住；线程 2 反复尝试推进。
/// 第一次 advance (0→1) 因 `e == g` 可通过，第二次 (1→2) 因 `e ≠ g` 被阻塞。
/// 外部 witness 验证 loom 至少找到一条阻塞交错。
#[test]
fn test_pinned_thread_blocks_advance() {
    let saw_block = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let w = std::sync::Arc::clone(&saw_block);

    loom::model(move || {
        let c = Arc::new(Collector::new(2));

        let c1 = c.clone();
        let t1 = thread::spawn(move || {
            let g = c1.pin(0);
            for _ in 0..5 {
                loom::thread::yield_now();
            }
            c1.unpin(g);
        });

        let c2 = c.clone();
        let w2 = w.clone();
        let t2 = thread::spawn(move || {
            for _ in 0..10 {
                if !c2.try_advance() {
                    w2.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                loom::thread::yield_now();
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });

    assert!(
        saw_block.load(std::sync::atomic::Ordering::Relaxed),
        "try_advance was never blocked — loom failed to find a blocked interleaving"
    );
}

/// 并发安全：退役线程仍 pin 时对象不会被释放。
///
/// 外部 witness：若任何 loom 交错中 `is_freed(55)` 返回 true 且退役线程
/// 仍 pin 在相关 epoch，则违反 EBR 核心安全性质。
#[test]
fn test_concurrent_safety_fuzz() {
    let seen_unsafe = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let witness = std::sync::Arc::clone(&seen_unsafe);

    loom::model(move || {
        let c = Arc::new(Collector::new(3));
        let ready = Arc::new(AtomicBool::new(false));

        let c1 = c.clone();
        let r1 = ready.clone();
        let w1 = witness.clone();
        let t_work = thread::spawn(move || {
            let g = c1.pin(0);
            c1.retire(0, 55);
            r1.store(true, Ordering::Release);
            for _ in 0..5 {
                loom::thread::yield_now();
                if c1.is_freed(55) {
                    w1.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            c1.unpin(g);
        });

        let c2 = c.clone();
        let r2 = ready.clone();
        let t_adv = thread::spawn(move || {
            while !r2.load(Ordering::Acquire) {
                loom::hint::spin_loop();
            }
            for _ in 0..8 {
                c2.try_advance();
                loom::thread::yield_now();
            }
        });

        t_work.join().unwrap();
        t_adv.join().unwrap();
    });

    assert!(
        !seen_unsafe.load(std::sync::atomic::Ordering::Relaxed),
        "object freed while pinning thread was still pinned at retirement epoch"
    );
}

/// RFC §Correctness Case 1: unlink 的 SC fence < pin 的 SC fence。
#[test]
fn test_rfc_case1_retire_before_pin() {
    loom::model(|| {
        let c = Arc::new(Collector::new(3));
        let data = Arc::new(AtomicUsize::new(42));
        let done = Arc::new(AtomicBool::new(false));

        let cu = c.clone();
        let du = data.clone();
        let done_u = done.clone();
        let tu = thread::spawn(move || {
            let g = cu.pin(0);
            du.store(0, Ordering::Relaxed);    
            done_u.store(true, Ordering::Release);
            cu.retire(0, 42);                   
            cu.unpin(g);
        });

        let ca = c.clone();
        let da = data.clone();
        let done_a = done.clone();
        let ta = thread::spawn(move || {
            let g = ca.pin(1);
            if done_a.load(Ordering::Acquire) {
                assert_eq!(da.load(Ordering::Relaxed), 0,
                    "Case 1: U 的 Release 传播后 A 必须看到 data==0");
            }
            ca.unpin(g);
        });

        tu.join().unwrap();
        ta.join().unwrap();

        assert!(c.try_advance(), "0 -> 1");
        assert!(!c.is_freed(42));
        assert!(c.try_advance(), "1 -> 2");
        assert!(c.is_freed(42));
    });
}

/// RFC §Correctness Case 2: pin 的 SC fence < unlink 的 SC fence。
#[test]
fn test_rfc_case2_pin_before_retire() {
    loom::model(|| {
        let c = Arc::new(Collector::new(3));
        let data = Arc::new(AtomicUsize::new(42));

        let cu = c.clone();
        let du = data.clone();
        let tu = thread::spawn(move || {
            let g = cu.pin(0);
            du.store(0, Ordering::Relaxed);
            cu.retire(0, 42);
            cu.unpin(g);
        });

        let cb = c.clone();
        let db = data.clone();
        let tb = thread::spawn(move || {
            let g = cb.pin(1);
            let _val = db.load(Ordering::Relaxed); // 可能 42 或 0，都是安全的
            cb.unpin(g);
        });

        tu.join().unwrap();
        tb.join().unwrap();

        assert!(c.try_advance(), "0 -> 1");
        assert!(!c.is_freed(42));
        assert!(c.try_advance(), "1 -> 2");
        assert!(c.is_freed(42));
    });
}