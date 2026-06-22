# Summary

本文提出在 epoch GC 背景下设计一套更好的原子操作 API。

该 API 处理指向受 epoch GC 保护的堆分配对象的指针。共有两类指针：

1. 原子指针，通常存在于堆上。
2. 栈上指针，可以从原子指针中加载，也可以存入原子指针。

# Motivation

Crossbeam 当前的 API 存在以下问题：

1. 不安全（safe 代码允许数据竞争）。
2. 灵活性不足（例如缺少操作裸指针的方法）。
3. 对性能的控制不完整（例如 CAS 在失败时不加载当前值）。
4. 缺少指针标记（pointer tagging）功能（实现链表等结构时需要）。
5. 人机工程学有待改进。

本提案旨在解决所有这些问题。一次性解决所有问题需要在众多权衡之间谨慎平衡，这是一项艰巨的任务，但将为未来的工作奠定重要基础。

详细分析如下。

### Pointer types

Crossbeam 目前有以下用于处理原子操作的类型：

1. `Atomic` —— 等价于 `AtomicPtr`。
2. `Owned` —— 等价于 `Box`。
3. `Shared` —— 等价于引用。

此外还有一个 `Guard` 类型。Guard 的存在证明了当前线程已被固定（pinned）。换句话说，`epoch::pin()` 返回一个 `Guard`，可以传递给需要此类证明的方法。

在人机工程学方面存在一个问题：`Atomic` 上的方法接受和返回 `Option<Owned<T>>` 和 `Option<Shared<T>>` 类型。

这是主观判断，但基于实际经验：这些可选类型带来的好处微乎其微，反而显得笨重且不必要。

考虑当前 API 中的以下方法：

```
fn cas(&self, old: Option<Shared<T>>, new: Option<Owned<T>>, ord: Ordering)
    -> Result<(), Option<Owned<T>>>;
fn cas_shared(&self, old: Option<Shared<T>>, new: Option<Shared<T>>, ord: Ordering)
    -> bool;
```

首先，有两种方式安装空指针：

```
atomic.cas(old, None, SeqCst);
atomic.cas_shared(old, None, SeqCst);
```

第一个方法中的 `Option<Owned<T>>` 没有实际用途，可以简化为：

```
fn cas(&self, old: Option<Shared<T>>, new: Owned<T>, ord: Ordering)
    -> Result<(), Owned<T>>;
```

同时，让 `Shared<T>` 在内部封装"可选性"，使其自身就能表示空引用：

```
fn cas(&self, old: Shared<T>, new: Owned<T>, ord: Ordering) -> Result<(), Owned<T>>;
fn cas_shared(&self, old: Shared<T>, new: Shared<T>, ord: Ordering) -> bool;
```

现在，要使用 CAS 安装空指针，可以这样做：

```
atomic.cas_shared(old, Shared::null(), SeqCst);
```

这种简化消除了冗余，也通过消除将共享引用包装到 `Option` 的需要，使 API 更易于使用（主观看法）。

此外，在内部处理"可选性"对于表示带标记的空指针也是必要的。指针标记将在下一节介绍。

一个缺点是，我们失去了一种表示非空共享指针的方式。但在实践中这不是问题。引入这样一个类型并不困难，但很可能根本不值得费心（同样是主观看法）。

由于 `Shared` 现在可以为空，调用 `Shared::null()` 可以在不固定的情况下创建它。换句话说，`Shared` 的存在曾经是当前线程已固定的证明，但现在不再是了，因此像 `cas_and_ref` 这样的方法必须接受一个 guard：

```
fn cas_and_ref<'g>(&self, old: Shared<T>, new: Owned<T>, ord: Ordering, _: &'g Guard)
    -> Result<Shared<'g, T>, Owned<T>>
```

### Pointer tagging

