<h1 align="center">Split-Ordered Lists</h1>
<h2 align="center">Lock-Free Extensible Hash Tables</h2>
<p align="center">Ori Shalev and Nir Shavit</p>

## 核心思想

**倒置 resize 范式**：传统哈希表 resize 是在 bucket 间移动元素；本文把元素固定在一个全局无锁有序链表中，resize 时只移动 bucket 指针，用 CAS 一次完成。

**Recursive split-ordering**：链表按 key 的**位反转值**排序。mod 2^i 哈希用低位决定 bucket 归属，反转后低位变高位，同 bucket 元素自然聚拢为连续段。分裂只需在分界点插入 dummy 节点，零元素迁移。

## 数据结构

- 一个全局链表（Michael 2002 无锁有序链表），节点存 split-order key
- bucket 数组指向各子链的 dummy 节点（永不删除）
- 分段间接索引（主数组→segment→bucket），避免大块 realloc

## 复杂度

insert / delete / find 均期望 **O(1)**，无锁（lock-free）保证。

## 评价

**优点**：首个仅用 CAS 的可扩展无锁哈希表，代码简洁，高负载下优于 Lea 的锁版本，影响广泛（Crossbeam 等继承此思路）。

**工程局限**：dummy 节点永不回收，内存随扩容单调增长；位反转在 64 位 key 上有额外计算开销；依赖模 2^i 哈希，无法泛化到素数模等分布更均匀的方案。
