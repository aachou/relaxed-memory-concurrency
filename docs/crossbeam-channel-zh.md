# Summary

引入 `crossbeam-channel` crate，旨在成为标准库中 `std::sync::mpsc` 的全方位升级替代：在功能、便利性和性能上全面超越。

原型实现可在[此处](https://github.com/stjepang/channel)找到。另请参阅[基准测试](https://github.com/stjepang/channel/tree/master/benchmarks)，了解它与 `std::sync::mpsc` 及其他提供不同类型通道和队列的 crate 的性能对比。

# Motivation

新通道的设计由回答三个问题的努力驱动：

1. 如何修复 `std::sync::mpsc` 的缺点？
2. 如何实现 Go 通道的灵活性，尤其是其 *select* 机制？
3. 如何将 Java 各种并发队列整合到统一的通道接口中？

## Shortcomings of `std::sync::mpsc`

我整理了一份 Rust 用户一直抱怨的问题列表：

1. StackOverflow 用户[提问](https://stackoverflow.com/questions/40384274/rust-mpscsender-cannot-be-shared-between-threads)为什么 [`Sender`](https://doc.rust-lang.org/std/sync/mpsc/struct.Sender.html) 没有实现 `Sync`。
2. 在 irc.mozilla.org 的 Rust 相关频道中，经常有人来询问 [MPMC](https://botbot.me/mozilla/rust/search/?q=mpmc) 或 [SPMC](https://botbot.me/mozilla/rust/search/?q=spmc) 通道和队列。
3. Reddit 用户[寻求](https://www.reddit.com/r/rust/comments/6remch/thoughts_on_avoiding_busy_waiting_for_threads/)多线程帮助，抱怨 API"相当令人困惑"和 [`Select`](https://doc.rust-lang.org/nightly/std/sync/mpsc/struct.Select.html) 的实现"明显平庸"。还有一些关于 MPMC 通道的讨论。
4. 另一位 Reddit 用户[想要](https://www.reddit.com/r/rust/comments/3mpke2/does_rust_has_something_like_channels_in_go/cvhs5kk/) SPMC 通道。原帖询问的是 Rust 中类似 Go 的通道。
5. Rust 问题跟踪器上[报告](https://github.com/rust-lang/rust/issues/12902)了一个烦恼：如果在结构体中引用 `Receiver`，`select!` 宏无法正常工作。
6. `Sender` 不是 `Sync` 是 Hyper 用户的常见障碍，详见 [**@seanmonstar**](https://github.com/seanmonstar) 的[解释](https://github.com/rust-lang/rfcs/pull/1299#issuecomment-146361678)。
7. [**@jonhoo**](https://github.com/jonhoo) [指出](https://github.com/rust-lang/rust/issues/27800#issuecomment-313455851)Rust 中的通道选择不像 Go 那样公平。`select!` 中的 case 列表每次应随机排列。
8. [**@alexcrichton**](https://github.com/alexcrichton) [表示](https://github.com/rust-lang/rust/issues/12902#issuecomment-319510050)`select!` 宏实际上已被弃用。它可能还活着只是因为 Servo 仍在使用它。
9. 从 GitHub 上某[评论](https://github.com/rust-lang/rust/issues/27800#issuecomment-140966321)获得的赞数来看，人们真的非常想要具有便捷选择功能的类似 Go 的通道。
10. [恳求](https://github.com/rust-lang/rust/issues/27800#issuecomment-270583295)能够跨动态通道列表进行选择。
11. 选择接口[不支持](https://github.com/rust-lang/rust/issues/27800#issuecomment-300880631)指定超时。
12. 一位 Rocket 用户[想要](https://www.reddit.com/r/rust/comments/6t8sdl/help_communicating_bidirectionally_with_rocket_in/)将 `Receiver` 传递给 `.manage(...)`，但这行不通，因为 `Receiver` 不是 `Sync`。
13. [**@alexcrichton**](https://github.com/alexcrichton) [列出](https://github.com/rust-lang/rust/pull/42397#issuecomment-315867774)了 `std::sync::mpsc` 当前设计的一系列遗憾。
14. 由于 `Receiver` 不是 `Sync`，人们通过将其包装在 `Mutex` 中来[绕过](https://github.com/pingcap/tikv/pull/637/files)问题，例如：`Arc<Mutex<Receiver<Option<T>>>>`。
15. Servo 中的另一个[例子](https://github.com/servo/servo/blob/38f4ae80c4b456b89ee33543c8c6699501696c9c/components/script/dom/paintworkletglobalscope.rs#L234)：将 `WorkletExecutor` 包装在 `Mutex` 中，因为 `Sender` 不是 `Sync`。
16. [**@jdm**](https://github.com/jdm) [说](https://mozilla.logbot.info/servo/20170719#c716258)通道上的 `.len()` 方法对分析 Servo 性能很有用。
17. 为了摆脱不稳定特性，Servo 正在[寻找](https://github.com/servo/servo/issues/5286) `mpsc_select` 的替代品。
18. [`ipc-channel`](https://github.com/servo/ipc-channel) 依赖于 `mpsc_select`，因此[无法](https://github.com/servo/ipc-channel/issues/118)在稳定版 Rust 上使用。
19. [Rust 严重令 ESR 失望](http://esr.ibiblio.org/?p=7294)：*不仅如此，Rust 中最易处理的并发原语 CSP 实现的限制使其无法满足 NTPsec 的需求（只能对静态通道集合进行选择），而且它甚至有被完全移除的危险！*

我还在一篇[博客文章](https://stjepang.github.io/2017/08/13/designing-a-channel.html)中详细批评了 `std::sync::mpsc`，并就如何设计一个更好的新通道提出了建议。

以下是 `std::sync::mpsc` 所有缺点的总结：

1. 通道不是 MPMC 的（不支持多个 `Receiver`）。
2. 有界变体（`sync_channel`）很慢，且由于内部锁竞争，扩展性差。
3. `Sender` 和 `Receiver` 不是 `Sync`。
4. 没有 `send_timeout` 方法（在向已满的有界通道发送数据时很有用）。
5. [`Sender`](https://doc.rust-lang.org/std/sync/mpsc/struct.Sender.html) 和 [`SyncSender`](https://doc.rust-lang.org/std/sync/mpsc/struct.SyncSender.html) 是不同的类型，这在一定程度上造成了接口重复。
6. `select!` 只能对*接收*操作进行选择，不能对*发送*操作进行选择。
7. `select!` 没有明确的稳定化路径。
8. 动态选择（使用 [`Select`](https://doc.rust-lang.org/nightly/std/sync/mpsc/struct.Select.html)）不安全且不符合人机工程学，Servo 中的[代码](https://github.com/servo/servo/blob/fa319170ebb34afcdfc120b7c3e47fe5b1c21210/components/script/script_thread.rs#L930)就是明证。
9. 通道选择不公平（选择 case 总是以相同的顺序触发）。
10. `select!` 接口有一些令人惊讶的瑕疵。
11. 最好能在 `Sender` 和 `Receiver` 上提供 `len` 方法。

到目前为止，最流行的修补其中一些问题的权宜之计是 [chan](https://github.com/BurntSushi/chan) crate，它提供了 MPMC 通道。不幸的是，它自己也带来了一些问题，其中最突出的是性能差。

## A look at Go'"'"'s channels

通道和选择是 Go 程序的核心构建块，也是该语言成功的巨大原因之一。

Go 的通道有两种风格：无缓冲（零容量）和有缓冲（固定正容量）。Go 没有内置的无界通道。

有时它们因速度慢而受到[批评](http://www.jtolds.com/writing/2016/03/go-channels-are-bad-and-you-should-feel-bad/)，这可能有点过于严厉。如果我们将它们与 Rust 的 `std::sync::mpsc` [比较](https://github.com/stjepang/channel/tree/master/benchmarks)，它们实际上表现得相当不错。

然而，Go 真正出彩的地方在于 *select* 的多功能性：

```
// 对名为 `match` 的零容量通道上的两个操作进行选择。
select {

// 从通道 `match` 接收消息 `peer`。
case peer := <-match:
    fmt.Printf("%s received a message from %s.\n", name, peer)

// 将消息 `name` 发送到通道 `match`。
case match <- name:
    fmt.Printf("Someone has matched with %s.\n", name)
}
```

这里我们声明了一个接收操作和一个发送操作，并阻塞直到其中一个成功。这是 `std::sync::mpsc` 根本无法做到的。事实上，很难找到一种语言（或库）能提供等效的功能。

另一方面，动态选择（动态声明选择 case 而非静态声明）可能[有点棘手](https://stackoverflow.com/questions/19992334/how-to-listen-to-n-channels-dynamic-select-statement)（而且可能很慢），但 Go 的 *select* 已经设立了一个很高的标杆。

[Dmitry Vyukov](http://www.1024cores.net/) 一直在努力提高 Go 通道的性能，通过避免互斥锁上的竞争（通常称为*无锁*通道，尽管严格来说它们并不完全是[无锁的](https://en.wikipedia.org/wiki/Non-blocking_algorithm#Lock-freedom)）：

- (2013/08 - 至今) [实现](https://codereview.appspot.com/12544043/) 提交审核。
- (2014/02) 为 Go 1.3 提出的初始[提案](https://groups.google.com/forum/#!msg/golang-dev/5zcEng3yvaU/wfosgbaKuEgJ)。
- (2014/02) 设计[文档](https://docs.google.com/document/d/1yIAYmbvL3JxOKOjuCyon7JhW4cSv1wy5hC0ApeGMV9s/pub)。
- (2014/07) 一次[更新](https://groups.google.com/forum/#!searchin/golang-dev/lock-free|sort:relevance/golang-dev/0IElw_BbTrk/FtV82Tr3S00J)。
- (2014/10) GitHub [问题](https://github.com/golang/go/issues/8899)。
- (2016/03) 作为对前述[批评](http://www.jtolds.com/writing/2016/03/go-channels-are-bad-and-you-should-feel-bad/)的可能回应进行[讨论](https://groups.google.com/forum/#!searchin/golang-nuts/lock-free|sort:relevance/golang-nuts/LM648yrPpck/o1rR6AUhAwAJ)。
- (2017/04) 另一次[更新](https://github.com/golang/go/issues/8899#issuecomment-292491545)。
- (2017/05) 再次[提出](https://github.com/golang/go/issues/20351#issuecomment-301895677)作为 Go 1.8 性能回归的可能解决方案。

这个新实现已经处于悬而未决的状态好几年了。它从未完全实现，但也没有被放弃或否决。它仍可能在未来的某个时间点进入 Go 的主线。我们拭目以待。

## A look at Java'"'"'s concurrent queues

Java 在 [`java.util.concurrent`](https://docs.oracle.com/javase/8/docs/api/index.html?java/util/concurrent/package-summary.html) 中有一系列相当丰富的数据结构。我们希望将其中许多作为 Crossbeam 项目的一部分在 Rust 中实现。

Java 的并发队列：

1. [`ArrayBlockingQueue`](https://docs.oracle.com/javase/8/docs/api/java/util/concurrent/ArrayBlockingQueue.html) 是一个由数组支持的有界阻塞队列。类似于 `std::sync::mpsc::sync_channel`。
2. [`ConcurrentLinkedQueue`](https://docs.oracle.com/javase/8/docs/api/java/util/concurrent/ConcurrentLinkedQueue.html) 是一个由链表支持的无界队列。类似于 `std::sync::mpsc::channel`。
3. [`DelayQueue`](https://docs.oracle.com/javase/8/docs/api/java/util/concurrent/DelayQueue.html) 是一个延迟元素的无界阻塞队列。元素只有在其延迟到期后才能被取出。这是一种特殊的优先队列。
4. [`LinkedBlockingQueue`](https://docs.oracle.com/javase/8/docs/api/java/util/concurrent/LinkedBlockingQueue.html) 是一个基于链表的可选有界阻塞队列。在有界形式下，它很少有用，因为 `ArrayBlockingQueue` 可能无论如何都是更好的选择。
5. [`LinkedTransferQueue`](https://docs.oracle.com/javase/8/docs/api/java/util/concurrent/LinkedTransferQueue.html) 类似于 `ConcurrentLinkedQueue`，但它还具有阻塞消费者的能力。
6. [`PriorityBlockingQueue`](https://docs.oracle.com/javase/8/docs/api/java/util/concurrent/PriorityBlockingQueue.html) 是一个无界阻塞优先队列。这只是在典型优先队列之上的一层薄封装。
7. [`SynchronousQueue`](https://docs.oracle.com/javase/8/docs/api/java/util/concurrent/SynchronousQueue.html) 是一个零容量（也称为* rendezvous*）队列。相当于 `std::sync::mpsc::sync_channel(0)`。

Rust 的 `std::sync::mpsc` 已经涵盖了 (1)、(2)、(5)、(7) 的功能集，但缺少多消费者能力以及一些不适合 Rust 所有权模型的功能。例如，Java 的所有并发队列都有 `peek()` 方法，它返回一个元素的引用但不将其从队列中移除。这在 Rust 世界中显然是一个危险信号。

队列 (3) 和 (6) 有点特殊，偏离了典型通道的领域。队列 (4) 实现起来并不困难，但同时它似乎也不是特别有用。因此，这三个队列不是 `crossbeam-channel` 的重点。

# Detailed design

## Design goals

*动机*部分列举的 `std::sync::mpsc` 的所有缺点都必须修复。`crossbeam-channel` 力求外表简单朴素，内里功能强大。该通道将：

1. 是一个 MPMC 通道（允许多个 `Sender` 和多个 `Receiver`）。
2. 避免锁，在有界情况下（`sync_channel` 的情况）比 `std::sync::mpsc` 快得多。
3. 在无界情况下不比 `std::sync::mpsc` 显著慢。
4. 拥有完整的 `try_send`/`send`/`send_timeout`/`try_recv`/`recv`/`recv_timeout` 方法系列。
5. 提供 `capacity`、`len` 和 `is_empty` 等便捷方法。
6. 实现 `Sync` 的 `Sender` 和 `Receiver`。
7. 拥有简单的接口，没有像 `SyncSender` 这样的特殊类型。
8. 支持对发送和接收操作的选择。
9. 允许带超时的选择。
10. 提供简单且安全的动态选择。
11. 选择没有 `select!` 的那些瑕疵。
12. 提供更公平的选择。

## The interface

构造函数：

```
pub fn unbounded<T>() -> (Sender<T>, Receiver<T>);
pub fn bounded<T>(cap: usize) -> (Sender<T>, Receiver<T>);
```

Sender：

```
pub struct Sender<T> { ... }

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T> Sender<T> {
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>>;
    pub fn send(&self, value: T) -> Result<(), SendError<T>>;
    pub fn send_timeout(&self, value: T, dur: Duration) -> Result<(), SendTimeoutError<T>>;

    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn capacity(&self) -> Option<usize>;
    pub fn is_disconnected(&self) -> bool;
}

impl<T> Clone for Sender<T> { ... }
```

Receiver：

```
pub struct Receiver<T> { ... }

unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Sync for Receiver<T> {}

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError>;
    pub fn recv(&self) -> Result<T, RecvError>;
    pub fn recv_timeout(&self, dur: Duration) -> Result<T, RecvTimeoutError>;

    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn capacity(&self) -> Option<usize>;
    pub fn is_disconnected(&self) -> bool;

    pub fn iter(&self) -> Iter<T>;
    pub fn try_iter(&self) -> TryIter<T>;
}

impl<T> Clone for Receiver<T> { ... }
```

迭代器：

```
impl<'a, T> IntoIterator for &'"'"'a Receiver<T> { ... }
impl<T> IntoIterator for Receiver<T> { ... }

pub struct Iter<'a, T: '"'"'a> { ... }
impl<'a, T> Iterator for Iter<'a, T> { ... }

pub struct TryIter<'a, T: '"'"'a> { ... }
impl<'a, T> Iterator for TryIter<'a, T> { ... }

pub struct IntoIter<T> { ... }
impl<T> Iterator for IntoIter<T> { ... }
```

选择：

```
impl Select {
    pub fn send<T>(&mut self, tx: &Sender<T>, value: T) -> Result<(), SelectSendError<T>>;
    pub fn recv<T>(&mut self, rx: &Receiver<T>) -> Result<T, SelectRecvError>;

    pub fn disconnected(&mut self) -> bool;
    pub fn would_block(&mut self) -> bool;
    pub fn timed_out(&mut self) -> bool;
}
```

错误类型与 `std::sync::mpsc` 相同，为简洁起见此处省略定义。以下是这些错误类型的完整列表：

```
RecvError
RecvTimeoutError
TryRecvError
SendError
SendTimeoutError
TrySendError
```

`Select::send` 和 `Select::recv` 中还有两个额外的错误类型：

```
pub struct SelectRecvError;

pub struct SelectSendError<T>(pub T);

impl<T> SelectSendError<T> {
    pub fn into_inner(self) -> T;
}
```

## The three flavors

在内部，有三种通道风格（实现）。

1. 构造函数 `unbounded()` 创建基于列表的风格。
2. 构造函数 `bounded(cap)` 在 `cap > 0` 时创建基于数组的风格。
3. 构造函数 `bounded(cap)` 在 `cap == 0` 时创建零容量风格。

每种风格都是一个完全独立的通道实现，并针对自己的用例进行了优化。理论上，可以减少风格种类（或只保留一种），但代价是性能下降。

更具体地说，风格在后台看起来像这样：

```
struct Channel<T> {
    senders: AtomicUsize,
    receivers: AtomicUsize,
    flavor: Flavor<T>,
}

enum Flavor<T> {
    Array(flavors::array::Channel<T>),
    List(flavors::list::Channel<T>),
    Zero(flavors::zero::Channel<T>),
}
```

`Sender` 和 `Receiver` 只是 `Channel` 的包装器：

```
pub struct Sender<T>(Arc<Channel<T>>);
pub struct Receiver<T>(Arc<Channel<T>>);
```

要尝试向通道发送值，`Sender` 提供了 `try_send` 方法，它只是在运行时匹配三种风格并调用相应风格中的方法：

```
impl<T> Sender<T> {
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        match self.0.flavor {
            Flavor::Array(ref chan) => chan.try_send(value),
            Flavor::List(ref chan) => chan.try_send(value),
            Flavor::Zero(ref chan) => chan.try_send(value, self.case_id()),
        }
    }
}
```

`Sender` 和 `Receiver` 上的所有其他方法的工作方式非常相似。

顺便提一下，`std::sync::mpsc` 也使用了相同的在运行时匹配不同风格的思想。它实际上有四种风格：

```
enum Flavor<T> {
    Oneshot(Arc<oneshot::Packet<T>>),
    Stream(Arc<stream::Packet<T>>),
    Shared(Arc<shared::Packet<T>>),
    Sync(Arc<sync::Packet<T>>),
}
```

### List-based flavor

这种风格实现了无界通道。

我在此试验了三种不同的队列：

1. Michael-Scott 队列。
2. Dmitry Vyukov 的 [MPSC 队列](http://www.1024cores.net/home/lock-free-algorithms/queues/non-intrusive-mpsc-node-based-queue) 的一个支持 MPMC 的修改版本。
3. 一种按块分配节点的队列（类似于 [`SegQueue`](https://docs.rs/crossbeam/0.3.0/crossbeam/sync/struct.SegQueue.html)）。

实现 (1) 和 (2) 分配过于频繁（每条消息一次分配），这对性能产生了负面影响。实现 (3) 在基准测试中表现更好，因此我选择了它。

实际代码与 [`SegQueue`](https://docs.rs/crossbeam/0.3.0/crossbeam/sync/struct.SegQueue.html) 中的代码非常相似。值得指出的是，该算法并非严格意义上的无锁，但它仍然具有良好的可扩展性。这还不错，尤其是考虑到 `std::sync::mpsc` 也不是无锁的。

不过，通过选择不同的队列，可以使其变成无锁的。Michael-Scott 队列是无锁的，[FAAArrayQueue](http://concurrencyfreaks.blogspot.hr/2016/11/faaarrayqueue-mpmc-lock-free-queue-part.html) 也是如此，它按块分配节点，很可能是这里的最佳选择。然而，FAAArrayQueue 实现起来要困难得多，所以我暂时没有去碰它。但我仍保留了未来实现它的可能性。

注意，由于分配是按块进行的（每块 32 个槽位），如果这种风格的通道传输的消息非常少，分配的内存会比实际需要的多。如果通道被用作*一次性通道*（只传输一条消息），尤其如此。

内存回收依赖于 Crossbeam 新的基于 epoch 的 GC（[`crossbeam-epoch`](https://github.com/crossbeam-rs/crossbeam-epoch)）。我们可以使用其他方案——特别是 hazard pointer 会是一个很好的替代方案，甚至可能更好。

### Array-based flavor

这种风格实现了正容量的有界通道。

实现基于 Dmitry Vyukov 著名的[有界 MPMC 队列](http://www.1024cores.net/home/lock-free-algorithms/queues/bounded-mpmc-queue)，它非常快速且相对容易实现。

该队列分配一个固定容量的 `[(AtomicUsize, T)]` 缓冲区，这意味着会有 `mem::size_of::<AtomicUsize>() * capacity` 字节的开销。

Dmitry Vyukov 原始实现的一个有趣特性是，该队列不是可线性化的。例如，原始实现无法通过以下测试：

```
#[test]
fn linearizable() {
    const COUNT: usize = 25_000;
    const THREADS: usize = 4;

    let (tx, rx) = bounded(THREADS);

    crossbeam::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| {
                for _ in 0..COUNT {
                    tx.send(0).unwrap();
                    rx.try_recv().unwrap();
                }
            });
        }
    });
}
```

测试失败是因为 `rx.try_recv()` 有时返回 `Err(TryRecvError::Empty)`。问题出在 `dequeue` 函数中：如果 `dif < 0`，则立即返回 `false`：

```
if (dif == 0)
{
  if (dequeue_pos_.compare_exchange_weak
      (pos, pos + 1, std::memory_order_relaxed))
    break;
}
else if (dif < 0)
  return false;
else
  pos = dequeue_pos_.load(std::memory_order_relaxed);
```

即使 `cell->sequence_` 可能落后于 `dequeue_pos_`（意味着 `dif < 0`），这并不一定意味着队列为空。可能有一个正在进行的入队操作：`enqueue_pos_` 已经递增，但相应的 `cell->sequence_` 尚未更新。

幸运的是，这个问题很容易修复。我们只需要额外检查一下 `dequeue_pos_` 和 `enqueue_pos_` 的当前值。如果这些值匹配，那么队列确实是空的。类似的修复也应应用于 `enqueue` 函数。

经过这些小调整后，队列通过了测试。

### Zero-capacity flavor

这种风格实现了零容量通道，也称为 *rendezvous* 通道。

[Go](https://github.com/golang/go/blob/1125fae989a3016d509c23fee15793b231e5e8e1/src/runtime/chan.go) 和 [`std::sync::mpsc`](https://github.com/rust-lang/rust/blob/49bee9d09a8f8c2baf4aff7d6a46cebff0c64594/src/libstd/sync/mpsc/sync.rs) 没有专用的零容量变体实现——它只是基于数组（有界）风格中的一个特例。

由于基于数组的风格实现完全不同，`crossbeam-channel` 不能采用同样的方法。零容量风格必须从头编写。

有趣的是，可以将零容量通道视为 Java 的 [`Exchanger`](https://docs.oracle.com/javase/8/docs/api/java/util/concurrent/Exchanger.html) 的一种变体。它们之间的关键区别在于，`Exchanger` 允许任何参与的线程配对并交换任意数据，而通道则更具限制性。零容量通道有两个端，发送端的线程总是用来自接收端的 `None` 交换一个 `Some(data)`。

实现本身相当复杂且繁琐，因此我在此不深入细节。我只提一点，它在内部实际上被表示为一个特殊的、双端的 `Exchanger`。

## Blocking operations

实现并发队列并不容易，但在它们之上添加阻塞操作和选择支持则更具挑战性。尤其是在我们想要保持良好的性能的情况下。

`crossbeam-channel` 中阻塞和选择的基础是 Dmitry Vyukov 的关于 Go 中更快通道的[设计文档](https://docs.google.com/document/d/1yIAYmbvL3JxOKOjuCyon7JhW4cSv1wy5hC0ApeGMV9s/pub)。

在该文档中，阻塞发送操作实现如下：

```
void asyncchansend(Hchan* c, T val) {
    for(;;) {
        if(asyncchansend_nonblock(c, val)) {
            // 发送成功，看看是否需要解除接收者的阻塞。
            if(c->recvq != nil) {
                lock(c);
                sg = removewaiter(&c->recvq);
                unlock(c);
                if(sg != nil)
                unblock(sg->g);
            }
            return;
        } else {
            // 通道已满。
            lock(c);
            sg->g = g;
            addwaiter(&c->sendq, sg);
            if(notfull(c)) {
                removewaiter(&c->sendq, sg);
                unlock(c);
                continue;
            }
            unlock(c);
            block();
            // 重试发送。
        }
    }
}
```

注意整个操作实现为一个循环，发送操作可能会被尝试多次。

一个通道有两个等待列表：一个用于阻塞的发送者，一个用于阻塞的接收者。当发送或接收操作成功时，它会从等待列表中移除一个阻塞的接收者或发送者（分别地）并将其唤醒。

在 `crossbeam-channel` 中，阻塞发送和接收操作的工作方式非常相似：

1. 检查通道是否已关闭。如果是，返回。
2. 尝试执行操作。如果成功，返回。
3. 将此线程添加到等待列表。
4. 检查操作现在是否可能成功。如果是，跳到步骤 (6)。
5. 暂停此线程（阻塞）直到有人解除其暂停（唤醒它）。
6. 将此线程从等待列表中移除。
7. 如果超时，返回。
8. 转到步骤 (1)。

考虑当一个接收操作阻塞，然后另一个线程发送消息时会发生什么。发送者将消息推入队列并唤醒接收者，然后接收者再次尝试从队列中弹出消息。

注意 Go 的工作方式略有不同。Go 的通道不会将消息推入队列。相反，它会直接将消息传递给睡眠中的接收者，然后唤醒它。接收者只需在将自己从等待列表中移除时取走消息。这不会涉及循环或重试。

虽然非循环机制似乎更高效，但它不适用于并发队列。Go 可以这样做是因为它的队列依赖于重度加锁。

## Selection

实际上，选择与阻塞发送和接收操作类似，只是它以更通用的形式出现。正如 [*Designing a channel*](https://stjepang.github.io/2017/08/13/designing-a-channel.html) 博客文章中解释的那样，存在一个不错的完全通用选择解决方案，它：

1. 支持发送和接收操作。
2. 允许指定超时。
3. 不使用宏。
4. 拥有完全安全的 API。
5. 可以拥有动态的 case 列表。

思路是暴露出阻塞机制的*循环核心*。用户声明一个循环并在循环内列举选择 case。一旦任何一个 case 成功，循环即被中断。

例如：

```
// 对零容量通道 `(tx, rx)` 上的两个操作进行选择。
let mut sel = Select::new();
loop {
    // 从通道接收消息 `peer`。
    if let Ok(peer) = sel.recv(&rx) {
        // 成功！收到一条消息。
        println!("{} received a message from {}.\n", name, peer);
        break;
    }

    // 将消息 `name` 发送到通道。
    if let Err(err) = sel.send(&tx, name) {
        // 发送失败。重新取得消息所有权。
        name = err.into_inner();
    } else {
        // 成功！消息已发送。
        break;
    }
}
```

每次调用 `sel.{send,recv}`（使用 `Sender` 或 `Receiver`）都会访问 `sel` 内部的一个状态机。循环的执行过程如下：

1. 机器在开始时未初始化。
2. 第一次调用 `sel.{send,recv}` 激活机器。它转换到*计数*状态，并通过 `Sender` 或 `Receiver` 的 ID 记住第一个选择 case。
3. 每次调用 `sel.{send,recv}` 只是递增一个 case 计数器。
4. 当机器注意到第一个选择 case 上重复调用 `sel.{send,recv}` 时，它转换到*尝试*状态。同时，它选择一个介于 0 和 case 数量之间的随机数——这就是 `start` 索引。
5. 在循环的下一次迭代中，所有对索引小于 `start` 的 case 上的 `sel.{send,recv}` 调用只是失败。在所有其他 case 上，尝试执行发送或接收操作。
6. 循环的下一次迭代类似于 (5)，但这次只尝试索引小于 `start` 的 case。
7. 循环的下一次迭代将每个 case 添加到其对应的等待列表。同时检查是否有任何 case 在添加到等待列表后可能立即成功。
8. 如果任何 case 现在可能成功，跳到 (10)。
9. 当前线程被暂停，直到有人唤醒它（或超时）。
10. 下一次迭代将每个 case 从其对应的等待列表中移除。
11. 转到步骤 (5)。

当任何一个 case 成功时，状态机变为未初始化状态，此时循环必须被中断。循环在步骤 (5) 或 (6) 处被中断。

还有一些特殊的选择 case：

```
impl Select {
    pub fn disconnected(&mut self) -> bool;
    pub fn would_block(&mut self) -> bool;
    pub fn timed_out(&mut self) -> bool;
}
```

它们的使用方法与 `sel.{send,recv}` 非常相似。以下是一个例子：

```
let mut sel = Select::new();
loop {
    if let Ok(msg) = sel.recv(&rx1) {
        println!("{}", msg);
        break;
    }
    if let Ok(msg) = sel.recv(&rx2) {
        println!("{}", msg);
        break;
    }
    if sel.disconnected() {
        // 如果所有 send/recv 操作都以 `Disconnected` 错误失败，则激活。
        break;
    }
    if sel.would_block() {
        // 如果所有 send/recv case 都会阻塞，则激活。
        break;
    }
}
```

另一个例子：

```
let mut sel = Select::with_timeout(Duration::from_millis(100));
loop {
    if let Ok(msg) = sel.recv(&rx1) {
        println!("{}", msg);
        break;
    }
    if let Ok(msg) = sel.recv(&rx2) {
        println!("{}", msg);
        break;
    }
    if sel.timed_out() {
        // 如果从选择开始已经过了 100 毫秒，则激活。
        break;
    }
}
```

要正确使用选择，必须遵循几条重要规则：

1. 循环的每次迭代必须触发相同的一组 case，且顺序相同。
2. 不能有两个 case 使用同一个通道的同一端。
3. 一旦一个 case 成功，必须立即中断循环。

违反这些规则将导致选择行为异常，可能导致死锁。

最后，让我们尝试对存储在向量中的所有接收者进行动态选择：

```
let receivers: Vec<Receiver<String>> = ...;

let mut sel = Select::new();
let msg = '"'"'select: loop {
    for rx in &receivers {
        if let Ok(msg) = sel.recv(rx) {
            break '"'"'select msg;
        }
    }
};

println!("Received message: {}", msg);
```

在 `"'`'"'select` 循环的每次迭代中，为向量中的每个 `Receiver` 触发一个 case。一旦从其中一个接收到消息，循环以消息作为结果被中断。

## Fairness

### Fair selection

Go [语言规范](https://golang.org/ref/spec#Select_statements)解释了其 `select` 语句的工作方式。有趣的是，如果多个 case 可以同时进行，如何选择成功的 case：

> 如果其中一个或多个通信可以进行，则通过均匀伪随机选择选择一个可以进行的通信。否则，如果有 default case，则选择该 case。如果没有 default case，则 "select" 语句阻塞，直到至少有一个通信可以进行。

这点随机性确保了如果一个 `select` 语句连续执行多次，没有 case 会受到优先对待。

不幸的是，Rust 中的 `select!` 宏没有类似的保证——如果第一个 case 可以立即进行，它就会成为成功的 case。这对它下面声明的其他 case 是不公平的。

`crossbeam-channel` 通过每次随机轮转 case 的循环列表来处理这个问题，然后像第一个 case 有优先权一样继续执行。注意这并没有使选择完全公平。例如，如果有三个 case 且只有前两个可以进行，那么第一个 case 有更高的成功几率。

### The *'"'"'*'"'"'* drive-by *'"'"'*'"'"'* problem

2015 年，Russ Cox 在 GitHub 上开了一个[问题](https://github.com/golang/go/issues/11506)，关于阻塞操作在等待列表中如何被（重新）排序（这是 Go 中通道的旧实现）：

> 向一个有缓冲通道发送数据，而此时有阻塞的接收者，会将值存储在缓冲区中，唤醒一个接收者，然后继续执行。当接收者最终被调度时，它检查通道，也许它很幸运，值还在那里。但也可能不在，这时它会排到队列的末尾。
>
> 从一个有缓冲通道接收数据，会从缓冲区中复制一个值，唤醒一个阻塞的发送者，然后继续执行。当发送者最终被调度时，它检查通道，也许它很幸运，缓冲区中仍然有空间发送。但也可能没有，这时它会排到队列的末尾。

这基本上就是 `crossbeam-channel` 中阻塞的工作方式。

他争辩说这种行为很糟糕，理论上，一个阻塞的操作可能永远被饿死，即使其他发送和接收操作一直在进行。提议是改变行为，使得唤醒其他线程的线程与它完成完整的协议，完全绕开队列。

Dmitry Vyukov [指出](https://github.com/golang/go/issues/11506#issuecomment-118007672)问题并非那么明确。改变行为基本上是用公平性（延迟）换取性能（吞吐量）。

最终，几个月后行为[被改变](https://go-review.googlesource.com/c/go/+/16740)，Go 通道现在有了更公平的行为。引用该拉取请求：

> 此更改移除了我们为缓冲通道使用的重试机制。相反，任何唤醒接收者的发送者或反之亦然，都与其对应方完成完整的协议。这意味着对应方在醒来时不需要重新锁定通道。（目前缓冲通道在唤醒时需要重新锁定。）

## Benchmarks

以下基准测试绝非详尽无遗，但它们至少应该让我们对 `crossbeam-channel` 与其他通道和队列的对比有所了解。一如既往，对基准测试结果应持保留态度。

共有 7 个不同的测试：

- `seq`：单个线程发送 `N` 条消息，然后接收 `N` 条消息。
- `spsc`：一个线程发送 `N` 条消息，另一个线程接收 `N` 条消息。
- `mpsc`：`T` 个线程各发送 `N / T` 条消息，一个线程接收 `N` 条消息。
- `mpmc`：`T` 个线程各发送 `N / T` 条消息，另外 `T` 个线程各接收 `N / T` 条消息。
- `select_rx`：`T` 个线程分别向各自的通道发送 `N / T` 条消息，另一个线程通过对 `T` 个通道进行选择来接收 `N` 条消息。
- `select_both`：`T` 个线程通过对 `T` 个通道进行选择来各发送 `N / T` 条消息，另外 `T` 个线程通过对 `T` 个通道进行选择来各接收 `N / T` 条消息。

其中一些测试不适用于某些通道或队列。例如，`mpmc` 测试不适用于 `std::sync::mpsc`。同样，`seq` 不适用于零容量通道，因为它们不能包含 `N` 条消息。

常量：

- `N = 5_000_000`
- `T = 4`

所有基准测试源码和附加脚本可在[此处](https://github.com/stjepang/channel/tree/master/benchmarks)找到。

结果：

[![Graphs](https://github.com/stjepang/channel/raw/master/benchmarks/plot.png)](https://github.com/stjepang/channel/raw/master/benchmarks/plot.png)

`crossbeam-channel` 在几乎所有基准测试中要么是最快的，要么是接近最快的通道。然而，有一个有趣的情况它输给了 `std::sync::mpsc`：那就是使用无界通道的 `spsc` 基准测试。

[**@JLockerman**](https://github.com/JLockerman) 最近[改进](https://github.com/rust-lang/rust/pull/44963)了 `std::sync::mpsc` 在 SPSC 模式下的性能，现在它比 `crossbeam-channel` 快了不少。

`std::sync::mpsc` 在第一个 `Sender` 被克隆时，会从 SPSC 风格动态切换到 MPSC 风格。`crossbeam-channel` 的 `Sender` 和 `Receiver` 实现了 `Sync`，因此它不能以类似的方式在风格之间动态切换。

### Integration into Servo

我已经成功地将 Servo 从 `std::sync::mpsc` 切换到 `crossbeam-channel` 并运行。代码可以在[此处](https://github.com/servo/servo/compare/master...stjepang:crossbeam-channel)找到。请注意，代码相当凌乱，只是为了实验而编写。

根据 [**@SimonSapin**](https://github.com/SimonSapin) 的建议，我运行了 [RoboHornet](http://www.robohornet.org/) 基准测试来观察 `crossbeam-channel` 对性能的影响。基准测试结果没有显著的差异。

# Drawbacks

1. 无界变体按块分配节点。即使只有几条消息通过通道发送，每次也会分配一个完整的块。
2. 选择接口很脆弱——API 用户必须遵守几条规则。Go 的 `select` 在这方面要健壮得多，但这是因为它在语言中有一级支持。
3. 无界变体在 SPSC 场景中比 `std::sync::mpsc` 慢。
4. 编译速度慢。即使只写 `unbounded().0.send(())`，也会编译三个 `send` 实现，每种风格一个。这是因为每次调用 `send` 都会在运行时匹配风格。同样，即使只使用有界变体，`crossbeam-epoch` 也会作为依赖被引入。
   - 例证：[CSP 示例](https://github.com/stjepang/channel/blob/master/examples/csp.rs)使用一个容量为 1 的有界通道（基于数组风格），在优化模式下构建需要 2.36 秒。然而，如果列表风格和零容量风格中的所有方法体都被替换为 `unimplemented!()`，则程序构建只需 1.65 秒。
5. 没有像 `std::sync::mpsc` 中的 *oneshot* 优化。
6. 在有界情况下（基于数组风格），每个槽位分配一个 `AtomicUsize`，从而增加了内存消耗。
7. 选择仍然不是完全公平的。

# Alternatives

1. 为列表风格使用基于 HP 的 GC 而不是基于 epoch 的 GC。
2. 不只使用一对通用的 `Sender`/`Receiver`，可以为不同类型的通道提供更多类型以获得更好的优化。例如，可以有 `unbounded::Sender`/`unbounded::Receiver`，或 `spsc::Sender`/`spsc::Receiver`，甚至 `oneshot::Sender`/`oneshot::Receiver`。
3. 不在运行时匹配风格，而是使用动态分发来调用正确的方法。虽然这会减少代码大小并改善编译时间，但基准测试显示对性能有负面影响。
4. 我们可以实现一个 `select!` 宏，尽管它肯定会带来一些小麻烦。大多数需要选择的 case 都是静态的接收操作列表，因此宏可能无论如何都是有用的？
5. 只有一个单一的 `Channel` 结构，没有 `Sender`/`Receiver` 的分离。这样的通道必须通过调用 `.close()` 来显式关闭。

### Bikeshedding

1. 不用两个全局构造函数 `unbounded` 和 `bounded`，我们可以只有一个名为 `new` 的构造函数，它接受 `Option<usize>` 作为容量。传递 `Some(n)` 创建容量为 `n` 的有界通道，传递 `None` 创建无界通道。

# Unresolved questions

### Basic queues

虽然 `crossbeam-channel` 本质上是一个增强版的队列，但拥有更精简、更专业化和更快速的队列仍然是有用的。也许我们可以有单独的 crate，例如：

- `crossbeam-spsc`
- `crossbeam-spmc`
- `crossbeam-mpsc`
- `crossbeam-mpmc`

另一个非常有趣的正在开发中的 crate 是 [**@jeehoonkang**](https://github.com/jeehoonkang) 的[无等待队列](https://github.com/jeehoonkang/crossbeam-queue)。

我们需要考虑如何将一堆不同的专业化队列整合到一组合理的 Crossbeam crate 中。

### Broadcast channel

另一个经常被请求的数据结构是广播通道。这是一种特殊类型的通道，仅适用于可克隆类型（`T: Clone`）。每个 `Receiver` 都会收到*所有*发送到通道中的消息的副本。一个流行的广播通道 crate 是 [`bus`](https://docs.rs/bus/1.3.2/bus/)。

不幸的是，由于 `T: Clone` 约束，无法将广播风格适配到 `crossbeam-channel` 的接口中。

### Improving `std::sync::mpsc`

既然 `crossbeam-channel` 相对于 `std::sync::mpsc` 有如此大的改进，也许某些部分可以移植到标准库中。

例如，`sync_channel` 的糟糕性能非常突出。也许可以用 Dmitry Vyukov 的有界 MPMC 队列重写它？今年早些时候我开了一个[问题](https://github.com/rust-lang/rust/issues/41021)建议这个想法。

另一个可能性是从 `crossbeam-channel` 的选择机制中汲取灵感，使 [`Select`](https://doc.rust-lang.org/nightly/std/sync/mpsc/struct.Select.html) 变得安全。