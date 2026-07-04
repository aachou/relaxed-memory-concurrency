# 《Split-Ordered Lists: Lock-Free Extensible Hash Tables》论文总结

## 一、核心贡献

论文提出了**首个仅使用 load、store 和 CAS（compare-and-swap）即可实现的无锁可扩展哈希表**，支持并发的 insert、delete、find 操作，且期望时间复杂度均为 O(1)。核心创新是 **recursive split-ordering（递归分裂序）**——通过简单的位反转（bit-reversal）对链表元素排序，使得 resize 时不需要迁移任何元素，只需一个 CAS 操作插入 dummy 节点、重定向 bucket 指针即可完成"分裂"。

### 关键设计

| 组件 | 说明 |
|------|------|
| **全局有序链表** | 所有元素存在一条统一的无锁链表中，按 split-order key 排序 |
| **bucket 数组** | 可扩展的指针数组，每个条目指向链表中的 dummy 节点，提供 O(1) 入口 |
| **Dummy 节点** | 每个 bucket 有一个，标记该 bucket 子链表的起始位置，永不删除 |
| **Split-order key** | 对原始 key 做位反转得到；常规 key 先置 MSB=1 再反转，dummy key 直接反转 |
| **惰性初始化** | bucket 首次被访问时才初始化（插入 dummy 节点） |

### 算法操作

- **insert**: 计算 `key mod size` → 初始化 bucket（如有必要）→ 用 Michael 链表算法插入 split-order key → `fetch-and-inc` 计数 → 判断是否需翻倍（`count > size × L`）
- **find/delete**: 类似流程，在有序链表中按 split-order 遍历查找
- **initialize_bucket**: 创建 dummy 节点，通过 parent bucket（`GET_PARENT` 清除最高 set bit）递归初始化后插入链表，然后原子地设置 bucket 指针

### Resize（翻倍）

翻倍只需在链表对应位置插入新 dummy 节点 → CAS 设置新 bucket 指针 → 新 bucket 的入口密度重新变为 O(1)。**不需要移动任何元素**，这是与所有先前方案的本质区别。均匀分布下，`initialize_bucket` 的递归初始化期望深度为常数。

### 分段数组（实践版本）

为避免连续数组翻倍时的全局 reallocation，实际实现使用两级结构：固定大小的主数组指向多个 bucket segment，每个 segment 在首次访问时按需分配。虽然渐近分析下是 O(log n)，但在有限内存（64 位地址空间）中最多 4 级即可穷尽，因此实际开销是常数。

---

## 二、论文引用的相关工作

### 1. Litwin [1980] — 顺序线性哈希（Sequential Linear Hashing）

线性哈希的核心思想是不一次性翻倍 + 全量 rehash，而是维护一个 `next` 指针标记分裂进度。**每次插入操作都伴随一个 bucket 的分裂**：把 `next` 指向的旧 bucket 中的元素按新哈希函数（`mod 2^(i+1)`）分配到两个 bucket 中。分裂过程分摊到每次插入，避免单次 O(n) 的大延迟。哈希函数采用模 2^i，保证了翻倍时分裂的位模式与 split-ordering 兼容。

### 2. Ellis [1983, 1987] — 两级锁定可扩展哈希

采用两级锁方案：一个**目录锁**（读/写锁）保护表目录结构，每个 bucket 有独立的**桶锁**（读/写锁）。查找时获取目录读锁 + 桶读锁，插入/删除时获取目录读锁 + 桶写锁。**Resize 时需要同时获取所有 bucket 的写锁**，开销大且阻塞线程。这是论文批评的"基于锁的典型缺陷"——死锁、长时间延迟、优先级反转。

### 3. Mellor-Crummey and Scott [1991] — 读写锁（reader-writer lock）

Michael 将其用于每 bucket 加锁的哈希方案。在不可扩展的固定大小哈希表上表现合理，但 resize 时需要同时持有所有锁。

### 4. Harris [2001] — 基于 CAS 的无锁有序链表

提出了第一个实用的基于 CAS 的无锁链表算法，支持并发的插入、删除、查找。该算法是 Michael [2002a] 算法的基础，也是论文 split-ordering 所依赖的底层链表实现。

### 5. Michael [2002a] — 无锁哈希表（固定大小）

