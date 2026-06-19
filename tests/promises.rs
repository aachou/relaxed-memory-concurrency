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
//! | ④ 语法依赖 + RW coherence | `r1=X;Y=r1 \|\| r2=Y;r3=X;if(r2==1){X=r2}` | `r1=r2=r3=1` **不可达** |

use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::Arc;
use loom::thread;

// ═══════════════════════════════════════════════════════════════════════════════
//  场景 ①：Store hoisting 无数据依赖
// ═══════════════════════════════════════════════════════════════════════════════

/// 线程2 的 `X=1` 不依赖任何读操作，可以 hoist 到 `r2=Y` 之前执行。
///
/// 对应文档：
/// ```text
/// r1 = X    ||    r2 = Y
/// Y = r1    ||    X = 1
/// ```
///
/// 注意：Store hoisting 是编译器优化，Loom（基于 C11 内存模型）不建模。
/// Loom 仅验证此代码不会出现数据竞争或死锁。`r1=1 && r2=1` 的理论可达性
/// 依赖于 Promises 的 store hoisting 机制，不在 Loom 的能力范围内。
#[test]
fn store_hoist_wo_dep() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let y1 = y.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            let v = x1.load(Ordering::Relaxed);
            y1.store(v, Ordering::Relaxed);
            r1c.store(v, Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            let v = y2.load(Ordering::Relaxed);
            x2.store(1, Ordering::Relaxed);
            r2c.store(v, Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Store hoisting is a compiler optimization not modelled by Loom.
        // This test merely runs the pattern without asserting outcomes.
        let _v1 = r1.load(Ordering::Relaxed);
        let _v2 = r2.load(Ordering::Relaxed);
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
//  场景 ②：Store hoisting 有数据依赖 → OOTA 被禁止
// ═══════════════════════════════════════════════════════════════════════════════

/// 线程2 的 `X = r2` 依赖 `r2 = Y` 的结果，不能 hoist。
///
/// 对应文档：
/// ```text
/// r1 = X    ||    r2 = Y
/// Y = r1    ||    X = r2   // X 写入依赖 r2
/// ```
///
/// 如果允许 `r1 = r2 = 1`，则出现 **out-of-thin-air** (OOTA) 行为：
/// 线程1 读 X=1 是因为线程2 写了 X=1，线程2 写 X=1 是因为读了 Y=1，
/// 而 Y=1 又是线程1 写入的，线程1 写入 Y=1 是因为读了 X=1——形成了
/// 因果循环。Promises 机制保证了此类 OOTA 行为不可能发生。
#[test]
fn store_hoist_w_dep_oota() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let y1 = y.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            let v = x1.load(Ordering::Relaxed);
            y1.store(v, Ordering::Relaxed);
            r1c.store(v, Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            let v = y2.load(Ordering::Relaxed);
            x2.store(v, Ordering::Relaxed);
            r2c.store(v, Ordering::Relaxed);
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

/// 分支的两种结果都是 `X=1`，编译器可将分支优化为无条件 `X=1`，等价于场景 ①。
///
/// 对应文档：
/// ```text
/// r1 = X    ||    r2 = Y
/// Y = r1    ||    if r2 == 1 { X = r2 } else { X = 1 }
/// ```
///
/// 同场景 ①，此编译器优化超出 Loom 的建模范围。Loom 仅确保无数据竞争。
#[test]
fn store_hoist_syntactic_dep() {
    loom::model(|| {
        let x = Arc::new(AtomicUsize::new(0));
        let y = Arc::new(AtomicUsize::new(0));
        let r1 = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::new(AtomicUsize::new(0));

        let x1 = x.clone();
        let y1 = y.clone();
        let r1c = r1.clone();
        let t1 = thread::spawn(move || {
            let v = x1.load(Ordering::Relaxed);
            y1.store(v, Ordering::Relaxed);
            r1c.store(v, Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let r2c = r2.clone();
        let t2 = thread::spawn(move || {
            let v = y2.load(Ordering::Relaxed);
            if v == 1 {
                x2.store(v, Ordering::Relaxed);
            } else {
                x2.store(1, Ordering::Relaxed);
            }
            r2c.store(v, Ordering::Relaxed);
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Same as scenario 1: store hoisting is a compiler optimization.
        let _v1 = r1.load(Ordering::Relaxed);
        let _v2 = r2.load(Ordering::Relaxed);
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
//  场景 ④：Store hoisting 有语法依赖 + RW coherence 阻止兑现
// ═══════════════════════════════════════════════════════════════════════════════

/// 线程2 promise X=1 后，自身又通过 RW coherence 读到 X=1，此时 per-thread
/// view 已更新到 X=1 之后，导致写 `X = r2` 无法兑现 promise。
///
/// 对应文档：
/// ```text
/// r1 = X    ||    r2 = Y
/// Y = r1    ||    r3 = X
///           ||    if r2 == 1 { X = r2 }
/// ```
///
/// 线程2 先 r3=X 读到 1（view 更新），之后执行 X=r2 时 view 已经在 X=1
/// 的右边，无法在正确位置写入 X=1，因此 r1=r2=r3=1 **不可达**。
#[test]
fn store_hoist_syntactic_dep_rw_coherence() {
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
            let v = x1.load(Ordering::Relaxed);
            y1.store(v, Ordering::Relaxed);
            r1c.store(v, Ordering::Relaxed);
        });

        let x2 = x.clone();
        let y2 = y.clone();
        let r2c = r2.clone();
        let r3c = r3.clone();
        let t2 = thread::spawn(move || {
            let v = y2.load(Ordering::Relaxed);
            r2c.store(v, Ordering::Relaxed);
            let w = x2.load(Ordering::Relaxed);
            r3c.store(w, Ordering::Relaxed);
            if v == 1 {
                x2.store(v, Ordering::Relaxed);
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
