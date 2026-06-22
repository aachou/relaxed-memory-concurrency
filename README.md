# Relaxed Memory Concurrency

> 使用 [Loom](https://github.com/tokio-rs/loom) 对 **Promising Semantics** 建模的 Relaxed Behaviors & Orderings 及三种互斥锁进行测试。

## Structure

```
src/
├── lib.rs              # 模块声明
├── spin_lock.rs        # SpinLock — CAS + Acquire/Release
├── ticket_lock.rs      # TicketLock — fetch_add + Acquire/Release
├── clh_lock.rs         # CLHLock — swap(AcqRel) + 链表队锁
└── ebr.rs              # Epoch-Based Reclamation（Fraser epoch）

tests/
├── multi_valued_memory.rs    # ① Load Hoisting
├── message_adjacency.rs      # ② RMW 原子性
├── views.rs                  # ③ Coherence + Synchronization
├── promises.rs               # ④ Store Hoisting
├── ebr_tests.rs              # ⑤ Epoch-Based Reclamation
└── lock_tests.rs             # ⑥ SpinLock / TicketLock / CLHLock
```

## Relaxed Behaviors & Orderings Test

### ① Multi-Valued Memory — Load Hoisting

内存建模为 location → message list 的映射，线程可以读到旧值。

```
X = 1;   r1 = Y;     ||     Y = 1;   r2 = X;
```

| 测试 | 验证 | 预期 |
|------|------|------|
| `test_load_hoisting` | Relaxed 下 load 可读到旧值 | `r1=r2=0` **可达**（witness 断言） |

### ② Message Adjacency — RMW Atomicity

RMW 操作的新 message 必须邻接到被读 message 的右侧，防止 RMW 读到旧值。

| 测试 | 验证 | 预期 |
|------|------|------|
| `message_adjacency_rmw_2_threads` | 双线程 `fetch_add(1)` | 不会同时读到 0 |
| `message_adjacency_rmw_3_threads` | 三线程链式 RMW | 每线程读到唯一值，最终 X=3 |

### ③ Views — Coherence & Synchronization

三种 View 约束线程行为：

- **Per-thread view** → 4 种 per-location coherence
- **Per-message view** → Release/Acquire 同步
- **Global view** → SC fence 同步

| 测试 | 对应机制 | 验证 |
|------|---------|------|
| `test_rr_coherence` | RR | `r1=1 则 r2≠0` |
| `test_rw_coherence` | RW | `r=42 在 X=100 之前` |
| `test_wr_coherence` | WR | `X=1 后读得 1` |
| `test_ww_coherence` | WW | 最终 `X=2` |
| `test_release_acquire_sync` | Per-message View | Release/Acquire 保证消息传递 |
| `test_sc_fence_sync` | Global View | 双 SC fence 保证同步 |
| `test_relaxed_no_sync` | 对照 | 无同步时读旧值合法 |

### ④ Promises — Store Hoisting

线程可承诺未来写入某个值，承诺必须能被兑现。

Store hoisting (`r1=X;Y=r1 || r2=Y;X=1 → r1=r2=1`) 在 C++11 内存模型下**允许**，但 Loom **不支持 store hoisting**。Promising Semantics 通过 promise 机制显式建模 store hoisting。

| 测试 | 场景 | C++11 | Loom | PS |
|------|------|-------|------|----|
| `test_store_hoisting_wo_dep` | 无依赖 | 允许 | 不支持 | **可达** |
| `test_store_hoisting_w_dep_oota` | 数据依赖 (OOTA) | 允许（已知缺陷） | 不可达 | **不可达** |
| `test_store_hoisting_syntactic_dep` | 语法依赖 | 允许 | 不支持 | **可达** |
| `test_store_hoisting_syntactic_dep_rw_coherence` | 语法依赖 + RW coherence | `r1=r2=1` 允许，`r3=0`（故三者同时为 1 不可达） | 不可达 | **不可达** |

## ⑤ EBR GC — Epoch-Based Reclamation

基于 Fraser epoch 算法、遵循 crossbeam-relaxed-memory RFC 内存顺序的 EBR 垃圾回收器。

| 关键操作 | 内存顺序 |
|---------|---------|
| `pin()` | `load(Relaxed)` → `store(Relaxed)` → `fence(SeqCst)` |
| `unpin()` | `store(SENTINEL, Release)` |
| `retire()` | `fence(SeqCst)` → `load(Relaxed)` → 入 `retire_lists[epoch]` |
| `try_advance()` | `load(Relaxed)` → `fence(SeqCst)` → 检查所有线程 → `fence(Acquire)` → `store(Release)` |

| 测试 | 验证 |
|------|------|
| `test_basic_reclamation` | 单线程 retire + 两次 epoch 推进后 obj 被释放 |
| `test_full_epoch_rotation` | 三个 epoch 轮转，每个 epoch retire 的对象两次推进后释放 |
| `test_multiple_retires_same_epoch` | 同一 epoch retire 多个 obj 同时释放 |
| `test_repeated_pin` | 重复 pin/unpin 不影响正确性 |
| `test_rfc_case1_retire_before_pin` | RFC Case 1: unlink 的 SC fence < pin 的 SC fence |
| `test_rfc_case2_pin_before_retire` | RFC Case 2: pin 的 SC fence < unlink 的 SC fence |

## ⑥ Mutex Lock

| 锁 | lock | unlock | 关键语义 |
|----|------|--------|---------|
| **SpinLock** | `CAS(false→true, Acquire)` | `store(false, Release)` | CAS 利用 Message Adjacency；Acquire/Release 实现 View 合并 |
| **TicketLock** | `fetch_add(1, Relaxed)` → `load(Acquire) 自旋` | `store(Release)` | 公平排队，Release/Acquire 保证临界区数据可见 |
| **CLHLock** | `swap(node, AcqRel)` → `load(Acquire) 自旋` | `store(false, Release)` | 链式队锁，AcqRel 提供双向 View 合并 |

每个锁两个测试：

| 测试 | 验证 |
|------|------|
| `spin_lock::mutual_exclusion` | 两线程各递增计数器，最终值 = 2 |
| `spin_lock::message_passing` | 线程1 写 data=42 → unlock → 线程2 lock → 读到 42 |
| `ticket_lock::mutual_exclusion` | 同上 |
| `ticket_lock::message_passing` | 同上 |
| `clh_lock::mutual_exclusion` | 同上 |
| `clh_lock::message_passing` | 同上 |

## Run

```powershell
cargo promises
```

运行所有 26 个测试，Loom 会穷举所有线程交错，验证断言在所有调度下均成立。

## Reference

- [Promising Semantics](https://sf.snu.ac.kr/promise-concurrency/)
- [Loom](https://github.com/tokio-rs/loom)
- [KAIST CS431: Concurrent Programming](https://github.com/kaist-cp/cs431)
- [crossbeam-relaxed-memory RFC](./docs/crossbeam-relaxed-memory.md)
- [RC11: Repairing Sequential Consistency in C/C++11 笔记](./docs/scfix-summary.md)

完整文档：[Relaxed Memory Concurrency](./relaxed%20memory%20concurrency.md)
