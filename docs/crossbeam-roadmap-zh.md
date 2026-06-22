# Summary

本 RFC 提出了 Crossbeam 近期路线图。它描绘了项目在未来 3 到 6 个月内应该达到的目标，以及此后应前进的方向。

Crossbeam 应首先专注于构建一个强大的 GC 和一小套功能完善的数据结构：工作窃取双端队列和 MPMC 通道。重点是尽快交付最有用的数据结构。该计划特意不过于雄心勃勃。

之后，Crossbeam 将探索映射表、基数树、链表、栈、广播通道等更多内容。

# Motivation

Crossbeam 旨在构建一套专注于并发和并行的各种数据结构和工具。自成立以来，该项目提出了非常有趣的想法（"无需垃圾收集的无锁编程"）并获得了大量正面关注。但开发基本停滞不前，我们需要一个清晰的计划来推进。

目前，我们有一些性能不足、API 受限、内存泄漏或存在其他问题的数据结构。这些问题将被修复，几个月后 Crossbeam 将交付一套感觉稳固、精良和完善的数据结构。

# Detailed design

总体路线图分为几个里程碑，很可能按描述的顺序逐一实现。

### First milestone: Garbage collector

Crossbeam 的核心是其基于 epoch 的 GC，在支持所有并发数据结构方面发挥着重要作用。目前，它在几个方面存在不足：

1. 不支持析构函数。
2. 不是增量的，可能导致长时间的暂停。
3. 大量垃圾可能卡在线程的线程本地袋中。
4. 过于懒惰——例如，它几乎从不释放双端队列产生的垃圾。
5. 其 `Atomic` API 不安全。

这些问题都不容易解决，需要仔细思考。由于其他一切都依赖于它，改进 GC 具有最高优先级。

它不必立刻完美，但我们将尝试现在解决所有问题，并讨论未来可能出现的问题。

更详细的设计将在单独的 RFC 中讨论。

时间估计：1 个月

### Second milestone: Deque

当前的 deque 对于 Rayon 的用例来说慢得难以接受。另一个大问题是垃圾收集过于懒惰（线程本地袋需要至少填满 32 个 deque 缓冲区才能释放任何垃圾）。

性能可以相对容易地提升，但缓冲区回收问题依赖于 GC 改造。

更详细的设计将在单独的 RFC 中讨论。

时间估计：1 周

### Third milestone: Channels

