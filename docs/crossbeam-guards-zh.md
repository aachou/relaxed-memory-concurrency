# Summary

用 `Guard` 替换 `Scope`。

# Motivation

在 [Atomic API RFC](https://github.com/crossbeam-rs/rfcs/blob/master/text/2017-05-02-atomic-api.md#scopes-or-guards) 中，我们决定使用作用域（scope）而不是 guard 来建模固定（pinning）。理由是之前使用 guard 的固定实现是不安全的，并且修复问题的方式并不完全明确。

Guard 的问题在于，它们持有指向线程本地数据的指针/引用，但不受生命周期约束（如作用域那样），因此它们可能比引用的线程本地数据存活得更久。上述 RFC 中给出了一个演示此问题的例子。

解决问题的关键如下：不是*限制*作用域/guard 的生命周期使其不超过线程本地数据，而是通过引用计数来*延长*线程本地数据的生命周期。如果线程本地数据是引用计数的，guard 可以安全地长时间持有对该数据的引用。

# Detailed design

已经有一个[可供审查的 PR](https://github.com/crossbeam-rs/crossbeam-epoch/pull/31) 将作用域替换为 guard。

## The interface

```
// guard 类型。
pub struct Guard { ... }

// 此接口与 `Scope` 的接口没有区别。
impl Guard {
    pub unsafe fn defer<F, R>(&self, f: F)
    where
        F: FnOnce() -> R + Send;

    pub fn flush(&self);
}

// Guard 是可克隆的。
impl Clone for Guard { ... }

// 此函数不再接受闭包——它返回一个 guard。
pub fn pin() -> Guard;

// 返回一个特殊虚拟 guard 的引用，该 guard 不固定任何线程。
pub unsafe fn unprotected() -> &'"'"'static Guard;

// 方法现在接受 `&Guard` 而不是 `&Scope`。
impl<T> Atomic<T> {
    pub fn compare_and_set<'"'"'g, O>(
        &self,
        current: Ptr<T>,
        new: Ptr<T>,
        ord: O,
        _: &'"'"'g Guard,
    ) -> Result<(), Ptr<'"'"'g, T>>
    where
        O: CompareAndSetOrdering;

    // 其他方法同理...
}
```

## How reference counting works

每个参与基于 epoch 的垃圾回收的线程都有关联的堆分配数据（在 `Local` 结构体中）。所有这些 `Local` 被连接成一个链表，该链表的头指针保存在全局数据中（在 `Global` 结构体中）。`Global` 还持有全局 epoch 和全局垃圾队列。

每个 `Local` 持有一个 `ManuallyDrop<Arc<Global>>`，从而保持其所在的垃圾收集器存活。类似地，每个 `Handle` 和每个 `Guard` 都持有一个指向关联 `Local` 的指针。Handle 和 guard 使用 `Local` 内部的 `handle_count` 和 `guard_count` 字段计数。当两个计数都达到零时，堆分配的 `Local` 被标记为已删除，然后其 `Arc<Global>` 被立即释放（使用 `ManuallyDrop::drop`）。如果该 `Local` 持有对 `Global` 的最后一个引用，则 `Global` 也会被销毁。

## Unprotected access to `Atomic`s

有趣的是，`epoch::unprotected()` 返回 `&'"'"'static Guard` 而不是 `Guard`。这个决定旨在鼓励直接使用 `unprotected()` 而不是创建独立的非受保护 guard。

考虑以下示例：

```
// 创建一个独立的虚拟 guard。
let guard = &epoch::unprotected().clone();
let buffer = self.buffer.load(Relaxed, guard);

// 直接将 `&'"'"'static Guard` 引用传递给 `load`。
let buffer = self.buffer.load(Relaxed, epoch::unprotected());
```

我的个人观点：

- 第一个版本虽然在语法上完全合法，但通过在栈上创建一个 guard 给人一种虚假的安全感。它暗示"这里我有一个 guard（但实际上不是——它是假的！），然后使用这个 guard 进行了一次 load"。
- 第二个版本在意图上更明确。它表明"这是一个不受保护的 load"。在大多数情况下应该使用这个版本。

推动这个决策的示例（调整 Chase-Lev 双端队列大小）：

```
#[cold]
unsafe fn resize(&self, new_cap: usize) {
    // 加载 bottom、top 和 buffer。
    let b = self.bottom.load(Relaxed);
    let t = self.top.load(Relaxed);

    let buffer = self.buffer.load(Relaxed, epoch::unprotected());

    // 分配一个新的 buffer。
    let new = Buffer::new(new_cap);

    // 将数据从旧 buffer 复制到新 buffer。
    let mut i = t;
    while i != b {
        ptr::copy_nonoverlapping(buffer.deref().at(i), new.at(i), 1);
        i = i.wrapping_add(1);
    }

    let guard = &epoch::pin();

    // 用新 buffer 替换旧 buffer。
    let old = self.buffer
        .swap(Owned::new(new).into_ptr(guard), Release, guard);

    // 稍后销毁旧 buffer。
    guard.defer(move || old.into_owned());

    // 如果 buffer 非常大，则刷新线程本地垃圾以便尽快释放。
    if mem::size_of::<T>() * new_cap >= FLUSH_THRESHOLD_BYTES {
        guard.flush();
    }
}
```

首先，`self.buffer` 使用不受保护的 guard 加载，然后（使用 `swap`）使用真实的 guard 再次加载。如果我们在栈上创建两个 guard，可能后来会容易混淆它们。

## Changes to the `Collector` interface

### Should `Handle` be `Send`?

如果允许 `Handle` 为 `Send`，以下代码将编译：

```
let c = Collector::new();
let h = c.handle();
let guard = h.pin();

thread::spawn(move || {
    let guard = h.pin();
});
```

这是一段可疑的代码，尽管不一定*错误*。然而，如果我们允许这样做，`guard_count` 必须是一个原子整数，对计数器的原子操作会不必要地拖慢固定操作。

因此，`Handle` 不会是 `Send`。

注意，目前 `Handle::clone` 会创建一个全新的句柄，可以传递给另一个线程（参见 [PR #26](https://github.com/crossbeam-rs/crossbeam-epoch/pull/26)），但对于非 `Send` 的句柄来说，这种克隆行为不再非常有用了。

### Accessing the default handle

此外，我们在为 `default_handle` 方法找到令人满意的签名方面遇到了困难（参见 [PR #28](https://github.com/crossbeam-rs/crossbeam-epoch/pull/28)）。

以下是我们可以选择的选项列表：

#### 1st option

如 [PR #28](https://github.com/crossbeam-rs/crossbeam-epoch/pull/28) 中所提议的，并增加一个 `try_default_handle` 函数，该函数放在 `nightly` 特性门后（因为 `LocalKey::try_with` 仍不稳定）：

```
pub unsafe fn default_handle() -> &'"'"'static Handle {
    &*HANDLE.with(|handle| handle as *const _)
}

#[cfg(feature = "nightly")]
pub unsafe fn try_default_handle() -> Option<&'"'"'static Handle> {
    HANDLE.try_with(|handle| &*(handle as *const _)).ok()
}
```

优点：零成本。

缺点：函数是 `unsafe` 的（因为 `'"'"'static` 生命周期是假的）。

