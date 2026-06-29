# 宽松内存并发同步模式

2017 年 8 月 23 日

最近我一直在分析宽松内存（又称弱内存）并发中的各种数据结构和算法。我的计划是识别那些**反复出现的同步模式**，并用这些发现的模式来解释并发程序。这篇文章总结了我到目前为止的观察。最引人注目的是，令人惊讶的是，似乎大多数并发数据结构和算法仅用**三种**同步模式就可以解释！

我为什么要做这件事？因为在我开始学习并发编程时，我迫切地需要它。我阅读了许多数据结构的源代码以及解释它们为什么正确的文章。每种数据结构看起来都有道理，其理由非常微妙且引人入胜，但总有一个问题挥之不去："他们是怎么想到这种数据结构的？"当宽松内存并发加入进来时，情况变得更加糟糕：大多数关于共享内存并发的文章——从博客文章到已发表的论文——都忽略了由于硬件和编译器优化导致的宽松行为，只说应该以某种方式在实现中正确插入 fence 来限制这些宽松行为。简而言之，至少对我来说，整个并发领域看起来就像黑魔法。我希望能有一些光照亮它。

在深入探讨我发现的同步模式之前，作为背景知识，我将首先回顾共享内存并发及其宽松行为。（对于专家：我将偏离基于"happens-before"关系的传统解释，原因我稍后会解释。）

## 共享内存并发与宽松行为

让我们从一个单线程程序开始。内存基本上**看起来像**一个从地址到值的映射：

```
memory: Map<Address, Value>
```

无论内存层次结构中是否存在缓存，无论 CPU 和编译器是否对内存的加载和存储指令进行重排序，都应该是这样的。事实上，所有这些"优化"都应该保证，对于单线程程序，内存看起来就是一个从地址到值的映射。

到目前为止一切顺利。现在让我们考虑一个多线程程序。有多个线程共享同一个内存，因此一个线程存储的值可以被另一个线程加载。不幸的是，由于所有为单线程程序引入的硬件和编译器优化，多线程程序无法享受"内存只是一个映射"这种幻想的奢侈。为了说明这一点，让我们考虑以下程序：

```
fn main() {
    let a: AtomicUsize = 0;
    let b: AtomicUsize = 0;

    let th1 = fork(|| {
        a.store(42, Relaxed);
        b.load(Relaxed)
    });

    let th2 = fork(|| {
        b.store(42, Relaxed);
        a.load(Relaxed)
    });

    assert(th1.join() == 42 || th2.join() == 42);
}
```

