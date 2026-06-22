# Summary

本文是对 epoch GC 实现改进的提案，旨在解决当前实现中的弱点，并增加构建更复杂数据结构所需的新功能。

# Motivation

数据结构在使用 `Atomic` 之前会先固定（pin）当前线程，用完后解除固定（unpin）。固定操作会有效阻止垃圾回收——它会阻止在线程解除固定之前销毁任何新产生的垃圾。

epoch GC 的两个核心操作是：

1. **固定（Pinning）**：固定当前线程，执行一段使用 `Atomic` 的代码块，最后解除固定。在线程固定期间产生的任何垃圾，在解除固定之前都不会被销毁。
2. **添加垃圾（Adding garbage）**：将堆上分配的对象加入 GC。一旦所有已固定的线程都被解除固定，该对象将变得不可达，从而可以安全销毁。

垃圾回收偶尔会由上述两个操作自动触发。

以下小节分析 Crossbeam 当前 epoch GC 的问题，并提出可能的解决方案。

### Slow thread pinning

当前实现固定和解固一个线程需要执行 6 次原子操作和一个完整的内存栅栏（full fence）。而 [Coco](https://github.com/stjepang/coco) 证明固定操作可以高效得多：只需 3 次原子操作和一个完整栅栏。

基准测试（在现代 Intel x86-64 上）：

```
test pin_coco      ... bench:          11 ns/iter (+/- 0)
test pin_crossbeam ... bench:          27 ns/iter (+/- 1)
```

### Long GC pauses

Crossbeam 在固定当前线程后，如果发现全局 epoch 与线程本地 epoch 不一致，就会立即收集垃圾。然后它会销毁存储在三个线程本地垃圾向量之一的全部垃圾。

线程本地的向量中可能堆积大量垃圾（数十万项）。销毁垃圾导致的暂停可能长达数十或数百毫秒。长时间暂停是不可取的，因此最好能增量式地收集垃圾。

但具体应该在何时收集、收集多少垃圾呢？一个合适的时机是在产生垃圾之后立即收集一部分。通过收集比产生的垃圾量更多的垃圾，可以确保垃圾收集速度高于垃圾产生速度。

### Garbage stuck thread-locally

另一个问题是，存储在线程本地向量中的垃圾永远不会被回收，除非所属线程退出或被固定。

更好的方案是：允许线程本地向量中只保留有限数量的项，超出部分溢出到全局共享的向量中。

非常大的对象（例如支撑 Chase-Lev deque 或哈希表的数组）应该完全避免使用线程本地向量，直接放入全局向量，这样任何线程都可以回收它们。大对象必须比小对象更积极地回收，以便更快地回收浪费的内存。

### Destructors

目前，Crossbeam 只支持延迟内存释放，不支持运行析构函数。但有时运行析构函数、甚至完全任意的函数是有意义的。

例如，跳表节点如果直接将 tower 打包在节点内部（而不是持有一个指向单独分配的 tower 的指针），则可能需要使用自定义的分配/释放函数。此时可以这样销毁节点：

```
impl<K, V> for Skiplist<K, V> {
    fn remove(&self, key: K) {
        epoch::pin(|scope| {
            let node: Ptr<Node<K, V>>;
            // ...

            let raw = node.as_raw() as usize; // 使闭包满足 Send
            scope.defer(move || {
                Node::destroy(raw as *mut Node<K, V>);
            });
        });
    }
}
```

注意 `Scope::defer` 与 Linux 内核中的 `call_rcu` 非常相似。

# Detailed design

### Thread registration

线程在第一次被固定时注册。存在一个全局的无锁单链表，用于存储线程条目。注册时，线程向链表中添加一个新条目，其中包含其线程本地数据。

当线程退出时，其线程本地存储被销毁，触发某些析构代码。线程从链表中移除自己的条目，并将其所有线程本地垃圾移入全局垃圾池。

### Tracking epochs

有一个全局共享的 `AtomicUsize` 表示全局 epoch。当线程被固定时，它加载全局 epoch 并将其存储到线程本地的 `AtomicUsize` 中，表示该线程被固定时的全局 epoch。解固线程只需将线程本地的原子变量设置为一个哨兵值即可。

偶尔，我们会检查所有当前已固定的线程是否都已看到最新的全局 epoch——如果是，就将其递增。当一个对象变为垃圾时，它会被标记上当前的全局 epoch。一旦全局 epoch 推进了两次（即与标记值相差 2），该对象就可以安全销毁。

固定操作速度很快，垃圾标记的内存开销也很小。这种方法已在 SQL Server（[pdf](http://cidrdb.org/cidr2015/Papers/CIDR15_Paper15.pdf)，第 5.2 节）和 [Coco](https://github.com/stjepang/coco) 中取得了巨大成功。

### Flushing garbage

几乎所有垃圾对象都可以归为以下三类之一：

1. **小对象**。例如队列或跳表中的节点。这类对象通常销毁速度快，占用内存少。
2. **中等对象**。B 树、基数树或哈希表桶中的节点。它们由数十个较小的对象组成。
3. **大对象**。这类对象通常非常大，足以容纳整个数据结构——例如支撑 Chase-Lev deque 的数组。如果它们运行析构函数，销毁速度可能很慢。而且由于它们占用大量内存，我们希望尽快销毁它们。

垃圾对象缓存在线程本地存储中，在存储填满之前不会被销毁。添加大型垃圾对象后，最好完全刷新存储，以免垃圾无限期地卡在那里。

为此提供了 `Scope::flush` 函数。

### The interface

```
// 固定当前线程，执行一个函数，然后解固线程。
fn pin<F, T>(f: F) -> T where F: FnOnce(&Scope) -> T;

// 返回当前线程是否已固定。
fn is_pinned() -> bool;

// 表示当前线程被固定的作用域。
struct Scope;

impl Scope {
    // 堆上分配对象 `ptr` 的延迟释放。
    unsafe fn defer_free<T>(&self, ptr: Ptr<T>);

    // 堆上分配对象 `ptr` 的延迟销毁和释放。
    unsafe fn defer_drop<T: Send + 'static>(&self, ptr: Ptr<T>);

    // 任意函数 `f` 的延迟执行。
    unsafe fn defer<F: FnOnce() + Send + 'static>(&self, f: F);

    // 将线程本地存储中的所有垃圾刷新到全局垃圾队列，
    // 尝试推进 epoch，并收集一些垃圾。
    //
    // 即使可以显式调用刷新，它也会在线程本地存储填满时
    // 或固定当前线程达到一定次数时自动触发。
    fn flush(&self);
}
```

# Drawbacks

与当前 API 不兼容。

# Alternatives

### Thread entries

线程条目不必形成链表——也可以使用其他数据结构。链表以缓存局部性差而著称。

另一种方案是使用线程条目数组。在线程注册时，我们只需遍历数组并保留一个空位。这种方法的主要好处是，遍历线程条目以检查哪些当前线程已固定可能比链表快得多，因为缓存局部性更好。但这种方法实现起来可能稍微复杂一些。

无论如何，这需要实验和基准测试。

### Epoch tracking

存在替代的 epoch 追踪方案。也许 Crossbeam 当前的方案（使用三个轮转的线程本地向量）同样可以得到改进和优化。

同样，这需要尝试。

### System-wide fences

固定线程需要发出 `SeqCst` 栅栏，代价非常高。原因是我们必须声明当前线程已被固定，确保该声明对所有其他线程可见（通过发出栅栏），然后才能继续。

另一种方法是从另一侧解决问题：当线程想要检查哪些线程已被固定时，它可以发出一个系统级屏障，实际上在所有 CPU 核心上执行 `SeqCst`。这样我们就不用在每次固定操作时执行屏障，而是在每次 epoch 推进操作时执行。

系统级屏障可以通过在相对较新的 Linux 内核上使用 `sys_membarrier` 系统调用，以及在 Windows 上使用 `FlushProcessWriteBuffers` 来发出。主要问题是可移植性。此外，为了充分利用此优化的潜力，我们不能在每次固定操作时都检查系统级屏障是否可用，而必须提供多个编译时特化的代码版本。

### Flushing garbage

在添加非常大的垃圾对象后，除了刷新之外，也许可以指定一个数值来表示该对象的"紧急程度"。例如，长度为 1 的数组紧急度为 1，长度为 1000 的数组紧急度为 1000。

然后，我们可以对线程本地存储中所有垃圾的紧急度求和，如果总和超过某个阈值，就将它们全部迁移到全局垃圾队列中。

这种机制比简单的刷新方法更精确。这无疑是一个值得探索的有趣想法，但本 RFC 已经足够复杂，我们将其推迟到以后再讨论。

### Dedicated GC threads

在 Crossbeam 中，线程协作地收集垃圾——每个线程都做一些工作。另一种收集垃圾的方法是生成一个（或多个）专用线程，其唯一工作是收集和销毁垃圾、推进 epoch 等。

# Unresolved questions

### Extreme use cases

为了实现真正的可扩展性，Crossbeam 必须在以下场景中表现良好：

1. 延迟敏感的代码。
2. 非常高的线程数（数百，甚至数千）。

针对这些场景进行优化需要大量关注和调优，但目前不是高优先级。

### Linking multiple crate versions

如果程序链接了多个版本的 `crossbeam-epoch`，将存在多个 epoch GC 运行时。这是浪费的，因为多个运行时会累积垃圾。

Rayon 曾经遇到过类似的问题，并决定将其核心提取到一个名为 `rayon-core` 的独立 crate 中，该 crate 很少更新，并且基本从不破坏向后兼容性。

也许为 `crossbeam-epoch` 采用类似的方案是个好主意。

### Destructors and local GCs

有两种并发数据结构：

1. **不允许借出元素的数据结构**。例如队列、deque 和栈。弹出元素会获取其完全所有权。无法仅仅"窥视"元素内部（即临时借用它）。
2. **允许借出元素的数据结构**。例如集合和映射。查找操作返回找到元素的引用。删除操作涉及找到元素并将其标记为已删除。

数据结构属于第一类还是第二类，对延迟析构函数有深远影响。考虑哈希集合。如果一个元素被从哈希集合中移除，它必须被添加到 GC 中进行延迟销毁。例如，哈希集合可能这样实现 `remove`：

```
impl<T> for HashSet<T> {
    fn remove(&self, value: &T) {
        epoch::pin(|scope| {
            let elem: Ptr<T>;

            // 找到元素并将其标记为已删除。
            // ...

            // 稍后销毁它。
            scope.defer_drop(elem);
        });
    }
}
```

epoch GC 会在将来的某个时间销毁垃圾，但它不保证具体什么时候销毁。由于 `T` 可能持有非静态引用，我们不能在那些引用过期后销毁 `T`。有三种解决方案：

1. **限制类型**，使其要么没有析构函数，要么没有非静态引用。示例如下：

```
pub unsafe trait DeferredDropSafe {}
unsafe impl<T: Send + Copy> DeferredDropSafe for T {}
unsafe impl<T: Send + 'static> DeferredDropSafe for T {}

impl<T: DeferredDropSafe> for HashSet<T> {
    // ...
}
```

2. **为每个 `HashSet` 实例引入本地垃圾存储**。当它被 drop 时，销毁其中包含的所有垃圾。例如：

```
struct HashSet<T> {
    // ...
    garbage: Garbage,
}

impl<T> for HashSet<T> {
    fn remove(&self, value: &T) {
        epoch::pin(|scope| {
            let elem: Ptr<T>;

            // 找到元素并将其标记为已删除。
            // ...

            // 稍后销毁它。
            self.garbage.defer_drop(elem, scope);
        });
    }
}
```

3. **在 `HashSet` 的析构函数中**，刷新所有线程的线程本地垃圾，等待全局 epoch 推进，然后等待所有垃圾被销毁。这种方法的主要问题是速度慢且使算法阻塞。

另一个可能的问题是：`HashSet::find` 到底如何返回引用？一个可能的解决方案是提供一个回调函数，在线程仍然固定时执行：

```
impl<T> for HashSet<T> {
    fn find<R, F: Fn(Option<&T>) -> R>(&self, value: &T, f: F) -> R {
        epoch::pin(|scope| {
            let elem: Ptr<T>;
            // 找到元素。
            // ...

            // 调用回调函数。
            unsafe {
                f(elem.as_ref())
            }
        })
    }
}
```

另一种方案是返回一个自定义包装器，该包装器递增元素的引用计数，然后逃出固定的作用域：

```
struct Entry<'a, T: 'a> {
    // ...
}

impl<'a, T: 'a> Drop for Entry<'a, T> {
    fn drop(&mut self) {
        if self.inner.decrement() == 0 {
            epoch::pin(|scope| unsafe {
                scope.defer_free(Ptr::from_raw(self.inner.as_raw()));
            })
        }
    }
}

impl<T> for HashSet<T> {
    fn find<'a>(&'a self, value: &T) -> Option<Entry<'a, T>> {
        epoch::pin(|scope| {
            let elem: Ptr<Element<T>>;
            // 找到元素。
            // ...

            // 尝试递增引用计数并逃出作用域。
            // 如果当前引用计数为正，递增将成功，否则失败。
            match unsafe { elem.as_ref() } {
                Some(e) if e.increment() > 0 => Some(Entry::new(self, e)),
                _ => None,
            }
        })
    }
}
```

最后，值得注意的是 `HashSet` 必须对 `T` 是不变的（invariant）。考虑以下代码：

```
impl<T> HashSet<T> {
    fn insert(&self, t: T) {
        // ...
    }
}

fn main() {
    let set: HashSet<&str> = HashSet::new();

    let s = "hello".to_string();
    let s: &str = &s;

    // `set` 的生命周期比插入的引用 `s` 的有效期更长！
    // 这是有问题的。为了防止这种情况下的插入，
    // `HashSet<T>` 中的 `T` 必须是不变的。
    set.insert(s);
}
```

### Oversubscription

如果线程数多于可用处理器，epoch GC 会积累大量垃圾。这是可以理解的，因为产生垃圾的线程更多，而且某些线程在被固定时甚至被抢占，从而拖慢了 GC。

总体而言，简单测试表明，累积的最大垃圾量大概与运行线程数呈线性关系。这还不算太糟，但需要注意的是，使用 epoch GC 的线程数最好与处理器数量大致相当。

其他内存管理方案（如 hazard pointer）受影响较小，因为它们累积的垃圾更少（且数量有界），但它们的内存消耗也随线程数线性增长。

这个问题可以通过鼓励在 Rust 中使用通用线程池抽象来缓解。Tokio/futures 和 Rayon 已经拥有成熟且通用的线程池。这在 Rust 的并发故事中仍是一个不断发展的领域，但手动生成不可预测数量的操作系统线程可能终将成为过去。
