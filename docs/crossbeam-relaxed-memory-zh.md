# Summary

本文旨在澄清基于 epoch 的内存回收方案中所执行的同步操作，并解释 Crossbeam 的实现相对于 C11/C++11/LLVM/Rust 弱内存并发模型（以下简称"C11 内存模型"）为何是正确的。

# Motivation

Crossbeam 基于 epoch-based memory reclamation（EBR）方案。在 EBR 中，维护了一个 **epoch**（可理解为时间戳），当对象被 unlink 时，会用当前 epoch 标记它。该 unlink 对象在后面 epoch 推进足够后即可安全释放。可以想见，EBR 的正确性关键依赖于 epoch 的不变式：unlink 后的对象在两个 epoch 推进后便不可访问。

EBR 最早由 [Keir Fraser 的博士论文](https://www.cl.cam.ac.uk/techreports/UCAM-CL-TR-579.pdf) 提出，后来 Aaron Turon 在 [他的 Crossbeam 博客文章](https://aturon.github.io/blog/2015/08/27/epoch/) 中将其介绍给 Rust 社区。两者都详细解释了 EBR，读者不仅能理解其背后的直觉，也能明白它为何正确。然而，两者都假设我们处于*顺序一致性（SC）并发模型*下：所有线程对内存的所有访问都是 *linearizable* 的，每个 load 根据 linearization order 读取最近写入的值。但不幸的是，现实世界并不允许我们享受 SC 的奢侈。具体来说，C11/C++11/LLVM/Rust 内存模型是*宽松（relaxed）* 或*弱（weak）* 的，允许每个 load 读到过时的旧值。

因此，经过两年的开发，Crossbeam 的实现相对于 C11 内存模型是否正确仍不清楚。我们认为，为了让 Crossbeam 真正成为 Rust 细粒度并发的"标配"库——正如 `java.util.concurrent` 之于 Java——其正确性应当被正式分析并有望得到验证。本 RFC 旨在朝这个目标迈出一步。

## Non-motivation

Crossbeam 是 EBR 与 Rust linear type system（含生命周期）的结合。然而，本 RFC 并不旨在阐明 Rust 的类型系统如何帮助确保 Crossbeam 的正确性。

# Detailed design

在本节中，我们以伪代码形式呈现 Crossbeam 的提议实现，并解释它相对于 C11 内存模型为何正确。

## Proposed implementation

正如 Aaron Turon 的原始[博客文章](https://aturon.github.io/blog/2015/08/27/epoch/)和[之前 RFC 的拉取请求](https://github.com/crossbeam-rs/rfcs/pull/2)中所讨论的，Crossbeam 保证*对对象的每次访问都必须发生在该对象被释放之前*。为了提供这一保证，Crossbeam 假设当一个固定（pinned）线程将对象标记为 unlink 时，*该线程已从内存中移除对该对象的所有引用，并且所有其他线程不会重新引入对该对象的引用*；同时 Crossbeam 将 unlink 对象的释放推迟到该对象被 unlink 时所有固定的线程都解除固定（unpinned）之后。

为了追踪并发固定的线程，Crossbeam 利用 epoch 如下：

- 每个*固定*的线程 `th` 有一个本地 epoch `th.epoch`，以及一个全局 epoch `EPOCH`。

- 当一个对象被释放时，它用**当前**全局 epoch 标记，如下面的伪代码所示（其中的 fence 和内存顺序，如 `Relaxed`、`Acquire`、`Release`、`AcqRel` 和 `SeqCst`，将在下文解释）：

  ```
  fn unlink(obj) {
    'L10: atomic::fence(SeqCst);
    'L11: let epoch = EPOCH.load(Relaxed);

    'L12: // 用 `epoch` 标记 `obj`
  }
  ```

- 本地 epoch `th.epoch` 被赋值为对应线程 `th` 固定时全局 epoch `EPOCH` 的值，如下所示：

  ```
  fn pin(th) {
    'L20: let e = EPOCH.load(Relaxed);
    'L21: th.epoch.store(e, Relaxed);
    'L22: atomic::fence(SeqCst);

    'L23: // 现在标记为 `<= e - 2` 的对象可以被释放
  }
  ```

  当一个线程解除固定时，其本地 epoch 被设置为一个哨兵值，如下所示：

  ```
  fn unpin(th) {
    'L30: th.epoch.store(SENTINEL, Release);
  }
  ```

- 全局 epoch `EPOCH` 仅在所有固定线程的本地 epoch 都等于全局 epoch 时才递增，如下所示：

  ```
  fn try_advance() {
    'L40: let G = EPOCH.load(Relaxed);
    'L41: atomic::fence(SeqCst);
  
    'L42: for th in threads {
    'L43:   let e = th.epoch.load(Relaxed);
    'L44:   if e != SENTINEL && e != G { return; }
    'L45: }
    'L46: atomic::fence(Acquire);
  
    'L47: EPOCH.store(G + 1, Release);
  
    'L48: // 现在标记为 `<= e - 1` 的对象可以被释放
  }
  ```

注意，在 `pin(): 'L23` 和 `try_advance(): 'L48` 处，标记的 epoch 比当前全局 epoch 早两代的解链对象可以被释放。换句话说，当当前全局 epoch `>= X+2` 时，你可以释放标记为 `X` 的对象。

## Correctness

现在我们来解释为何提议的实现相对于[最新的 C11 并发模型](http://sf.snu.ac.kr/promise-concurrency/)是正确的。

假设当前 epoch 为 `X+2`，线程 U `unlink()` 了一个对象并用 epoch `X` 标记它，线程 D 即将释放该对象。为了正确性，我们需要确保 `pin()` 固定的线程对该对象的访问发生在释放之前。例如，在下面的时间线中，线程 A 和 B 对 `obj` 的访问应该发生在 `obj` 的释放之前。

关于术语"happens before"：我们说 A happens before B，当且仅当 A 可见的所有内存信息对 B 也可见。注意这与 C11 标准中的"happens-before"概念不同。

```
         [X]                       [X+1]            [X+2]
EPOCH    +-------------------------+----------------+----------------------------------

                                                          [deallocating obj]
Thread D -------------------------------------------------+----------------------------

                                              [2. pinned]         [is obj accessible?]
Thread A -------------------------------------+-------------------+--------------------

           [obj removed] [1. unlinking obj] [unpinned]
Thread U --+-------------+------------------+------------------------------------------

               [3. pinned]    [unpinned]                          [is obj accessible?]
Thread B ------+--------------+-----------------------------------+--------------------

                                                    [4. try_advance]
Thread E -------------------------------------------+----------------------------------
```

我们将会看到，正确性严重依赖于 SC fence 的交错（interleaving）性质。直观地说，这意味着如果线程 A 的 SC fence 在另一个线程 B 的 SC  fence 之前执行，那么 A 的 fence happens before B 的 fence ，并且 A 的 fence 之前收集的所有信息在 B 的 fence 之后变得可见。在这种情况下，我们说 A 的 fence 之前的指令*通过 SC fence 对* B 的 fence 之后的指令*可见*。这种同步也被用于 [C11 版本的 Chase-Lev 双端队列](http://www.di.ens.fr/~zappa/readings/ppopp13.pdf)。关于这种交错性质的更多信息，请参见[对 C11 SC 原子操作的最新理解](http://plv.mpi-sws.org/scfix/)。

现在我们考虑 `unlink()` 和 `pin()` 的 SC  fence 顺序的两种情况。

### When `unlink()`'s SC fence is performed before `pin()`'s SC fence

例如，线程 A 对 `obj` 的访问应该发生在 `obj` 的释放之前。因为根据假设，执行 `unlink()` 的线程已经从它的视角移除了内存中对对象的所有引用。通过从 `unlink()` 的 SC fence（时间线中的1）到 `pin()` 的 SC fence（时间线中的2）的交错性质，`pin()` 固定的线程无法访问旧对象 `obj`。

### When `pin()`'s SC fence is performed before `unlink()`'s SC fence

例如，线程 B 对 `obj` 的访问应该发生在 `obj` 的释放之前。因为：

**①** 考虑实际将全局 epoch 推进到 `X+2` 的那次 `try_advance()` 调用。`unlink()` 的 SC fence（时间线中的1）在 `try_advance()` 的 SC fence（时间线中的4）之前执行。因为否则，从 `EPOCH` 读取 `X+1` 的 `try_advance(): 'L40` 将通过 SC  fence 对从 `EPOCH` 读取 `X` 的 `unlink(): 'L11` 可见，这将导致矛盾。

**②** 通过传递性，`pin()` 的 SC fence（时间线中的 3）在 `try_advance()` 的 SC fence（时间线中的 4）之前执行。由于 `pin()` 固定的线程在固定之前已将自己注册到链表 `threads` 中，因此 `try_advance(): 'L42` 对 `threads` 的迭代应该访问到该固定线程。那么 `pin(): 'L21` 对本地 epoch 的 store 通过 SC  fence 对 `try_advance(): 'L43` 从它的 load 可见，并且 `'L43` 应该读取到 `'L21` 写入的值或比那更新的值。

**③** 为了到达 `try_advance(): 'L47` 并递增全局 epoch，`'L43` 不应读取 `'L21` 写入的值。相反，它应该读取 `unpin(): 'L30` 写入的值或比那更新的值，并且通过 release-acquire 同步，`'L30` happens before `'L46`。

**④** 由于线程应在 `unpin()` 之前访问对象，因此 `pin()` 固定的线程对已 unlink 对象的每次访问都发生在该对象的释放之前。

---

**第一步：fence 顺序**

```
线程 B (pin):          fence (3) → 访问 obj → unpin
线程 U (unlink):       删除引用 → fence (1) → 读 epoch
线程 E (try_advance):  读 epoch → fence (4) → 检查 → 推进
```

根据 SC交错性质：如果 (4) 在 (1) 之前，那么 (4) 之前的所有写入，对 (1) 之后的所有读取可见。现在看 EPOCH 的值：

- 'L40 在 fence (4) 之前读到 EPOCH = X+1
- 如果 fence (4) 在 fence (1) 之前，交错性质要求这个 X+1 对 fence (1) 之后的所有读可见
- 所以 'L11 应该读到 X+1 或更新的值

但实际 'L11 读到的是 X（旧值），矛盾。所以假设不成立 → fence (4) 不能在 fence (1) 之前 → 必然是 fence (1) 在 fence (4) 之前。通过传递性，(3) 在 (1) 之前，因此 fence 的时间顺序：(3) → (1) → (4)。

---

**第二步：交错性质的后果**

(3) 在 (4) 之前 → pin() 里 fence (3) 之前的 store（'L21: th.epoch.store(e, Relaxed)）对 fence (4) 之后的 load（'L43: th.epoch.load(Relaxed)）可见。

正常情况下 'L43 应该读到 'L21 写入的值（epoch X）。

---

**第三步：矛盾与必要条件**

但 try_advance() 的全局 epoch 是 X+1，在 'L43 处检查：

```rust
if e != SENTINEL && e != G { return; }
// e = X, G = X+1 → X ≠ X+1 → return，推进失败！
```

如果读到 'L21 的值（X），推进必然被阻塞。要到达 'L47 成功推进，'L43 不能读 'L21 的值，必须读 unpin(): 'L30 写入的 SENTINEL（或更新的值）。

---

**第四步：最终的安全链**

```rust
线程 B:  访问 obj → unpin(): 'L30 store(SENTINEL, Release)
                                                  ↓ release-acquire 同步
线程 E:  'L43 读到 SENTINEL → ... → 'L46 fence(Acquire)
                                      → 'L47 EPOCH.store(G+1, Release)
                                      → 释放 obj
```

'L30 的 Release store 与 'L46 的 Acquire fence 形成 release-acquire 同步 → 'L30 happens-before 'L46 → obj 访问发生在释放之前。

---

**'L47 EPOCH(G+1, Release)**

```rust
C: 访问 obj → 'L30 store(SENTINEL, Release)
                           ↓ release-acquire
A (try_adv): 'L43 load → 'L46 fence(Acquire)
             同一线程顺序 →
              'L47 store(EPOCH, Release)
                           ↓ release-acquire
B (collect): 'L40 load(EPOCH, Relaxed) → 读到 A 写的新值
              'L41 fence(SeqCst) ← 与 'L47 Release 配对
              然后 pop 过期 bag → drop(sealed_bag) ← 释放 obj
```

没有 'L47 的 Release，B 的 'L41 SeqCst fence 虽然有 Acquire 语义，但找不到配对的 Release，happens-before 链在 A→B 之间断了 → drop 时可能存在 obj 访问 = 数据竞争。

---

**repin**

*We store the new epoch with Release because we need to ensure any memory accesses from the previous epoch do not leak into the new one.*

repin 等价 unpin + 隐式 pin。Release store 的作用和 `unpin: 'L30 store(SENTINEL, Release)` 完全一样——与 `try_advance(): 'L43 → 'L46 fence(Acquire)` 形成 release-acquire 同步，告诉 GC"我已离开旧 epoch，旧 epoch 下的事都做完了"。

*We don't need a following SeqCst fence, because it is safe for memory accesses from the new epoch to be executed before updating the local epoch.*

考虑重排：Release 只阻止前面的操作移到 store 之后，不阻止后面的操作移到 store 之前。

```rust
store(epoch, X+1, Release)
// ← Release 边界，前面的操作不能到下面
// ← 但下面的操作可以跑到上面 (无害)
```

所以可能发生：
```rust
// 实际执行顺序（从 CPU 视角）：
读新 epoch 的数据               ← 先执行
store(epoch, X+1, Release)     ← 后执行
```

问题是：读操作提前执行了，此时本地 epoch 还是旧值 X，不是 X+1。`try_advance()` 来检查，看到本地 epoch = X，认为该线程还在旧 epoch → 不推进，不回收 → 安全。等真正执行了 store，`try_advance()` 才看到新 epoch，继续推进。

这最多导致 GC 延迟一个轮次——因为别的线程更晚才看到本地 epoch 更新了。但如果加了 SeqCst fence，会强制读操作必须等 store 完成才能开始，白交 20-40 周期的 mfence 成本，换来的只是"GC 可能提前几个 ns 看到新 epoch"，不值得。

## Correctness by promising semantics

**Case 1（unlink fence 在 pin fence 之前）**

```rust
U: 删除引用 → 'L10 fence(SC) ──→ global_view = {删除引用}
                                       │
A: 'L22 fence(SC) ←────────────────┘ merged into A.thread
   → A 看到 obj 已被删除，找不到它 → 安全 ✅
```

**Case 2（pin fence 在 unlink fence 之前）**

```rust
B: 'L22 fence(SC) ──→ global_view = {B 的起始状态}
   B 访问 obj...

U: 删除引用 → 'L10 fence(SC) ──→ global_view = max(global, U.thread)
   │ 现在 global_view 包含"obj 引用已删除"
   标记 epoch X

B: 'L30 store(SENTINEL, Release)
   │ message view = {B 的 obj 访问}
   ▼

E: 'L41 fence(SC) ──→ global_view = max(global, E.thread)
   │ E 现在看到 global_view = {U 的 obj 删除}
   │ 但此时 B 的数据还没进来
   'L43 load → 'L46 fence(Acquire) ← 合并 B 的 message view → E.thread = {B 的 obj 访问, U 的删除}
   'L47 store(EPOCH, Release)
   │ message view = {E.thread = {B 的 obj 访问, U 的 obj 删除, ...}}
   ▼

C: load(EPOCH) → 'L22 fence(SC)
   ← Acquire 侧合并 'L47 的 message view
   → C.thread = {B 的 obj 访问, U 的 obj 删除}
   → collect() 时 drop(sealed_bag) 安全 ✅
```

# Alternatives

## The current implementation

读者可能注意到上述伪代码与 Crossbeam 当前的实现有所不同。我们认为当前的实现在 C11 内存模型下存在 bug，因为它使用的 SC 读/写不提供交错性质。与普遍认知相反，根据 C11 标准，SC load 和 store 相对较弱：具体来说，内存模型设计者意图允许将 SC load 重排序到 relaxed store 之前，以及将 SC write 重排序到 relaxed load 之后。更多详情请参见[关于 C11 SC 语义的最新论文](http://plv.mpi-sws.org/scfix/)。

## Target-dependent implementation

或者，我们可以为每个目标架构编写 Crossbeam 的核心代码。这可能对性能有益：目前尚不清楚提议的实现是否为每个目标架构编译出了最高效的实现。缺点是为每个架构做适配将耗费大量成本。

## Waiting for the memory models to be stabilized

在开始正式解释 Crossbeam 为何正确之前，也许值得等待底层的 C11 内存模型稳定下来。该模型在过去几十年中已经得到了极大的澄清和改进，但仍可能会有很大变化。然而，Crossbeam 所依赖的模型片段相当稳固，因此值得基于[最新的 C11 并发模型](http://sf.snu.ac.kr/promise-concurrency/)来推理 Crossbeam。

## Ensuring correctness by testing

为了确保 Crossbeam 实现的正确性，可以使用真实工作负载对其进行全面测试。然而，正如 [Dijkstra 早在早期就指出](http://homepages.cs.ncl.ac.uk/brian.randell/NATO/nato1969.PDF) 的那样，"测试只能证明 bug 的存在，无法证明 bug 的不存在"。

# Unresolved questions

这*真的*是 C11 内存模型下最高效的实现吗？特别是，如果能够移除 `unlink()` 中的 `SeqCst`  fence ，将对性能大有裨益。即使在释放对象之前需要等待三个 epoch 推进（而不是两个），这也可能是可行的。这是未来的一个重要研究方向。

上述描述是否存在任何正确性漏洞？更大胆地说，我们能否正式验证 Crossbeam 的正确性？