假设这是用一种语法类似 [Rust](https://www.rust-lang.org/) 的虚构语言编写的。`a` 和 `b` 是无符号整数（`AtomicUsize`），初始值为 `0`；派生的线程 `th1` 将 42 存储到 `a`，然后从 `b` 加载；另一个派生的线程 `th2` 将 42 存储到 `b`，然后从 `a` 加载。`Relaxed` 大致意味着这些加载和存储应该被编译成汇编中的普通加载和存储指令。断言的问题是：从 `a` 加载的值等于 42，或者从 `b` 加载的值等于 42，这两者是否总有一个成立？乍一看，这似乎是显然的，因为在加载它们之前，42 肯定已经被存储到 `a` 或 `b` 中了。**然而**，由于神秘的原因，情况并非如此，断言可能会失败——从 `a` 和 `b` 都读出了 0。[^1]

[^1]: CPU 可能会重排序 `th1` 和 `th2` 中的存储和加载指令，实际上在将 42 存储到 `a` 和 `b` 之前就从它们加载了 0。也可能是缓存的原因：每个线程都写入和读取自己的缓存，而缓存可能不同步，对特定地址中存储的值持有不同的看法。你知道吗？编译器也可能重排序这些指令，仅仅因为 CPU 可能会这样做。

这个故事的寓意是：**将内存抽象为映射的模型已经失效**，程序可以表现出更多的行为，甚至以程序员意想不到的方式。

为了驯服这些行为，程序必须被正确地**同步**。CPU 架构如 x86-64、ARMv7/8、POWER，以及编程语言如 C、C++、LLVM、Rust 都支持同步原语，而不仅仅是普通的加载和存储指令。不幸的是，这些原语实际上非常**难以**理解，用它们编程就更加困难。结果，只有一小群专家通晓这些原语并为其他人构建并发库。然而，这些专家也经常误用这些原语，更糟糕的是，ISA 和语言标准本身的规范也存在错误！

为了解决这个问题，去年我和合作者开发了[一种有前景的宽松内存并发语义](http://sf.snu.ac.kr/promise-concurrency)，它清晰地解释了 C/C++ 同步原语的语义。现在我将解释其中的一些概念，即 coherence（一致性）和 view（视图），它们共同回答了内存一致性模型的核心问题：一个线程可以从内存中读取哪些值？正如你将看到的，这些概念将在解释什么是同步以及同步模式如何工作中发挥关键作用。

### 一致性与视图

首先让我们看看内存是什么样的。在这种有前景的语义中，内存是从地址到**一组值**的映射，这些值由**时间戳**唯一排序：

```
memory: Map<Address, Map<Timestamp, Value>>
```

我们把每个值称为一条**消息**。当线程向一个地址存储值时，一条包含该值的消息被添加到该地址；当线程从某个地址加载时，返回该地址某条消息的值。这有两个含义：（1）线程可能能够同时从同一个地址读取不同的值；但（2）至少可以保证，对同一地址的写入存在"某种"全局顺序（即时间戳）。我们称之为**"一致性顺序"**（coherence order，在 C/C++ 标准术语中也称为"修改顺序"）。例如，地址 `x` 的消息可能如下所示：

```
          [x=1@10]  [x=42@20] [x=37@30] [x=2@40]  [x=3@50]
x --------+---------+---------+---------+---------+-------
```

这里有 5 条地址 `x` 的消息：时间戳 10 处的值 1，时间戳 20 处的值 42，30 处的 37，40 处的 2，以及 50 处的 3。值得注意的是，不同地址的时间戳完全无关，因为一致性顺序只针对单个地址。

在加载和存储内存时，线程必须遵循一致性顺序：一旦线程确认了一条消息，它就不能相对于一致性顺序读取或写入该消息之前的时间戳。更精确地说，每个线程维护一个**线程视图**，这是一个从地址到时间戳的映射，记录了该线程为每个地址确认的最大时间戳：

```
view: Map<Address, Timestamp>
```

线程视图与加载和存储指令的交互如下。假设线程在地址 `x` 上的视图处于时间戳 `t1`（`view[x] == t1`）。换句话说，该线程以某种方式确认了地址 `x` 在时间戳 `t1` 处的消息，而没有确认更晚的消息。当它从 `x` 读取时间戳 `t2` 处的消息时，必须满足 `t1 <= t2`，同时 `view[x]` 更新为 `t2`（因为在 `t2` 处的消息刚刚被确认）。当它向 `x` 写入一条时间戳为 `t2` 的消息时，必须满足 `t1 < t2`，同时 `view[x]` 更新为 `t2`（因为在 `t2` 处的消息自动被确认）。简而言之，**加载和存储会更新线程的视图**，并且**视图应该是非递减的**。

这种一致性条件可以说是最低限度的合理性要求，因此（几乎）所有 CPU 和编译器默认都保证这一点，即使没有任何同步原语。[^2]

[^2]: 一个显著的例外是 C/C++ 的非原子加载：它们可能不遵循一致性顺序。更多细节请参见[有前景的语义学论文](http://sf.snu.ac.kr/promise-concurrency)。另外，我听说 Alpha 处理器不保证一致性。

（对于专家：有前景的语义中的一致性条件大致相当于 C/C++ 标准中的一致性公理。详细的比较请参见[该论文](http://sf.snu.ac.kr/promise-concurrency)。）

### 什么是同步？

一致性是好的，但仅靠它，程序员无法构建并发程序。这主要是因为一致性条件一次只适用于单个地址：确认地址 `x` 的一条消息与线程在另一个地址 `y` 上的视图毫无关系。正是同步**将多个地址的确认关联起来**。

我分析了各种真实的并发数据结构和算法，并将它们使用的同步模式分为三类：**正向捎带同步**（positive piggybacking synchronization）、**负向捎带同步**（negative piggybacking synchronization）和**交错同步**（interleaving synchronization）。在这篇文章的剩余部分，我将解释这三种模式，并用一些并发数据结构作为实例进行说明。

让我们从最基本、最易于理解的模式开始：正向捎带同步。（对于专家：可以理解为 `Release`-`Acquire` 同步。我故意使用了比 C/C++ 特定术语更通用的术语，例如"正向"和"捎带"而不是"`Release`-`Acquire`"，以表明这些模式同样适用于 CPU 和其他语言。）

## 模式 1：正向捎带同步

在这种模式中，我们通过**将对一个地址的认识捎带到另一个地址**来关联多个地址。例如，考虑以下程序：

```
fn main() {
    let data: AtomicUsize = 0;
    let flag: AtomicUsize = 0;

    let th1 = fork(|| {
        data.store(42, Relaxed);
        flag.store(1, Release)
    });

    let th2 = fork(|| {
        if (flag.load(Acquire)) {
            assert(data.load(Relaxed) == 42);
        }
    });
}
```

这里，线程 `th1` 将 42 写入 `data`，然后设置 `flag`。线程 `th2` 检查 `flag` 是否被设置，如果是，则断言 `data` 始终包含 42。为了断言成功，对 `data` 变量的认识必须捎带到 `flag` 变量上。

这个同步工作由 `flag` 的加载和存储指令中的 `Release` 和 `Acquire` 排序注解完成。当一个线程以 `Release` 排序写入一条消息时，该线程的视图被注解到写入的消息中；当一个线程以 `Acquire` 排序读取一条消息时，该消息的视图被读取线程确认。为了实现这一点，我们在内存的消息上附加了一个视图：

```
memory: Map<Address, Map<Timestamp, (Value, View)>>
```

让我们看看上面的例子发生了什么。假设 `th1` 在时间戳 `10` 处存储了 `data = 42`，在时间戳 `5` 处存储了 `flag = 1`，如下面的时间线所示。在写入 `flag = 1` 的时刻，线程 `th1` 的视图是 `[data@10, flag@5]`，由于 `Release` 排序，这个视图被注解到消息 `flag@5` 中。当 `th2` 读取 `flag = 1` 时，由于 `Acquire` 排序，其视图变为 `[data@10, flag@5]`，强制后续从 `data` 的加载读取 `42`（或在更晚时间戳写入的消息）。

```
              [data=42@10]
data ---------+-----------

         [flag=1@5, view=[data@10, flag@5]]
flag ----+---------------------------------
```

### 示例：自旋锁

这种正向捎带同步模式用于实现自旋锁，它在多个线程的**临界区**（或代码区域）之间提供**互斥**。我所说的"互斥"是指（1）所有临界区之间存在一个全局全序；并且（2）一个临界区结束时的视图应该在后续临界区开始时被确认。（对于专家：这与传统的"一个临界区发生在后一个临界区之前"的要求不同。）

以下是自旋锁的实现。函数 `new()` 创建一个新的自旋锁，`lock()` 标记临界区的开始，`unlock()` 标记临界区的结束：

```
struct Spinlock {
    lock: AtomicUsize,
}

impl Spinlock {
    fn new() -> Spinlock {
        Spinlock { lock: 0, }
    }

    fn lock(&self) {
        while (self.lock.compare_and_swap(0, 1, Acquire) != 0) {}
    }

    fn unlock(&self) {
        self.lock.store(0, Release);
    }
}
```

自旋锁由一个整数（`AtomicUsize`）变量 `lock` 组成，它表示是否被某个临界区锁定。如下面的时间线所示，如果其相对于一致性顺序的最后一个值是 `0`，则表示未锁定；如果是 `1`，则表示已锁定：

```
[UNLOCKED]
     (init) (lock)-(unlock)
     [0]    [1]    [0]
lock +!!!!!!+------+-------

[LOCKED]
     (init) (lock)-(unlock)   (lock)
     [0]    [1]    [0]        [1]
lock +!!!!!!+------+!!!!!!!!!!+-----

[UNLOCKED, again]
     (init) (lock)-(unlock)   (lock)----(unlock)
     [0]    [1]    [0]        [1]       [0]
lock +!!!!!!+------+!!!!!!!!!!+---------+-------
```

`new()` 函数通过将内部 `lock` 变量赋值为 `0` 来初始化自旋锁，表示初始时未锁定。

`lock()` 函数自旋直到成功将变量从 `0` 更新为 `1`。为了确保只有一个线程能将变量从 `0` 更新为 `1`，`compare_and_swap()` 函数解决了对 `lock` 变量的竞争。为了让一个线程赢得竞争并进入临界区，该线程必须能够读取旧值（这里为 `0`），然后紧接着写入新值（这里为 `1`），且旧值和新值之间不能存在其他消息。如果是这种情况，旧值和新值之间的时间戳被标记为将来不可用（在时间线中用 `!` 表示），并返回旧值。例如，成功的 `lock()` 可以将内存从上面时间线中的 `[UNLOCKED]` 状态变为 `[LOCKED]` 状态。得益于 `compare_and_swap()` 的排他性（以及只有最后一条消息的邻居未被标记为不可用的不变性），在任何时刻，只有一个线程能成功更新 `lock` 变量。如果线程竞争失败，`compare_and_swap()` 像 `load()` 一样返回内存中的值，让 `lock()` 函数继续重试。

`unlock()` 函数简单地将 `0` 存储到 `lock` 变量。例如，`unlock()` 可以将内存从 `[LOCKED]` 变为 `[UNLOCKED, again]`。

现在让我们看看上述实现如何保证互斥。首先，由于 `unlock()` 对 `lock` 变量的存储使用了 `Release` 排序，临界区结束时的视图被注解到 `lock` 变量中。然后，由于 `lock()` 的 `compare_and_swap()` 使用了 `Acquire` 排序，这个被注解的视图在下一个临界区开始时被确认。这种对 `lock` 变量的（正向）捎带同步将前一个临界区的视图传递给了后一个临界区。

### 变体

除了 `Release`-`Acquire` 同步之外，还存在其他类型的捎带同步。理论上，你可以考虑以下几个变化维度：

- **捎带的桥梁。** 在 `Release`-`Acquire` 同步中，对捎带地址（例子中的 `flag`）的一次 `Release`-写入和一次 `Acquire`-读取形成了一个捎带的"桥梁"。我们可以称之为 WR（写-读）桥。你可以想象 RRc（通过一致性的读-读）桥，其中对捎带地址的两次读取由一致性顺序排序；RWc 桥，一次读与随后的一致性写入组成的桥；WRc 桥；以及 WWc 桥。（对于专家：你可以将 C/C++ release sequence 称为 "(WR)\* 桥"（"\*" 表示 Kleene 星号）。）

- **同步标记。** 在例子中，`Release` 和 `Acquire` 排序被注解在存储和加载指令上。相反，你可以将桥梁写入之前的某个程序点标记为 `Release`，将桥梁加载之后的某个程序点标记为 `Acquire`；我们将这些标记称为"fence"。例如，上述例子也可以这样写：

```
// ... 其余部分不变

let th1 = fork(|| {
    data.store(42, Relaxed);
    fence(Release);
    flag.store(1, Relaxed)
});

let th2 = fork(|| {
    if (flag.load(Relaxed)) {
        fence(Acquire);
        assert(data.load(Relaxed) == 42);
    }
});
```

这里，`fence(Release)` 时的视图在 `fence(Acquire)` 时被确认，从而成功断言 `data.load(Relaxed) == 42`。你也可以混合使用 `Release` 存储与 `Acquire` fence，或者 `Release` fence 与 `Acquire` 加载来达到同样的效果。

实际上，并非所有这些组合都在 CPU 和语言中实现，部分原因是它们无法高效实现。无论如何，C/C++ 支持：（1）通过带有 `Release` 和 `Acquire` 排序注解的 WR（及 WRu）桥进行的同步，即普通的 `Release`-`Acquire` 同步；（2）带 fence 的变体；以及（3）在两端都有 `SeqCst` fence 的情况下，通过 RRc、RWc、WRc、WWc 桥进行的同步，其中 `SeqCst` 是 C/C++ 中最强也最昂贵的排序。

### 注

相比于其他两种模式，正向捎带同步是相对容易理解的。许多最早的并发数据结构，例如 [Treiber 栈](http://domino.research.ibm.com/library/cyberdig.nsf/0/58319a2ed2b1078985257003004617ef?OpenDocument) 和 [Michael-Scott 队列](http://dl.acm.org/citation.cfm?id=248106)，仅依赖于这种模式。在 C/C++ 中，作为内存模型核心的"happens-before"关系也只建立在这种模式之上。虽然对于一般的宽松内存并发还没有令人满意的程序逻辑，但[针对 C/C++ 的 release/acquire 片段的程序逻辑](http://plv.mpi-sws.org/igps/)已经成功开发出来。

这种模式是"正向"的，意思是**同步依赖于正向信息**，即对一条消息的确认。对于示例程序，由于 `th2` 观察到了 `flag = 1`，它也应该观察到了 `data = 42`。下一种模式同样基于捎带，但它是"负向"的：它依赖于负向信息，即**未确认**。

## 模式 2：负向捎带同步

对于上面的示例程序，确认 `flag = 1` 意味着确认 `data = 42`。其逆否命题也成立：未确认 `data = 42` 意味着未确认 `flag = 1`。这种模式用于["sequence lock"（顺序锁）](http://www.hpl.hp.com/techreports/2012/HPL-2012-68.pdf)：一种读写锁的优化实现。注意，读写锁是一种保护数据的机制，它保证写入者之间的互斥，同时为读取（但不写入）受保护数据提供了一种专门优化的方法。为了正确性，读取必须是**原子的**，即它不应观察到并发写入者对数据的中间修改。

### 示例：顺序锁

以下是顺序锁的实现。函数 `new()` 创建一个新的顺序锁，`writer_lock()` 和 `writer_unlock()` 标记写入者可以访问受保护数据的临界区的开始和结束，`read()` 返回受保护的数据：

```
struct<T> Seqlock<T: Copy> {
    seq: AtomicUsize,
    data: Atomic<T>,
}

impl<T: Copy> Seqlock<T> {
    fn new(data: T) -> Seqlock<T> {
        Seqlock { seq: 0, data: Atomic::new(data), }
    }

    fn writer_lock(&self) -> (usize, &mut T) {
        loop {
            let seq = self.seq.load(Relaxed);
            if (seq & 1 != 0) { continue };

            if (self.seq.compare_and_swap(seq, seq + 1, Acquire) != seq) { continue };

            fence(Release);
            return (seq + 2, self.data as &mut T); // 不完全符合 Rust 语法..
        }
    }

    fn writer_unlock(&self, seq: usize) {
        self.seq.store(seq, Release);
    }

    fn read(&self) -> T {
        loop {
            let seq1 = self.seq.load(Acquire);
            if (seq1 & 1 != 0) { continue };

            let result = self.data.load(Relaxed);
            fence(Acquire);

            let seq2 = self.seq.load(Relaxed);
            if (seq1 != seq2) { continue };

            return result;
        }
    }
}

// 使用 seqlock
fn main() {
    let seqlock = Seqlock::new(...);

    let th1 = fork(|| {
        let (seq_next, val) = seqlock.writer_lock();
        ... // 写入者的临界区
        seqlock.writer_unlock(seq_next);
    });

    let th2 = fork(|| {
        let val = seqlock.read();
    });
}
```

`new()`、`writer_lock()` 和 `writer_unlock()` 函数保证互斥的原因与自旋锁相同，但使用偶数（而不是 `0`）表示未锁定状态，奇数（而不是 `1`）表示锁定状态。以下是 `seq` 变量的示例时间线。例如，一个写入者将 `seq` 从 0 更新为 1，然后将 `2` 写入 `seq`。我们称之为 `W2`。类似地，将 `4` 写入 `seq` 的写入者称为 `W4`，以此类推：

```
                                             (R4)
    (init) (W2: lock)-(unlock) (W4: lock)----(unlock)  (W6: lock)--(unlock)
    [0]    [1]        [2]      [3]           [4]       [5]         [6]
seq +!!!!!!+----------+!!!!!!!!+-------------+!!!!!!!!!+-----------+-------
```

`read()` 函数之所以是原子的，原因如下。假设一个读取者 `R4` 观察到 `seq1 = seq2 = 4`。我将证明 `R4` 读取的是 `W4` 写入的数据。

首先，`W4` 结束时的视图通过正向捎带同步——从 `writer_unlock()` 的 `Release`-写入（`self.seq.store(seq, Release)`）到 `read()` 的 `Acquire`-加载（`let seq1 = self.seq.load(Acquire)`）——被传递到 `R4` 的开始。特别地，我们知道 `W4` 结束时在数据上的视图 ≤ `R4` 开始时在数据上的视图。

其次，`R4` 结束时在数据上的视图 ≤ `W4` 结束时在数据上的视图。否则，`R4` 读取的部分数据来自比 `W4` 更晚的写入者。通过从更晚写入者的 `writer_lock()` 的 `fence(Release)` 到 `read()` 的 `fence(Acquire)` 的正向捎带同步，`seq = 5` 或更晚的消息本应在 `read()` 的 `fence(Acquire)` 之后才被确认。但这会产生矛盾，因为读取者观察到 `seq2 = 4`。换句话说，通过数据上的负向捎带同步，`seq2 = 4` 意味着 `R4` 结束时在数据上的视图 ≤ `W4` 结束时在数据上的视图。

因此，在整个 `R4` 的执行过程中，它在数据上的视图应该等于 `W4` 结束时在数据上的视图。所以 `R4` 精确地读取了 `W4` 完全写入的数据。

（对于专家：值得注意的是，`R4` 并不发生在 `W6` 之前：我们只知道 `R4` 在数据上的视图 ≤ `W6` 开始时在数据上的视图。这恰好足以使读写锁正确。在我看来，某些数据结构（包括顺序锁）的规范用视图比用 happens-before 关系更自然地表达。这就是我更喜欢用视图来解释同步模式的原因。）

### 注

回想一下，在目前介绍的同步模式中，对一个地址的认识被捎带到另一个地址，复用了"另一个地址"的一致性顺序作为桥梁。但在某些情况下，我们需要更强的同步——通过**对来自不同线程的任意程序点进行排序**。这是交错 fence 的职责。

## 模式 3：交错同步

交错 fence（C/C++ 中的 `SeqCst` fence，以及 CPU 架构中最重量级的 fence）标记了需要被全序化的程序点。当线程 `th1` 在另一个线程 `th2` 执行另一个交错 fence 之前（按全序）执行了一个交错 fence 时，`th1` 在执行 fence 之前的视图应该被 `th2` 在执行 fence 之后确认。为了实现这一点，有前景的语义为交错 fence 维护了一个全局视图：

```
static mut interleaving_view: View // 在 Rust 中需要 `unsafe` 访问器，但..
```

当一个线程执行交错 fence 时，它计算其视图和全局交错视图的（逐地址）最大值，并将该最大值同时设置为其自身视图和全局交错视图：

```
fn execute_interleaving_fence(&mut thread) {
    let view = max(thread.view, interleaving_view);
    thread.view = interleaving_view = view;
}
```

交错同步非常强大：事实上，它涵盖了两种形式的捎带同步。然而，在我看来，它比捎带同步更难推理，因为分析所有可能的交错时会发生组合爆炸。我建议只在确实需要交错能力时才使用这种模式。

这是在所谓的"顺序一致性语义"或"交错语义"中唯一支持的同步类型，其中所有指令默认都是交错的（因此不允许宽松行为）。出于这个原因，我认为顺序一致性语义至少和宽松内存并发语义一样难以推理。我知道你们中的许多人在这个问题上不会同意我的看法；几十年来，顺序一致性一直被认为是共享内存并发的理想和最简单的语义。但我认为情况已经发生了一些变化：现在我们可以用视图来解释同步模式了。

交错同步的用途包括 [Peterson 算法](https://en.wikipedia.org/wiki/Peterson%27s_algorithm)（一种早期的互斥算法）和 [Chase 与 Lev 的工作窃取 deque](http://www.di.ens.fr/~zappa/readings/ppopp13.pdf)。在本节的剩余部分，我将更详细地分析 Peterson 算法。

### 示例：Peterson 互斥算法

以下是 Peterson 互斥算法的实现。与自旋锁和顺序锁不同，我这里介绍的 Peterson 算法只支持两个线程：

```
fn main() {
    let flag: [AtomicBool; 2];
    let turn: AtomicUsize = 0;

    fn lock(id: Usize) {
        flag[id].store(true, Relaxed);
        fence(SeqCst);                 // A
        turn.store(1 - id, Relaxed);
        fence(SeqCst);                 // B
        while (flag[1 - id].load(Acquire) && turn.load(Relaxed) == 1 - id) {}
    }

    fn unlock(id: Usize) {
        flag[id].store(false, Release);
    }

    let th0 = fork(|| {
        lock(0);
        // 临界区
        unlock(0);
    });

    let th1 = fork(|| {
        lock(1);
        // 临界区
        unlock(1);
    });
}
```

Peterson 算法保证互斥的原因如下。在 `th0` 和 `th1` 对 `lock()` 的调用中，有四个 `SeqCst` fence：`th0` 的第一个 fence（`A0`）、`th0` 的第二个 `fence(SeqCst)`（`B0`）、`th1` 的第一个 fence（`A1`）和 `th1` 的第二个 fence（`B1`）。我们将分析这些 fence 的每一种可能的顺序，但不失一般性，只需分析以下顺序就足够了：

- `A0` -> `B0` -> `A1` -> `B1`。

  根据交错性质，`flag[0] = true` 和 `turn = 1` 应在 `A1` 之后被确认。因此 `th1` 应该将 `turn = 0` 写入到一致性顺序中的 `turn = 1` 之后，并且它应该自旋直到 `th0` 在 `unlock()` 中将 `flag[0] = false`。通过对 `flag[0]` 的正向捎带同步，`th0` 临界区结束时的视图被传递到 `th1` 临界区的开始。

- `A0` -> `A1` -> `B0` -> `B1` 或 `A0` -> `A1` -> `B1` -> `B0`。

  根据交错性质，`flag[0] = true` 和 `flag[1] = true` 应在 `B0` 和 `B1` 之后都被确认。不失一般性，假设 `th0` 将 `turn = 1` 写入在 `th1` 将 `turn = 0` 写入之前（按一致性顺序）。那么 `th1` 的 `lock()` 应该自旋直到 `th0` 在 `unlock()` 中将 `flag[0] = false`。通过对 `flag[0]` 的正向捎带同步，`th0` 临界区结束时的视图被传递到 `th1` 临界区的开始。

## 案例研究：Crossbeam

到目前为止，我识别了三种宽松内存并发同步模式，并用一些并发数据结构混合这三种模式进行了解释。现在让我们分析一个更大的项目：[Crossbeam](https://github.com/crossbeam-rs) 库，它用 Rust 实现了基于 epoch 的并发内存回收方案。关于这个库的更多信息，我推荐你阅读 [Aaron Turon 对 Crossbeam 的介绍](https://aturon.github.io/blog/2015/08/27/epoch/)。我撰写了一份 [Crossbeam RFC](https://github.com/crossbeam-rs/rfcs/blob/master/text/2017-07-23-relaxed-memory.md)，解释了为什么 Crossbeam 的实现相对于 C/C++ 内存模型是正确的。（事实上，我在这里介绍的许多想法都是在编写这个 RFC 时构思的。）我相信你现在可以阅读它，并找出 Crossbeam 在何处、以何种方式使用了哪些模式！

## 未来工作

我试图尽可能全面地识别同步模式，但可能还有更多的模式有待发现。最值得注意的是，我省略了一些新兴的同步原语，它们有潜力成为新的、更快的同步模式的基础：

- **数据依赖的捎带同步。** 例如，C/C++ 中的 `Consume` 加载是 `Acquire` 加载的一个变体，其中 `Acquire` 的效果（确认被 `Release` 的视图）仅对依赖于 `Consume`-加载值的指令生效。例如，考虑以下程序：

  ```
  fn main() {
      let data: AtomicUsize = 0;
      let ptr: AtomicUsize = 0;

      let th1 = fork(|| {
          data.store(42, Relaxed);
          ptr.store(&data as usize, Release)
      });

      let th2 = fork(|| {
          let p = ptr.load(Consume) as &AtomicUsize;
          if (!p.is_null()) {
              assert(p.load(Relaxed) == 42);    // 依赖于 `p`，安全
              assert(data.load(Relaxed) == 42); // 独立于 `p`，不安全
          }
      });
  }
  ```

  这里，`ptr` 使用 `Consume` 注解读取。由于断言 `p.load(Relaxed) == 42` 依赖于从 `ptr` 读取的值 `p`，捎带同步发生，断言应该成功。另一方面，由于断言 `data.load(Relaxed) == 42` 在**语法上**不依赖于 `p`，捎带同步不会发生，`data.load(Relaxed)` 可能读取到初始值 `0` 导致断言失败。

  在 ARM 和 Power 等宽松 CPU 架构上，`Consume` 或 `READ_ONCE` 比 `Acquire` 更快，并且实际上被 Linux 内核使用（以 `READ_ONCE` 的名义）。不幸的是，我们目前[还不知道一个好的语义](http://www.open-std.org/jtc1/sc22/wg21/docs/papers/2016/p0371r1.html)。然而，我相信其用法属于正向/负向捎带同步模式。

- **强同步的加载/存储指令。** C/C++ 允许对加载和存储指令使用 `SeqCst` 注解。这些指令旨在比仅用 `Release` 和 `Acquire` 注解的指令具有更强的同步性。甚至，为了支持它们的更高效编译，ARMv8 架构引入了 `LDA`（load-acquire）和 `STL`（store-release）指令（及其变体）。然而，正如[这篇论文](http://plv.mpi-sws.org/scfix/)所讨论的，`SeqCst` 加载和存储的语义已经被严重破坏，迄今为止提出的唯一修复方案也过于复杂。修复其语义并识别其使用模式是一个重要的未来工作。

- **系统级同步。** Linux 上的 `sys_membarrier` 系统调用和 Windows 上的 `FlushProcessWriteBuffers` 本质上在所有 CPU 核心上执行 `SeqCst` fence。正如[一篇 Crossbeam RFC](https://github.com/crossbeam-rs/rfcs/blob/master/text/2017-05-23-epoch-gc.md#system-wide-fences) 所讨论的，这些系统级 fence 可以用来消除关键路径上的 fence，代价是在冷路径上引入系统级 fence。识别其使用模式也是一个重要的未来工作。

- **异构系统的同步。** 到目前为止，我只关注板载 CPU 之间的同步，但如今越来越多的硬件设备——包括 GPU、NIC（网络接口控制器）卡，可能还有 TPU（张量处理单元）——正在相互同步。在这些异构系统中是否存在独特的使用模式？

## 结论

我希望这篇文章达到了它的目的：通过识别同步模式阐明宽松内存并发的本质，从而帮助你开始编写并发程序。我希望并发对你来说不再像两年前对我那样是黑魔法，而是现代系统编程中一门有章可循的学科。祝并发编程愉快！

### 致谢

我要感谢 @foollbar、@stjepang、@Vtec234、Benjamin Fry、Derek Dreyer 和 Gil Hur 对本文早期版本提出的宝贵意见。