#### 2nd option

这些函数类似于 `LocalKey::with` 和 `LocalKey::try_with`，它们接受一个对 `&Handle` 进行操作的闭包：

```
pub fn with_default_handle<R, F: FnOnce(&Handle) -> R>(f: F) -> R {
    HANDLE.with(|handle| f(handle))
}

#[cfg(feature = "nightly")]
pub fn try_with_default_handle<R, F: FnOnce(&Handle) -> R>(f: F) -> Option<R> {
    HANDLE.try_with(|handle| f(handle)).ok()
}
```

优点：零成本。

缺点：接口稍显不便。

#### 3rd option

如果我们让 handle 非 `Send`，现在我们可以计算 `Local` 内部的 handle 数量，并提供与 `std::thread::current()` 非常相似的接口：

```
pub fn default_handle() -> Handle {
    HANDLE.with(|handle| handle.clone())
}

#[cfg(feature = "nightly")]
pub fn try_default_handle() -> Option<Handle> {
    HANDLE.try_with(|handle| handle.clone()).ok()
}
```

优点：最方便的接口。

缺点：增加/减少内部 handle 计数有少量成本。

### The new interface

首先，`Handle::clone` 将被更改，使其递增内部引用计数（`handle_count`）并返回同一个 handle 的新引用（就像 `Thread::clone` 返回同一线程的新引用一样）。

