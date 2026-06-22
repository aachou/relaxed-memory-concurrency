# Summary

引入 `crossbeam-deque` crate，提供 Chase-Lev 双端队列的实现。

# Motivation

工作窃取双端队列是调度器中的常见构建块，例如在 [Rayon](https://github.com/nikomatsakis/rayon/) 和 [futures-pool](https://github.com/carllerche/futures-pool) 中。目前，这些 crate 使用的是 [Coco](https://github.com/stjepang/coco) 中的 deque，而 Coco 是一个实验性 crate，即将被弃用并由 Crossbeam 取代。

`crossbeam-deque` 将使用新的 Crossbeam 基于 epoch 的垃圾收集器。

# Detailed design

实现的基础是 Coco 中的 deque，它已经在 Rayon 以及随后的 Stylo 中得到了良好的验证。但会有一些不同之处：

1. 内存管理使用 `crossbeam-epoch`。
2. `Worker`/`Stealer` 接口重新设计为 `Deque`/`Stealer`。
3. 引入了一些新的便捷方法。

除此之外，`push`、`pop` 和 `steal` 等关键（也是实现最棘手的）方法不偏离原始实现（除了与移植到 Crossbeam 新的基于 epoch 的 GC 相关的更改）。

## The interface

```
/// 双端队列的"worker"端。
pub struct Deque<T>;

unsafe impl<T: Send> Send for Deque<T> {}

impl<T> Deque<T> {
    /// 创建一个新的双端队列。
    pub fn new() -> Deque<T>;

    /// 创建一个具有指定最小容量的新双端队列。
    pub fn with_min_capacity(min_cap: usize) -> Deque<T>;

    /// 如果双端队列为空，返回 `true`。
    pub fn is_empty(&self) -> bool;

    /// 返回双端队列中的元素数量。
    pub fn len(&self) -> usize;

    /// 将一个元素推入双端队列的底部。
    pub fn push(&self, value: T);

    /// 从双端队列的底部弹出一个元素。
    pub fn pop(&self) -> Option<T>;

    /// 从双端队列的顶部窃取一个元素。
    pub fn steal(&self) -> Steal<T>;

    /// 为双端队列创建一个新的 stealer。
    pub fn stealer(&self) -> Stealer<T>;
}

/// 双端队列的"stealer"端。
pub struct Stealer<T>;

unsafe impl<T: Send> Send for Stealer<T> {}
unsafe impl<T: Send> Sync for Stealer<T> {}

impl<T> Stealer<T> {
    /// 如果双端队列为空，返回 `true`。
    pub fn is_empty(&self) -> bool;

    /// 返回双端队列中的元素数量。
    pub fn len(&self) -> usize;

    /// 从双端队列的顶部窃取一个元素。
    pub fn steal(&self) -> Steal<T>;
}

impl<T> Clone for Stealer<T> { ... }

/// 窃取操作的可能结果。
pub enum Steal<T> {
    /// 双端队列为空。
    Empty,
    /// 窃取了某些数据。
    Data(T),
    /// 在与其他并发操作的竞争中未能抢到数据。请重试。
    Retry,
}
```

与 Coco API 的一个有趣区别是，现在使用 `Deque::new()` 而不是返回 `(Worker<T>, Stealer<T>)` 对的全局函数来构造双端队列。此外，有两种方式创建多个 stealer：可以通过 `Deque::stealer` 分别创建每个 stealer，也可以克隆现有的 stealer——哪种方式最适合你。

另一个新增功能是 `with_min_capacity` 构造函数。双端队列会随着元素的插入和移除而动态增长和收缩。通过指定较大的最小容量，可以减少重新分配的频率。

`steal` 方法返回一个 `Steal<T>`。它在 CAS 操作失败时不会重试，而是立即返回 `Steal::Inconsistent`。这使调度器能够精细控制在高竞争情况下应该做什么。

有一个根据此 RFC 实现的开放[拉取请求](https://github.com/crossbeam-rs/crossbeam-deque/pull/1)。

# Drawbacks

无。

# Alternatives

### Hazard pointers

最终，hazard pointer 可能比基于 epoch 的垃圾收集更适合双端队列。

理论上，如果固定线程被抢占，基于 epoch 的 GC 可能会无限期地泄漏内存，而 hazard pointer 提供更严格的保证。基于 HP 的 GC 保证始终存在一个有界的尚未回收的垃圾对象数量。另一方面，基于 epoch 的 GC 的优势在于允许快速遍历链表数据结构。Chase-Lev deque 不是链表数据结构，因此选择基于 epoch 的 GC 而非基于 HP 的 GC 在这里没有任何好处。

此外，基于 HP 的 GC 将允许我们强制实现完全积极的缓冲区销毁：就像 `Arc` 一样，但没有竞争引用计数的开销。思路是，当一个线程想要销毁一个缓冲区时，它会检查所有 hazard pointer 以查看它是否仍在使用中。如果有线程正在使用它，我们将 CAS 使用该缓冲区的 hazard pointer 并将其设置为 null。当该线程注意到有人将其 hazard pointer 设置为 null 时，它将接管销毁缓冲区的责任。然后它会检查所有 hazard pointer 以查看是否仍有其他人在使用它，并继续同样的操作。

在未来的某个时间点，我们绝对应该尝试基于 HP 的垃圾收集。

### Signature of `Stealer::steal`

除了返回显式的枚举 `Steal<T>`，`steal` 函数还可以返回 `Result`，有以下两种形式之一：

1. `Result<Option<T>, StealError>`，其中 `StealError` 等同于 `Steal::Retry`。
2. `Result<T, StealError>`，其中 `StealError` 是 `Empty` 和 `Retry` 的枚举。

返回 `Result` 将允许它与 `?` 运算符以及所有其他常用组合器一起使用。然而，由于窃取是一个相当不寻常且很少使用的操作，在这种情况下人机工程学的优先级不高。

# Unresolved questions

在构建 `futures-pool` 时，**[@carllerche](https://github.com/carllerche)** 想要几个额外的方法：

1. [一次窃取多个值的方法。](https://github.com/stjepang/coco/issues/11#issuecomment-339785208)
2. [一个 `steal_when_greater` 方法。](https://github.com/stjepang/coco/issues/10#issuecomment-339785563)

虽然缺乏这些方法并非致命问题，但拥有它们仍然很好。

目前，我可能倾向于先推进当前的最小化设计，然后再讨论这些方法的具体工作方式，可能是在（如果？）我们切换到基于 HP 的 GC 之后。

总之，始终欢迎建议。