有一个[开放的 PR](https://github.com/crossbeam-rs/crossbeam/pull/70) 引入了带标记的原子指针。大多数指针有几个未使用的低有效位，可以携带一些信息。这对于构建基于列表的数据结构非常有用，例如链表和跳表。

问题来了：普通原子指针和带标记原子指针是否需要不同的类型？正如 [coco](https://github.com/stjepang/coco) 所展示的，这种区别并非必要。我们可以将标记作为 `Atomic`/`Owned`/`Shared` 的内置功能，这样不使用标记的指针就用零标记。标记是一个不会在未使用时造成干扰的功能。

将标记功能内置的一个缺点是，带标记的指针有一些开销：

1. 创建新的 owned 指针时，必须断言它是对齐的。
2. 加载和解引用指针时，必须清除低有效位。

但这种开销实际上可以忽略不计（无论在代码大小还是运行时成本上），归结为简单的位运算。我们还必须强制指针适当对齐，但这并非严重的限制。

如果不需要标记，用户甚至不必知道它的存在。标记是一个非常简单的功能——它只在 `Owned` 和 `Shared` 上引入了两个方法：

```
fn tag(&self) -> usize;
fn with_tag(self, tag: usize) -> Self;
```

### Soundness hole

Aaron Turon 的[博客文章](https://aturon.github.io/blog/2015/08/27/epoch/)在介绍 Crossbeam 时指出：

*"获取快照后，无需使用 unsafe 即可解引用它，因为 guard 保证了它的存活。"*

这是不正确的。只有当我们为 load 和 store 选择了正确的内存顺序时，才能安全地解引用。

考虑以下场景。有两个线程。第一个线程执行：

```
a.cas(Shared::null(), Owned::new(777), Relaxed, &guard);
```

第二个线程执行：

```
println!("{}", a.load(Relaxed, &guard).as_ref().unwrap());
```

堆上分配的数字（777）可能不会被打印出来，因为第一个线程中的 CAS 与第二个线程中的 load 没有同步。内存顺序必须至少分别是 `Release` 和 `Acquire`。

使用宽松的内存顺序，我们可以在 safe 代码中访问未同步的非原子数据。这是不安全的，因为 Rust 不允许在 safe 代码中出现数据竞争（也不能读取未初始化的数据）。

可以通过限制 API 来解决此问题：load 只接受 `Acquire` 或更强，store 只接受 `Release` 或更强，CAS 只接受 `AcqRel` 或更强。这将保证没有数据竞争。然而，有时我们确实需要更宽松的内存顺序。设计一个灵活、符合人机工程学且安全的 API，允许真正安全的 `as_ref()`，是非常困难的。

解引用是关键问题：没有适当的同步就不安全。因此，`as_ref()` 将是一个 unsafe 方法。

这或许是个令人失望的消息。有人可能会问：如果 Crossbeam 连安全的解引用都不再支持，那么 Rust 这门语言到底带来了什么？实际上，Rust 在多个方面防止了 epoch GC 的误用：

1. 它强制正确使用 owned 指针。例如，CAS 在成功时消耗它，在失败时将其返回。这是通过 Rust 的移动语义和仿射类型实现的。
2. 它强制正确使用共享指针。共享指针仅在当前线程已固定时有效。这是借助 Rust 的生命周期实现的。

### Destructors

并发数据结构的析构函数通常只是遍历所有节点并逐个释放。例如：

```
impl<T> Drop for Stack<T> {
    fn drop(&mut self) {
        let guard = epoch::pin();

        let mut curr = self.head.load(Relaxed, &guard);
        while !curr.is_null() {
            unsafe {
                let next = curr.as_ref().unwrap().next.load(Relaxed, &guard);
                drop(Box::from_raw(curr.as_raw()));
                curr = next;
            }
        }
    }
}
```

这段代码的问题在于，它在整个析构期间固定了当前线程。大型栈的析构可能需要很长时间，因此这是很糟糕的。可以通过每次只销毁少量节点然后重新固定线程来解决。

但这种技巧在其他数据结构上，尤其是树形结构，可能更难实现。在析构函数中不固定当前线程就能加载原子指针，是更理想的做法。

如果有一种执行"伪固定"的方式就好了。如果我们知道当前线程是唯一访问该数据结构的线程，则完全不需要固定。

# Detailed design

既然我们已经讨论了当前 API 中存在的问题，那么退后一步，从头设计一个新的 API。让我们从一张白纸开始……

基于 epoch 的 GC 将有两个关键类型：

1. `Atomic<T>` —— 指向堆分配的 `T` 的原子指针，通常存在于堆上。
2. `Ptr<'scope, T>` —— 指向堆分配的 `T` 的指针，存在于 `'scope` 内的栈上。

加载 `Atomic<T>` 返回一个新的 `Ptr<'scope, T>`，可以解引用（如果不为空）。示例：

```
let a = Atomic::new(7); // 指向堆上分配的数字 7 的原子指针
let p = a.load(SeqCst);
```

但这还不行！基于 epoch 的 GC 完全围绕作用域（scope）展开。在处理堆分配的对象时，我们不希望其他线程在我们的眼皮底下并发销毁它们。假设我们想打印这个数字：

```
let p = a.load(SeqCst);
println!("{}", *p.as_ref().unwrap());
```

在 `load` 和 `println` 之间，我们确实不希望其他线程释放堆上分配的数字。有一种方式可以对 GC 说："请在此作用域结束之前，不要销毁在此期间产生的任何垃圾"：

```
epoch::pin(|scope| {
    let p = a.load(Seqcst, scope);
    println!("{}", p.as_ref().unwrap());
})
```

现在，如果另一个线程决定在 `load` 和 `println` 之间的某个时刻释放该堆分配的数字，释放操作将被简单地推迟。垃圾收集器会跟踪此类作用域，并在安全的时候销毁垃圾。

注意，所有 `Ptr` 的类型中都带有一个生命周期：`Ptr<'scope, T>`，它指示了指针在哪个作用域内可用。加载原子指针需要一个作用域，并将返回的 `Ptr` 绑定到该作用域。

示例代码仍然无法编译，因为 `as_ref()` 是 unsafe 的。只有在使用正确的内存顺序来同步对堆分配对象的写入和读取时，解引用才是安全的。最终，这样可以编译：

```
epoch::pin(|scope| {
    let p = a.load(Seqcst, scope);
    println!("{}", unsafe { p.as_ref() }.unwrap());
});
```

如果你想知道为什么这里要调用 `unwrap()`，那是因为指针可能为空。转换为引用仅在非空时成功。

`epoch::pin` 创建的作用域应该是短命的，否则 GC 可能会被阻塞很长时间，积累过多的垃圾。

然而，有时我们想要更长生命周期的作用域，并且我们知道当前线程是唯一访问原子变量的线程。例如，在销毁大型数据结构时，或者从长迭代器构建数据结构时。在这种情况下，我们不需要过度保护，因为无需担心其他线程并发销毁对象。这时我们可以这样做：

```
impl<T> Drop for Stack<T> {
    fn drop(&mut self) {
        unsafe {
            epoch::unprotected(|scope| {
                let mut curr = self.head.load(Relaxed, scope);
                while !curr.is_null() {
                    let next = curr.deref().next.load(Relaxed, scope);
                    drop(Box::from_raw(curr));
                    curr = next;
                }
            })
        }
    }
}
```

`epoch::unprotected` 函数是 unsafe 的，因为我们必须承诺没有其他线程同时访问这些 `Atomic` 和对象。该函数仅在以下情况下安全使用：

- 当前线程是唯一访问这些原子变量的线程。
- 没有线程（包括我们自己的）在修改这些原子变量。

与安全的 `epoch::pin` 函数一样，原子变量的非受保护使用也被封闭在一个作用域内，以便在其中创建的指针不会泄漏或与其他作用域的指针混淆。

如何将新对象存储到 `Atomic` 中？为此，有一个 `Owned<T>` 类型，它与 `Box<T>` 几乎相同。事实上，`Owned<T>` 甚至可以直接从 `Box<T>` 构造而来。

我们可以将 `Owned<T>` 转换为 `Ptr<'scope, T>`：

```
let mut owned = Owned::new(Node {
    value: value,
    next: Atomic::null(),
});

epoch::pin(|scope| {
    let node = owned.into_ptr(scope);
    // ...
})
```

或者，我们可以使用 compare-and-swap 将新对象安装到原子指针中。以下是将一个新的 `owned` 节点压入栈的方法：

```
epoch::pin(|scope| {
    let mut head = self.head.load(Acquire, scope);
    loop {
        owned.next.store(head, Relaxed);
        match self.head.compare_and_swap_weak_owned(head, owned, AcqRel, scope) {
            Ok(_) => break,
            Err((h, o)) => {
                head = h;
                owned = o;
            }
        }
    }
})
```

### The new interface

首先是 `Atomic` 类型。注意，只有在该方法从 `Atomic` 返回一个新加载的指针时，才需要传递作用域。

`compare_and_swap_owned` 和 `compare_and_swap_weak_owned` 方法在失败时会返回未安装的 owned 指针以及原子变量中存储的当前值。这使得 CAS 循环更高效，无需重新加载。

```
struct Atomic<T> { ... }

impl<T> Atomic<T> {
    pub fn null() -> Self;
    pub fn new(t: T) -> Self;
    pub fn from_owned(owned: Owned<T>) -> Self;
    pub fn from_ptr(ptr: Ptr<T>) -> Self;

    pub fn load<'scope>(&self, ord: Ordering, _: &'scope Scope) -> Ptr<'scope, T>;

    pub fn store(&self, new: Ptr<T>, ord: Ordering);
    pub fn store_owned(&self, new: Owned<T>, ord: Ordering);

    pub fn swap<'scope>(&self, new: Ptr<T>, ord: Ordering, _: &'scope Scope)
        -> Ptr<'scope, T>;

    pub fn compare_and_swap<'scope>(
        &self,
        current: Ptr<T>,
        new: Ptr<T>,
        ord: Ordering,
        _: &'scope Scope,
    ) -> Result<(), Ptr<'scope, T>>;

    pub fn compare_and_swap_weak<'scope>(
        &self,
        current: Ptr<T>,
        new: Ptr<T>,
        ord: Ordering,
        _: &'scope Scope,
    ) -> Result<(), Ptr<'scope, T>>;

    pub fn compare_and_swap_owned<'scope>(
        &self,
        current: Ptr<T>,
        new: Owned<T>,
        ord: Ordering,
        _: &'scope Scope,
    ) -> Result<Ptr<'scope, T>, (Ptr<'scope, T>, Owned<T>)>;

    pub fn compare_and_swap_weak_owned<'scope>(
        &self,
        current: Ptr<T>,
        new: Owned<T>,
        ord: Ordering,
        _: &'scope Scope,
    ) -> Result<Ptr<'scope, T>, (Ptr<'scope, T>, Owned<T>)>;
}
```

接下来是 `Owned`。它基本上就是一个可标记的 `Box<T>`。一个有趣的方法是 `into_ptr`，它将 `Owned<T>` 提升到"epoch 宇宙"中。

```
struct Owned<T> { ... }

impl<T> Deref for Owned<T> { ... }
impl<T> DerefMut for Owned<T> { ... }

impl<T> Owned<T> {
    pub fn new(t: T) -> Self;
    pub fn from_box(b: Box<T>) -> Self;
    pub unsafe fn from_raw(raw: *mut T) -> Self;

    pub fn into_ptr<'scope>(self, _: &'scope Scope) -> Ptr<'scope, T>;

    pub fn tag(&self) -> usize;
    pub fn with_tag(self, tag: usize) -> Self;
}
```

最后，`Ptr<'scope, T>` 类似于一个可标记的 `Option<&'scope T>`。最有趣的方法可能是 `deref` 和 `as_ref`，它们解引用指针。如上所述，它们是 unsafe 的，因为我们必须承诺 load/store/CAS 已经正确同步。

```
struct Ptr<'scope, T: 'scope> { ... }

impl<'scope, T> Clone for Ptr<'scope, T> { ... }
impl<'scope, T> Copy for Ptr<'scope, T> { ... }

impl<'scope, T> Ptr<'scope, T> {
    pub fn null() -> Self;
    pub unsafe fn from_raw(raw: *const T) -> Self;

    pub fn is_null(&self) -> bool;
    pub fn as_raw(&self) -> *const T;
    pub unsafe fn deref(&self) -> &'scope T;
    pub unsafe fn as_ref(&self) -> Option<&'scope T>;

    pub fn tag(&self) -> usize;
    pub fn with_tag(&self, tag: usize) -> Self;
}
```

作为对比，以下是旧版接口：

- [`Atomic`](https://docs.rs/crossbeam/0.2.10/crossbeam/mem/epoch/struct.Atomic.html)
- [`Owned`](https://docs.rs/crossbeam/0.2.10/crossbeam/mem/epoch/struct.Owned.html)
- [`Shared`](https://docs.rs/crossbeam/0.2.10/crossbeam/mem/epoch/struct.Shared.html)

### Example: linked list

以下是一个链表遍历的示例。已删除节点的 `next` 指针被标记为数字 1。

```
epoch::pin(|scope| {
    'retry: loop {
        let mut pred = self.head;
        let mut curr = pred.load(Acquire, scope);

        while let Some(c) = unsafe { curr.as_ref() } {
            let succ = c.next.load(Acquire, scope);

            // 当前节点是否标记为已删除？
            if succ.tag() == 1 {
                let succ = succ.with_tag(0);

                // 尝试解除链接
                if pred.compare_and_swap(curr, succ, AcqRel, scope).is_err() {
                    // 失败，从头开始重新遍历
                    continue 'retry;
                }

                curr = succ;
            } else {
                // 刚刚遍历了 `c`！

                // 移动到下一个节点
                pred = &c.next;
                curr = succ;
            }
        }

        // 完成！成功遍历整个链表。
        break;
    }
})
```

### Scopes or guards?

为什么 `epoch::pin` 接受一个闭包而不是返回一个 `Scope` 或像以前一样的 `Guard`？

首先明确一点：当前 Crossbeam 中实现的 guard 是不安全的。考虑以下程序：

```
struct Foo;

impl Drop for Foo {
    fn drop(&mut self) {
        GUARD.with(|guard| {
            let guard = guard.borrow();
            let guard = guard.as_ref().unwrap();

            // 不受保护的 load！！！
            println!("{}", *ATOMIC.load(SeqCst, guard).unwrap());
        });

        // 移除 guard 并遗忘它
        GUARD.with(|guard| mem::forget(guard.borrow_mut().take()));
    }
}

lazy_static! {
    static ref ATOMIC: Atomic<i32> = Atomic::new(7);
}

thread_local! {
    static GUARD: RefCell<Option<Guard>> = RefCell::new(None);
    static FOO: Foo = Foo;
}

fn main() {
    thread::spawn(|| {
        GUARD.with(|_| ());
        FOO.with(|_| ());
        GUARD.with(|guard| *guard.borrow_mut() = Some(epoch::pin()));

        // 1. `LOCAL_EPOCH` 被释放，线程取消注册。
        // 2. `FOO` 被释放，加载数字并打印在屏幕上。
        // 3. `GUARD` 被释放，因我们在释放 `FOO` 时已移除 guard，故无事发生。
    }).join().unwrap();
}
```

再在 `mem::epoch::local` 模块的 `LocalEpoch::drop` 中添加如下代码：

```
println!("Unregister!");
```

运行程序输出：

```
Unregister!
7
```

正如我们所看到的，线程首先被取消注册。然后我们使用线程本地 guard 加载全局原子变量并打印在屏幕上。在线程已在 epoch GC 中取消注册的情况下，我们成功访问了一个原子变量，这是错误且不安全的！

Guard 很难做到完全正确。它们当然比闭包更符合人机工程学，但闭包更明确地表明作用域中正在发生重要的事情。闭包也更难被误用。

此外，很难解释原因，但闭包在我的基准测试中表现出稍好的数据（仅为个人经验）。

也许这不是一个非黑即白的决定，但基于上述原因，闭包比 guard 更受青睐。它们更明确、更难被误用、也更容易正确实现。

# Drawbacks

我们将完全破坏与旧版本 Crossbeam 的兼容性。

# Alternatives

1. 为带标记的原子指针创建独立于 `Atomic` 的类型。
2. 区分可为空和不可为空的 `Ptr`。
3. 某些方法名称非常长，例如 `compare_and_swap_weak_owned`。它们可以缩短为 `cas_weak_owned`，但这会降低一些清晰度。毕竟，C++ 有更长的函数名，如 `atomic_compare_exchange_strong_explicit`。

# Unresolved questions

如果需要一个使用自定义分配和释放函数的 `Owned<T>`，应该怎么办？我们是否应为此类情况提供良好的接口？

提供一个用于重新固定当前线程的 unsafe 函数是有意义的，主要用于长时间运行的作用域。重新固定是 unsafe 的，因为它会使所有现有的 `Ptr` 失效。此功能还有哪些更具体的用例？最佳的接口形式是什么？

动态大小类型（DST）如何与这个 API 交互？

### Fat pointers and DSTs

考虑跳表中的一个节点。它包含：key、value、tower。tower 是一个原子指针数组。为了节省一层间接引用，将整个 tower 布局在节点内部是明智的。由于 tower 由可变数量的指针组成，跳表节点是动态大小的。

另一个例子可能是支持哈希表或 Chase-Lev 双端队列的数组。它们也是动态大小的，因此将长度与数组元素一起布局可能是有意义的。B 树节点同样可能是动态大小的。

Rust 通常将 DST 类型的实例表示为胖指针（fat pointer）。这在无锁编程中是有问题的，因为胖指针由两个字组成。对这类类型进行原子操作在现代架构上是可能的（它们有 DWCAS 指令），但这仍然不够可移植，也不在稳定版 Rust 中可用。还有其他原因可能让人更倾向于使用瘦指针并将长度存储在对象内部，例如性能（缓存局部性）和内存消耗。

因此，`Atomic` 不会尝试支持胖指针。动态大小类型是一个有趣的话题，可能在未来的 RFC 中进行更详细的讨论。