其次，`default_handle` 将这样实现（第三个选项）：

```
lazy_static! {
    static ref COLLECTOR: Collector = Collector::new();
}

thread_local! {
    static HANDLE: Handle = COLLECTOR.handle();
}

// 如果 `HANDLE` 已被销毁，则 panic。
pub fn default_handle() -> Handle {
    HANDLE.with(|handle| handle.clone())
}

// 如果 `HANDLE` 已被销毁，返回 `None`。
#[cfg(feature = "nightly")]
pub fn try_default_handle() -> Option<Handle> {
    HANDLE.try_with(|handle| handle.clone()).ok()
}
```

通过这个接口，访问默认 handle 是安全且符合人机工程学的，但有一些引用计数的相关成本。幸运的是，这种成本小到可以原谅，基准测试将证明这一点……

## Benchmarks

以下是一个简单的基准测试，确保 guard 不会带来性能回归（至少不会显著）：

```
// 之前：使用作用域进行固定。
#[bench]
fn pin_empty(b: &mut Bencher) {
    b.iter(|| epoch::pin(|_| ()));
}

// 之后：使用 guard 进行固定。
#[bench]
fn pin_empty(b: &mut Bencher) {
    b.iter(|| epoch::pin());
}
```

前后的基准测试显示相同的数据：

```
test pin_empty ... bench:          12 ns/iter (+/- 0)
```

现在来看看使用 `default_handle` 进行固定的表现：

```
#[bench]
fn default_handle_pin(b: &mut Bencher) {
    b.iter(|| epoch::default_handle().pin());
}
```

结果：

```
test default_handle_pin ... bench:          13 ns/iter (+/- 0)
```

从结果可以看出，使用 `epoch::pin()` 固定比使用 `epoch::default_handle().pin()` 略快（12 ns 和 13 ns）。这是由于引用计数的开销，但差异足够小，在实际情况下不会产生影响。

# Drawbacks

理论上，使用作用域进行固定有可能稍微快一些。

作用域具有更严格的结构——它们是完美嵌套的，如下例所示：

```
epoch::pin(|first| {
    epoch::pin(|second| {
        // `second` 在 `first` 之后创建。
        // ..
        // `second` 在 `first` 之前销毁。
    })
})
```

另一方面，guard 可以在任意时间点创建、移动和释放：

```
let a = epoch::pin(); // 创建第一个 guard。
let b = epoch::pin(); // 创建第二个 guard。

let c = a; // 移动第一个 guard。

drop(c); // 释放第一个 guard。
drop(b); // 释放第二个 guard。
```

当取消固定线程时，作用域和 guard 都会递减一个计数器，该计数器跟踪有多少层嵌套。然而，在取消固定时，作用域已经知道计数器是否会变为零，而 guard 不知道。对于作用域，计数器的值不可能与创建作用域时遇到的值不同。

长话短说：guard 在取消固定时多了一个分支。但这个分支对性能的影响似乎微乎其微。

# Alternatives

1. 保持 `Scope`，不引入 `Guard`。
2. 同时提供 `Scope` 和 `Guard`。

# Unresolved questions

1. 我们是否也需要一个公共的 `default_collector()` 函数？
2. 是否需要 `Handle::collector()` 访问器？