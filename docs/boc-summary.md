# BoC 论文总结

Behaviour-Oriented Concurrency（BoC）通过 `when` 原语统一并行与协调。程序状态分解为隔离的 cown（并发拥有者）；behaviour（行为）是异步工作单元，声明所需 cown 后原子获取、执行、释放。BoC 保证无数据竞争、无死锁，支持跨多资源协调，性能与 Actor 相当甚至更优。

## 1. 背景与动机

并行与协调的设计决策紧密交织，但现有机制往往将两者解耦：线程/fork-join 擅并行不擅协调，锁/事务擅协调但易死锁。Actor 模型用隔离 actor 自然产生并行，但**跨多 actor 的原子操作极其困难**。BoC 用 `when` 原语统一两者。

## 2. 核心概念

**Cown**：保护隔离数据，唯一入口点。状态可用或已被获取。类型为 `cown[T]`，不能直接访问内部数据。

**Behaviour**：并发执行单元，含 cown 列表和闭包。执行前原子获取全部 cown，执行期间独占，执行完一次性释放。不能运行中获取或提前释放 cown。

**`when` 表达式**：声明所需 cown，立即返回，闭包异步执行。可嵌套，嵌套语义与顶层一致。

## 3. 关键语义

**Happens-before**（Definition 2.1）：b 发生在 b′ 之前 ⟺ cowns(b) ∩ cowns(b′) ≠ ∅ 且 b 在 b′ 之前创建。happens-before 是创建全序的子集（仅保留 cown 重叠的对），故必然无环。

**无数据竞争**：cown 状态隔离 + 行为独占访问 → 不可能同时修改同一状态。

**无死锁**：原子获取（无 hold-and-wait）+ happens-before 无环。

**性能 vs 正确性**：BoC 的设计错误影响性能而非正确性。不必要的 cown 重叠导致串行化（性能差但不会死锁）。对比锁（死锁/数据竞争）和事务内存（回滚后串行化），BoC 的优势在于 happens-before 是源代码级语义，可观察到并修复。

## 4. 形式化模型

**底层语言**：元组 `(Context, Heap, ↪, finished)`，极少约束 Context/Heap 结构。

**BoC 扩展**：新增 Tag（cown 标识符）、behaviour 生成关系 ↪\_{when(κ){E}}、配置 P（挂起序列）× R（运行集合）。

| 规则 | 作用 |
|------|------|
| **step** | 运行中行为执行一步，更新堆和上下文 |
| **spawn** | 创建行为，将 (κ′, E′′) 追加到 P 末尾 |
| **run** | 条件：P 中无更早行为 cown 重叠，且 cown 不被 R 占用 → 移入 R |
| **end** | 行为终止后从 R 移除 |

**无死锁证明**：R 非空则始终可推进（step/spawn/end）；R 空 P 非空则 run 移入行为进 R；R 空 P 空则不存在行为，程序结束。

## 5. 实现

**依赖图（DAG）**：替代线性 P。节点 = behaviour，边 = Request = happens-before。根节点（无前驱）可立即执行。每个 cown 指向链上最后一个 Request。

| 语义 | 实现 |
|------|------|
| P | 分布式 DAG |
| run | 从根节点调度 |
| spawn | 添加为 cown 链叶子节点 |
| end | 从后继前驱集合移除自己 |

**C# 实现**：CownBase（last）、Request（next/scheduled/target）、Behaviour（thunk/requests/count）。两阶段原子入链：阶段一 Exchange 设 cown.last 并链接前驱，阶段二设 scheduled 告知后继。`count = #cowns + 1` 门控确保第二阶段完成前行为不会启动。Release 处理三种情况（next 已设 / 链尾 CAS 清空 / CAS 失败等 next）。

**C++ 优化**：单次分配打包所有对象、位域编码 next+scheduled、工作窃取调度器（线程本地批量执行同一 happens-before 链，n 次后强制提交防饿死）。

**正确性**：图操作间原子无死锁（两阶段入链 + CAS）；thunk 由 cown 互斥保证（cown 重叠则串行，不重叠则操作独立数据）；图操作与 thunk 内存不相交（thunk 只碰 cown 数据，图操作只碰 next/count/last）→ 行为间原子无死锁。

## 6. BoC 与 Actor 对比

| 维度 | Actor | BoC |
|------|-------|-----|
| 基本单元 | Actor（封装状态+行为） | Cown（状态）+ Behaviour（行为）分离 |
| 跨资源操作 | 困难，需链式回调/Saga 等 | 自然支持 `when(c1, c2)` |
| 执行顺序 | 同一 Actor 内消息按入队顺序执行 | 重叠 cown 的 behaviour 按 happens-before 排序 |
| 代码紧凑度 | 跨 actor 操作需复杂编排 | 嵌套 + 多 cown 可紧凑表达 |

评估（Savina）：使用 BoC 替代 Actor 可以更紧凑地表达，而且性能相当甚至更好。

## 7. 贡献总结

1. **BoC 范式**：统一原语提供并行和灵活协调，有序执行 + 跨多资源原子操作
2. **形式化模型**：操作语义定义执行规则，证明原子性、无数据竞争、无死锁
3. **高效 C++ 实现**：MCS 依赖图 + 两阶段入链 + 工作窃取调度器，近乎完美扩展
4. **实证验证**：Savina 基准测试验证实用性和性能竞争力

