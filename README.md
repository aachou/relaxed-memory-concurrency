# Relaxed Memory Concurrency

> 使用 [Loom](https://github.com/tokio-rs/loom) 框架对 **Promising Semantics** 的四个核心机制及三种锁实现进行模型检测。

## 项目结构

```
src/
├── lib.rs              # 模块声明
├── spin_lock.rs        # SpinLock — CAS + Acquire/Release
├── ticket_lock.rs      # TicketLock — fetch_add + Acquire/Release
└── clh_lock.rs         # CLHLock — swap(AcqRel) + 链表队锁

tests/
├── multi_valued_memory.rs    # ① Load Hoisting
├── message_adjacency.rs      # ② RMW 原子性
├── views.rs                  # ③ Coherence + Synchronization
├── promises.rs               # ④ Store Hoisting
└── lock_tests.rs             # SpinLock / TicketLock / CLHLock
```

## Promising Semantics 四机制测试

### ① Multi-valued Memory — Load Hoisting

内存建模为 location → message list 的映射，线程可以读到旧值。

```
X = 1;   r1 = Y;     ||     Y = 1;   r2 = X;
```

| 测试 | 验证 | 预期 |
|------|------|------|
| `relaxed_can_read_old` | `Relaxed` 下读旧值是否允许 | loom 正常探索 |
| `store_buffering_allowed` | store buffering 模式 | `r1=r2=0` 可达 |

### ② Message Adjacency — RMW 原子性

RMW 操作的新 message 必须邻接到被读 message 的右侧，防止 RMW 读到旧值。

| 测试 | 验证 | 预期 |
|------|------|------|
| `rmw_no_double_zero` | 双线程 `fetch_add(1)` | 不会同时读到 0 |
| `rmw_three_threads` | 三线程链式 RMW | 每线程读到唯一值，最终 X=3 |

### ③ Views — Coherence & Synchronization

三种 View 约束线程行为：

- **Per-thread view** → 4 种 per-location coherence
- **Per-message view** → Release/Acquire 同步
- **Global view** → SC fence 同步

| 测试 | 对应机制 | 验证 |
|------|---------|------|
| `rr_coherence` | RR | `r1=1 则 r2≠0` |
| `rw_coherence` | RW | `r=42 在 X=100 之前` |
| `wr_coherence` | WR | `X=1 后读得 1` |
| `ww_coherence` | WW | 最终 `X=2` |
| `release_acquire_sync` | Per-message View | Release/Acquire 保证消息传递 |
| `sc_fence_sync` | Global View | 双 SC fence 保证同步 |
| `relaxed_no_sync` | 对照 | 无同步时读旧值合法 |

### ④ Promises — Store Hoisting

线程可承诺未来写入某个值，承诺必须能被兑现。

| 测试 | 场景 | 预期 |
|------|------|------|
| `store_hoist_wo_dep` | 无依赖 | `r1=r2=1` 可达 |
| `store_hoist_w_dep_oota` | 数据依赖 (OOTA) | `r1=r2=1` 不可达 |
| `store_hoist_syntactic_dep` | 语法依赖 | `r1=r2=1` 可达 |
| `store_hoist_syntactic_dep_rw_coherence` | 语法依赖 + RW coherence | `r1=r2=r3=1` 不可达 |

## 三种锁

| 锁 | lock | unlock | 关键语义 |
|----|------|--------|---------|
| **SpinLock** | `CAS(false→true, Acquire)` | `store(false, Release)` | CAS 利用 Message Adjacency；Acquire/Release 实现 View 合并 |
| **TicketLock** | `fetch_add(1, Relaxed)` → `load(Acquire) 自旋` | `store(Release)` | 公平排队，Release/Acquire 保证临界区数据可见 |
| **CLHLock** | `swap(node, AcqRel)` → `load(Acquire) 自旋` | `store(false, Release)` | 链式队锁，AcqRel 提供双向 View 合并 |

每个锁两个测试：

| 测试 | 验证 |
|------|------|
| `mutual_exclusion` | 两线程各递增计数器，最终值 = 2 |
| `message_passing` | 线程1 写 data=42 → unlock → 线程2 lock → 读到 42 |

## 运行

```powershell
# Cargo alias（推荐）
cargo loom

# Batch 脚本
loom-test.bat
```

运行所有 21 个测试，Loom 会穷举所有线程交错，验证断言在所有调度下均成立。

## 参考

- [Promising Semantics](https://sf.snu.ac.kr/promise-concurrency/)
- [Loom — tokio-rs/loom](https://github.com/tokio-rs/loom)
- [KAIST CS431: Concurrent Programming](https://github.com/kaist-cp/cs431)
- 详细文档见 [`relaxed memory concurrency.md`](./relaxed%20memory%20concurrency.md)
