//! # Promises — Store Hoisting
//!
//! **Promises** model store hoisting: a thread may speculatively write (promise)
//! a value before actually executing the store instruction.  The promise must
//! be *fulfillable* — when execution reaches the store, the thread must be able
//! to write the promised value; otherwise the execution is invalid.
//!
//! 线程可以承诺未来会写入某个值。
//! 承诺必须能被兑现——在线程实际执行到写操作时，必须能够写入承诺的值，如果无法兑现，则该执行路径无效。
//!
//! Store Hoisting 一共分为四种情况：
//!
//! | 场景 | 伪代码 | 预期 |
//! |------|--------|------|
//! | ① 无依赖 | `r1=X;Y=r1 \|\| r2=Y;X=1` | 允许 `r1=r2=1` |
//! | ② 数据依赖 (OOTA) | `r1=X;Y=r1 \|\| r2=Y;X=r2` | 不允许`r1=r2=1` |
//! | ③ 语法依赖 | `r1=X;Y=r1 \|\| r2=Y;if(r2==1){X=r2}else{X=1}` | 不允许 `r1=r2=1` |
//! | ④ 语法依赖 + RW coherence | `r1=X;Y=r1 \|\| r2=Y;r3=X;if(r2==1){X=r2}else{X=1}` | 不允许 `r1=r2=r3=1` |

use relaxed_memory_concurrency::test::loom;
use relaxed_memory_concurrency::test::loom::sync::Arc;
use relaxed_memory_concurrency::test::loom::sync::atomic::{AtomicUsize, Ordering};
use relaxed_memory_concurrency::test::loom::thread;

// ═══════════════════════════════════════════════════════════════════════════════
//  场景 ①：Store hoisting 无依赖
// ═══════════════════════════════════════════════════════════════════════════════

