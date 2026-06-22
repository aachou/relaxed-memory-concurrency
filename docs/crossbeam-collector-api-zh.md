# Summary

本 RFC 引入了 `Collector` API，以支持多样化的用例。

# Motivation

随着 Crossbeam 项目的发展，用户需要比现有 API 更多样化的接口。

首先，大多数用户会对当前与默认单例垃圾收集器交互的 API 感到满意。

其次，一些用户可能希望创建自己的垃圾收集器，并通过一个*句柄（handle）*来使用该收集器。由于当前 API 只提供了一个垃圾收集器，Crossbeam 目前不支持这种用例。

最后，一些用户希望将垃圾收集器嵌入到其他系统库中，例如内存分配器和线程管理环境。请注意，这通常是标准库的一部分，因此这种用例要求（至少部分）Crossbeam 在 `#[no_std]` 环境中实现。同样，Crossbeam 目前也不支持这种用例。

为了支持这些多样化的用例，本文作者提议引入 **`Collector` API**。

# Detailed design

供参考，本 RFC 已在此[分支](https://github.com/jeehoonkang/crossbeam-epoch/tree/handle)中完全实现。关于该分支的讨论可在此 [PR](https://github.com/crossbeam-rs/crossbeam-epoch/pull/21) 中找到。

## `Collector` API

在此 API 层中，公共结构体 `Collector` 是一个拥有自己全局 epoch 的通用垃圾收集器，公共结构体 `Handle` 是收集器中一个参与者的抽象。`Collector` 是对垃圾收集全局数据（`Global`）的计数引用，`Handle` 是全局数据计数引用和本地数据（`Local`）的元组：

```
pub struct Collector(Arc<Global>);

pub struct Handle {
    global: Arc<Global>,
    local: Local,
}
```

你可以通过创建新的 `Global` 数据来创建新的收集器，并通过共享其 `Global` 数据来向收集器添加句柄。使用 `Handle`，可以创建一个 `Scope` 并将其传递给给定的函数：

```
impl Collector {
    pub fn new() -> Self;
    pub fn handle(&self) -> Handle;
}

impl Handle {
    pub fn pin<F, R>(&self, f: F)
    where F: FnOnce(&Scope) -> R {
        self.local.pin(&self.global, f)
    }

    ... // 其他方法
}
```

注意，`Handle::pin()` 与 `Local::pin()` 不同，不需要引用 `Global`，因为 `Handle` 已经持有一个引用。

详细信息请参见[实现](https://github.com/jeehoonkang/crossbeam-epoch/blob/handle/src/collector.rs)。

## The default garbage collector API

将会有默认的收集器和每个线程的默认收集器句柄：

```
lazy_static! { pub COLLECTOR: Collector = Collector::new(); }
thread_local! { pub HANDLE: Handle = COLLECTOR.handle(); }

pub fn pin<F, R>(f: F)
where F: FnOnce(&Scope) -> R {
    HANDLE.with(|handle| { handle.pin(f) })
}
```

详细信息请参见[实现](https://github.com/jeehoonkang/crossbeam-epoch/blob/handle/src/default.rs)。

# Alternatives

`&'"'"'scope Scope` vs. `Scope<'"'"'scope>`：目前我们传递 `&'"'"'scope Scope` 作为当前参与者已固定的见证。引用类型可能会引入嵌套间接的运行时开销，但本文作者认为这种开销是可以承受的。可以通过使用 `Scope<'"'"'scope>` 作为见证来避免这种开销，但这是一个非正统的选择。

`Arc<Global>` vs. `&Global`：提议的 `Collector` 实现为 `Arc<Global>`，这引入了引用计数的运行时开销。本文作者认为这种开销可以忽略不计，因为句柄创建很可能在冷路径中。使用 `&Global` 代替 `Arc<Global>` 可能会消除运行时成本，但会显著复杂化 API。

# Unresolved questions

`Collector` API 和默认收集器都依赖于隐藏的 `Global` 和 `Local` 结构体。为了最后的优化空间，暴露这个内部 API 可能是有益的，尽管这个 API 比 `Collector` API 复杂得多。不幸的是，目前我们缺乏具体的用例来精确评估这种权衡。

对于 `#[no_std]` 环境，长期计划是将依赖于 `std` 的部分（即默认收集器 API）和不依赖的部分（即 `Collector` API）分离。提议的 `Collector` API 几乎不依赖于 `std`，除了 (1) `Collector` API 使用 `std::sync::Arc`，以及 (2) 它们依赖于标准库中的数据结构。本文作者认为，通过 (1) 在 `#[no_std]` 中重新实现 `Arc`，以及 (2) 在[分配器 trait 稳定后](https://github.com/rust-lang/rust/issues/32838)使用 `#[no_std]` 数据结构，我们可以轻松使其完全独立于 `std`。