在 Harris 链表算法的基础上增加了改进的内存管理（与 EBR 等方案配合良好），并用它构造了一个**固定大小的无锁哈希表**：bucket 数组大小在初始化时确定，每个 bucket 是一条 Harris 风格的无锁链表。在多道程序环境下显著优于基于锁的方案。**论文引用其作为底层链表实现**。

**局限性**：不可扩展。元素增长超出预定大小时，bucket 链表变长，操作退化为 O(n)，不再是常数时间。

### 6. Michael [2002b] — 内存回收方案

与 Michael [2002a] 配套的内存管理方案（hazard pointers 相关），被论文引用作为无锁链表内存回收的参考。

### 7. Greenwald [1999] — 阻塞同步的缺陷

列举了基于锁的方案的三大缺陷：死锁（deadlocks）、长时间延迟（long delays）、优先级反转（priority inversions）。优先级反转指低优先级线程持有锁时被中优先级线程抢占，导致等待该锁的高优先级线程无限期阻塞。

### 8. Greenwald [2002] — 基于 DCAS 的两手持 emulate 可扩展哈希

使用 **DCAS（double-compare-and-swap）** 原子地同时操作两个非相邻内存地址，实现 resize 时从旧链表删除 + 插入新链表的原子性。需要 **helping 机制**：线程 A 中途挂起时，其他线程帮 A 完成操作。

**论文批评**：(1) DCAS 在主流 CPU 上不可用；(2) helping 机制在特定调度下导致时间复杂度与进程数线性相关，不是真正的 O(1) 可扩展哈希。

### 9. Gao 等人 [2004] — 基于开放寻址 + write-all 的几乎无等待可扩展哈希

采用**开放寻址（open addressing）** + **线性探测**，仅用 CAS 操作。Resize 时切换到全局 resize 状态，所有线程共同执行 **write-all 算法** 将元素从旧数组迁移到新数组。删除使用墓碑（tombstone）标记。

**论文批评**：(1) write-all 算法的平均复杂度非常数，不是真正的 O(1) 可扩展哈希表；(2) 实际性能取决于 write-all 实现，未经验证。

### 10. Lea [2003] — 分段锁 ConcurrentHashMap

Java `java.util.concurrent` 包的 `ConcurrentHashMap`，使用**分段锁（segment locking）**：将 bucket 数组分为多个 segment，每个 segment 一把 ReentrantLock。查找（get）完全无锁（利用 volatile + 不变性），插入/删除只锁目标 segment。Resize 时按 segment 逐个分裂，只锁当前 segment，允许并发查找。

**论文对比**：(1) 低负载下性能相当；(2) 高负载下论文的无锁算法显著优于 Lea 的分段锁版本。且论文的算法在 resize 时不阻塞任何操作，而 Lea 不允许 resize 期间的并发插入/删除。

### 11. Herlihy 等人 [2002] — 内存管理方案

被引用作为无锁链表内存回收的参考（与 hazard pointers / EBR 相关），配合 Michael 的链表算法使用。

### 12. Valois [1995] — Obstruction-free 链表算法

不依赖 CAS，使用更弱的同步原语。被提及作为 split-order 链表底层实现的备选方案之一（但论文选择 Michael 的算法，因其性能更好且与内存管理方案配合良好）。

### 13. Luchangco 等人 [2003] — 非阻塞链表算法

另一个 obstruction-free 链表实现，同样被提及为可选底层实现。

### 14. Herlihy & Wing [1990] — 可线性化（Linearizability）

定义了线性化（linearizability）的并发正确性条件：每个操作可在其调用和返回之间分配一个**线性化点**，所有线性化点构成一个全序，该全序等价于某个合法的顺序执行。Michael 的链表算法和论文的哈希表均满足这一性质，这是线程安全的理论基础。

### 15. Hesselink 等人 [2001] — Write-all 算法

被 Gao 引用用于 resize 时的元素迁移。Write-all 问题要求多个处理器在某个可能崩溃的环境中共同完成一组写操作。论文批评该算法的平均复杂度非常数。

### 16. Kanellakis and Shvartsman [1997] — Write-all 问题

与 Hesselink 等人配合，被论文引用作为 write-all 问题复杂度的理论分析来源。

### 17. Cormen 等人 [2001] — 算法导论

被引用作为可扩展哈希表基础定义（均摊分析、期望 O(1) 操作等）的标准参考。