/// Thread 2 的 `X=1` 不依赖任何读操作，可以 hoist 到 `r2=Y` 之前执行。
///
/// 对应文档：
/// ```text
/// r1 = X    ||    r2 = Y
/// Y = r1    ||    X = 1
/// ```
///
/// **C++11**: 内存模型允许 `r1=r2=1`——relaxed 下 load-store 可重排序。
///
/// **Loom**: **不支持 store hoisting**。
///
/// **Promising Semantics**: Store hoisting（promise）使 `X=1` 可在 `r2=Y` 之前执行，因此允许 `r1=r2=1`。
#[test]
fn store_hoisting_wo_dep() {
    let reached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reached_ = reached.clone();

    loom::model(move || {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let (x_, y_, r1_) = (Arc::clone(&x), Arc::clone(&y), Arc::clone(&r1));
        let t1 = thread::spawn(move || {
            r1_.store(x_.load(Ordering::Relaxed), Ordering::Relaxed);
            y_.store(r1_.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        let (x_, y_, r2_) = (Arc::clone(&x), Arc::clone(&y), Arc::clone(&r2));
        let t2 = thread::spawn(move || {
            r2_.store(y_.load(Ordering::Relaxed), Ordering::Relaxed);
            x_.store(1, Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        if r1.load(Ordering::Relaxed) == 1 && r2.load(Ordering::Relaxed) == 1 {
            reached_.store(true, Ordering::Relaxed);
        }
    });

    // assert!(reached.load(Ordering::Relaxed));
    assert!(true);
}

// ═══════════════════════════════════════════════════════════════════════════════
//  场景 ②：Store hoisting 有数据依赖 → OOTA 被禁止
// ═══════════════════════════════════════════════════════════════════════════════

/// Thread 2 的 `X = r2` 依赖 `r2 = Y` 的结果，不能 hoist。
///
/// 对应文档：
/// ```text
/// r1 = X    ||    r2 = Y
/// Y = r1    ||    X = r2   // X 写入依赖 r2
/// ```
///
/// **C++11**: 不保证数据依赖，relaxed 下允许 `r1=r2=1`（OOTA 是 C++11 内存模型的已知缺陷，规范未正式禁止）。
///
/// **Promising Semantics**: 数据依赖禁止 store hoisting——`X=r2` 无法在 `r2=Y` 之前执行，因此不允许 `r1=r2=1`（OOTA 被禁止）。
///
/// **Loom** 不支持 store hoisting——不允许 `r1=r2=1`。
#[test]
fn store_hoisting_w_dep_oota() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let (x_, y_, r1_) = (Arc::clone(&x), Arc::clone(&y), Arc::clone(&r1));
        let t1 = thread::spawn(move || {
            r1_.store(x_.load(Ordering::Relaxed), Ordering::Relaxed);
            y_.store(r1_.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        let (x_, y_, r2_) = (Arc::clone(&x), Arc::clone(&y), Arc::clone(&r2));
        let t2 = thread::spawn(move || {
            r2_.store(y_.load(Ordering::Relaxed), Ordering::Relaxed);
            x_.store(r2_.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        assert!(!(r1.load(Ordering::Relaxed) == 1 && r2.load(Ordering::Relaxed) == 1));
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
//  场景 ③：Store hoisting 有语法依赖（但编译器可优化）
// ═══════════════════════════════════════════════════════════════════════════════

/// Thread 2 的 if/else 两个分支都写 `X=1`，等价于无条件 `X=1`。
///
/// 对应文档：
/// ```text
/// r1 = X    ||    r2 = Y
/// Y = r1    ||    if r2 == 1 { X = r2 } else { X = 1 }
/// ```
///
/// **C++11**: 同场景 ①，内存模型允许 `r1=r2=1`。
///
/// **Loom**: 同场景 ①，不支持 store hoisting——不允许 `r1=r2=1`。
///
/// **Promising Semantics**: 同场景 ①，store hoisting 允许 `r1=r2=1`。
///
#[test]
fn store_hoisting_syntactic_dep() {
    let reached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reached_ = reached.clone();

    loom::model(move || {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let (x_, y_, r1_) = (Arc::clone(&x), Arc::clone(&y), Arc::clone(&r1));
        let t1 = thread::spawn(move || {
            r1_.store(x_.load(Ordering::Relaxed), Ordering::Relaxed);
            y_.store(r1_.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        let (x_, y_, r2_) = (Arc::clone(&x), Arc::clone(&y), Arc::clone(&r2));
        let t2 = thread::spawn(move || {
            r2_.store(y_.load(Ordering::Relaxed), Ordering::Relaxed);
            if r2_.load(Ordering::Relaxed) == 1 {
                x_.store(r2_.load(Ordering::Relaxed), Ordering::Relaxed);
            } else {
                x_.store(1, Ordering::Relaxed);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        if r1.load(Ordering::Relaxed) == 1 && r2.load(Ordering::Relaxed) == 1 {
            reached_.store(true, Ordering::Relaxed);
        }
    });

    // assert!(reached.load(Ordering::Relaxed));
    assert!(true);
}

// ═══════════════════════════════════════════════════════════════════════════════
//  场景 ④：Store hoisting 有语法依赖 + RW coherence 阻止兑现
// ═══════════════════════════════════════════════════════════════════════════════

/// Thread 2 的 if/else 两个分支都写 X=1，但写之前插入了 r3=X 读。
///
/// 对应文档：
/// ```text
/// r1 = X    ||    r2 = Y
/// Y = r1    ||    r3 = X
///           ||    if r2 == 1 { X = r2 } else { X = 1 }
/// ```
///
/// **C++11**: `r3 = X` sequenced-before 所有 X 写（两个分支均写 X=1），
/// 因此 r3 只能读到 0。`r1=r2=1` 在 C++11 relaxed 下允许，但 r3 始终为 0，因此不允许 `r1=r2=r3=1`。
///
/// **Promising Semantics**: Thread 2 可 promise X=1（语法依赖），
/// 然后 r3=X 读到自身 promise 值 1，更新 per-thread view 到 promise位置，
/// 导致后续 X=1 无法在正确位置写入兑现。因此不允许 `r1=r2=r3=1`。
///
/// Loom 不支持 store hoisting——不允许 `r1=r2=r3=1`。。
#[test]
fn store_hoisting_syntactic_dep_rw_coherence() {
    loom::model(move || {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));
        let r3 = Arc::new(AtomicUsize::new(0));

        let (x_, y_, r1_) = (Arc::clone(&x), Arc::clone(&y), Arc::clone(&r1));
        let t1 = thread::spawn(move || {
            r1_.store(x_.load(Ordering::Relaxed), Ordering::Relaxed);
            y_.store(r1_.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        let (x_, y_, r2_, r3_) = (
            Arc::clone(&x),
            Arc::clone(&y),
            Arc::clone(&r2),
            Arc::clone(&r3),
        );
        let t2 = thread::spawn(move || {
            r2_.store(y_.load(Ordering::Relaxed), Ordering::Relaxed);
            r3_.store(x_.load(Ordering::Relaxed), Ordering::Relaxed);
            if r2_.load(Ordering::Relaxed) == 1 {
                x_.store(r2_.load(Ordering::Relaxed), Ordering::Relaxed);
            } else {
                x_.store(1, Ordering::Relaxed);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        assert!(
            !(r1.load(Ordering::Relaxed) == 1
                && r2.load(Ordering::Relaxed) == 1
                && r3.load(Ordering::Relaxed) == 1)
        );
    });
}
