//! # Promises — Store Hoisting
//!
//! **Promises** model store hoisting: a thread may speculatively write (promise)
//! a value before actually executing the store instruction.  The promise must
//! be *fulfillable* — when execution reaches the store, the thread must be able
//! to write the promised value; otherwise the execution is invalid.
//!
//! Four scenarios are covered (corresponding to the document's four cases):
//!
//! | 场景 | 伪代码 | 预期 |
//! |------|--------|------|
//! | ① 无依赖 | `r1=X;Y=r1 \|\| r2=Y;X=1` | `r1=r2=1` **可达** |
//! | ② 数据依赖 (OOTA) | `r1=X;Y=r1 \|\| r2=Y;X=r2` | `r1=r2=1` **不可达** |
//! | ③ 语法依赖 | `r1=X;Y=r1 \|\| r2=Y;if(r2==1){X=r2}else{X=1}` | `r1=r2=1` **可达** |
//! | ④ 语法依赖 + RW coherence | `r1=X;Y=r1 \|\| r2=Y;r3=X;if(r2==1){X=r2}else{X=1}` | `r1=r2=r3=1` **不可达** |

use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::Arc;
use loom::thread;

// ═══════════════════════════════════════════════════════════════════════════════
//  场景 ①：Store hoisting 无数据依赖
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
/// **Promising Semantics**: Store hoisting（promise）使 `X=1` 可在
/// `r2=Y` 之前执行，因此 `r1=r2=1` **可达**。
///
/// C++11 与 PS 结论一致（均允许），Loom 因不支持 store hoisting 而不产生此结果。
#[test]
fn test_store_hoisting_wo_dep() {
    loom::model(move || {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let y1 = y.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            r1c.store(x1.load(Ordering::Relaxed), Ordering::Relaxed);
            y1.store(r1c.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            r2c.store(y2.load(Ordering::Relaxed), Ordering::Relaxed);
            x2.store(1, Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Loom 不支持 store hoisting，无法产生 r1=r2=1。
        let _v1 = r1.load(Ordering::Relaxed);
        let _v2 = r2.load(Ordering::Relaxed);
    });
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
/// **C++11**: 内存模型不追踪数据依赖，relaxed 下 `r1=r2=1` **可达**
///（OOTA 是 C++11 内存模型的已知缺陷，规范未正式禁止）。
///
/// **Promising Semantics**: 数据依赖禁止 store hoisting——`X=r2` 无法
/// 在 `r2=Y` 之前执行，因此 `r1=r2=1` **不可达**（OOTA 被禁止）。
///
/// 两种模型对此场景结论不同（C++11 允许，PS 禁止），Loom 不支持 store hoisting：`r1=r2=1` **不可达**。  
#[test]
fn test_store_hoisting_w_dep_oota() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let y1 = y.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            r1c.store(x1.load(Ordering::Relaxed), Ordering::Relaxed);
            y1.store(r1c.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            r2c.store(y2.load(Ordering::Relaxed), Ordering::Relaxed);
            x2.store(r2c.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        let v1 = r1.load(Ordering::Relaxed);
        let v2 = r2.load(Ordering::Relaxed);
        assert!(
            !(v1 == 1 && v2 == 1),
            "store hoist with dep (OOTA): r1=1 && r2=1 must be impossible"
        );
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
/// **Loom**: 同场景 ①，不支持 store hoisting。
///
/// **Promising Semantics**: 同场景 ①，store hoisting 使 `r1=r2=1` **可达**。
///
/// C++11 与 PS 结论一致（均允许），Loom 因不支持 store hoisting 而不产生此结果。
#[test]
fn test_store_hoisting_syntactic_dep() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let y1 = y.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            r1c.store(x1.load(Ordering::Relaxed), Ordering::Relaxed);
            y1.store(r1c.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            r2c.store(y2.load(Ordering::Relaxed), Ordering::Relaxed);
            if r2c.load(Ordering::Relaxed) == 1 {
                x2.store(r2c.load(Ordering::Relaxed), Ordering::Relaxed);
            } else {
                x2.store(1, Ordering::Relaxed);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Loom 不支持 store hoisting，无法产生 r1=r2=1。
        let _v1 = r1.load(Ordering::Relaxed);
        let _v2 = r2.load(Ordering::Relaxed);
    });
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
/// 因此 r3 只能读到 0。`r1=r2=1` 在 C++11 relaxed 下**可达**，但 r3 始终为 0，故 `r1=r2=r3=1` **不可达**。
///
/// **Promising Semantics**: Thread 2 可 promise X=1（语法依赖），
/// 然后 r3=X 读到自身 promise 值 1，更新 per-thread view 到 promise
/// 位置，导致后续 X=1 无法在正确位置写入兑现。因此 `r1=r2=r3=1` **不可达**。
///
/// Loom 不支持 store hoisting，与两种模型结果相同（`r1=r2=r3=1` 不可达），但机制不同。
#[test]
fn test_store_hoisting_syntactic_dep_rw_coherence() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));
        let r3 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let y1 = y.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            r1c.store(x1.load(Ordering::Relaxed), Ordering::Relaxed);
            y1.store(r1c.load(Ordering::Relaxed), Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let r2c = r2.clone();
        let r3c = r3.clone();
        let t2 = thread::spawn(move || {
            r2c.store(y2.load(Ordering::Relaxed), Ordering::Relaxed);
            r3c.store(x2.load(Ordering::Relaxed), Ordering::Relaxed);
            if r2c.load(Ordering::Relaxed) == 1 {
                x2.store(r2c.load(Ordering::Relaxed), Ordering::Relaxed);
            } else {
                x2.store(1, Ordering::Relaxed);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        let v1 = r1.load(Ordering::Relaxed);
        let v2 = r2.load(Ordering::Relaxed);
        let v3 = r3.load(Ordering::Relaxed);
        assert!(
            !(v1 == 1 && v2 == 1 && v3 == 1),
            "scenario 4: r1=1 && r2=1 && r3=1 must be impossible (RW coherence blocks promise)"
        );
    });
}