许多 Rust 用户请求 MPMC 通道。Crossbeam 的 `MsQueue` 和 `SegQueue` 提供了非常简单的 API，看起来不像传统的通道。[chan](https://github.com/BurntSushi/chan) 等替代品速度慢且扩展性不好。还有其他一些 crate 要么提供简陋的 API，要么性能不足。

Crossbeam 可以提供（无锁或几乎无锁的）MPMC 通道，支持有界和无界变体、类通道和类队列 API、recv/send 方法的阻塞/非阻塞/超时变体，以及 `select!` 宏。

有界 MPMC 通道的基础是[此文档](https://docs.google.com/document/d/1yIAYmbvL3JxOKOjuCyon7JhW4cSv1wy5hC0ApeGMV9s/pub)。无界通道可以通过简单地将整体设计适配到 Michael-Scott 队列来实现。

这样一个功能完善的 MPMC 通道将成为 Rust CSP 风格并发的宝贵资产。

更详细的设计将在单独的 RFC 中讨论。

时间估计：1 个月

### Fourth milestone: Tests

经验告诉我们，未经仔细压力测试的无锁代码很可能包含 bug。Crossbeam 中所有（或至少几乎所有）代码必须能够在使用线程 sanitizer 时通过压力测试。

除了广泛的正确性测试外，还需要基准测试来做出关于优化的明智决策，以及衡量增量改进和性能回退。

时间估计：1 个月

### Longer-term roadmap

未来有广泛的可能性。以下是一些较简单的挑战：

- Treiber 栈。这是一个很少使用的数据结构，但非常容易实现。
- 链表。这是一个很有用的数据结构。
- 广播通道。有几个 crate 提供此类功能。我们应该研究它们，探索如何扩展它们，并可能集成到 Crossbeam 中。
- 并发向量。

以下是更困难的挑战：

- 基本的有序映射表。跳表可能是最简单的。并发 B 树也很好。需要讨论权衡和接口。
- 无序映射表（哈希表）。这是一个广泛研究的领域，有大量已发表的论文和不同的设计。
- 基数树。它们通常可以作为哈希表的替代品。它们承诺比跳表和 B 树更好的性能，甚至允许有序遍历。

还有其他一些不完全是数据结构，而是可以探索的更广泛的技术：

- 扁平组合（Flat combining）。
- 消除回退（Elimination backoff）。
- 可扩展计数器和其它类型的聚合器。
- Hazard pointer。
- 基于原子 RC 的垃圾收集。
- 只允许单个写入者但不阻塞读取者的锁。
- 并发位集。

这不是一个详尽的列表。肯定还有其他可以探索的主题。

### A note on subprojects

Crossbeam 目前包含一组大多相互独立的组件，因此它将被拆分为多个 crate：

- `crossbeam-epoch`
- `crossbeam-deque`
- `crossbeam-channel`
- 等等

主 crate `crossbeam` 保留用于汇集最常见的并发数据结构。它必须感觉稳固、稳定和完整。

鼓励在 `crossbeam-rs` 保护伞下启动新的（可能是实验性的）子项目。这类子项目（长期路线图中提到了一些想法）可以独立于总体计划开始开发。

### Similar libraries

分析其他类似库提供的内容是有帮助的：

1. [Libcds](https://github.com/khizmax/libcds) 提供了广泛的数据结构和技术。对于类似用例有多种选择，例如 8 种队列、6 种映射表，以及可互换的 RCU 式 和 hazard pointer 回收方法。
2. [Concurrency Kit](https://github.com/concurrencykit/ck) 提供基于 epoch 的回收、hazard pointer、队列、哈希表、栈、扁平组合和其他几种工具。它有更专注的理念。
3. [Folly](https://github.com/facebook/folly) 是一个大型库，但最有趣的部分是其原子位集、基于整数键的哈希映射、仅插入的通用映射和基于锁的跳表。
4. [Boost.Lockfree](https://github.com/boostorg/lockfree) 非常基础。它包含非常简单的队列和一个没有高级内存回收的栈。
5. [Intel TBB](https://github.com/wjakob/tbb) 的目标与 Crossbeam 和 Rayon 结合的目标类似。有趣的数据结构是并发向量、队列和哈希映射。它们使用加锁而不是其他高级方法。
6. [Liburcu](https://github.com/urcu/userspace-rcu) 包含双向链表、栈、队列和哈希映射。数据结构由在用户空间实现的 RCU 机制支持。这与 Crossbeam 非常相关。
7. [Java.util.concurrent](https://docs.oracle.com/javase/8/docs/api/?java/util/concurrent/package-summary.html) 包含很多东西。最有趣的数据结构可能是 `ConcurrentSkipListMap`。这是最好用的广泛可用的无锁有序映射：它提供了所有合理的方法，并尽可能少地施加限制。内存回收由 Java 的 GC 处理。

总结可以学到的经验：

1. 很容易掉入实现一长串具有略微不同权衡的类似数据结构的陷阱。Crossbeam 中已经存在这个问题的例子：我们有看起来非常相似的 `MsQueue` 和 `SegQueue`。很少有用户能回答"该选哪个？何时选？"这个问题，因此这个困难的选择成为了一个绊脚石。提供多个选项是可以的，但前提是要清楚哪个是安全默认值，哪些是面向高级用户的替代方案。在这方面，Crossbeam 将努力更像 java.util.concurrent 而不是 libcds。
2. 通过牺牲 API 灵活性或便利性来过度优化数据结构是很有诱惑力的。Crossbeam 将首先提供性能合理的并发数据结构，它们易于使用且拥有丰富、不令人意外的 API。性能永远是第二位的：如果需要更快的替代方案，也可以一并提供。

总体主题是：易用的默认值，快速的替代方案。

# Drawbacks and alternatives

Crossbeam 试图以实用的方式解决备受学术研究关注的热点难题。为 GC 或数据结构选择错误的权衡是有可能的。新的论文会不断涌现，解决问题的新方法也会被发现。

路线图的部分内容可能需要修订，甚至某些数据结构可能需要完全重新设计和重写，甚至包括 Crossbeam 的核心。因此，将努力谨慎地处理这些问题，并预期未来会有更好的实现。

另一种选择是避免集中的整体方法，而只是尝试构建所有从研究中产生的数据结构，然后进行基准测试、比较，并让库有机地发展。

# Unresolved questions

如何将目标精细地分解为更小的可交付块？