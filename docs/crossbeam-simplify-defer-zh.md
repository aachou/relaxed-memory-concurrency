# Summary

简化推迟销毁操作，移除 `Scope::defer_free`。

# Motivation

在 [rsdb coco 之旅报告](https://github.com/stjepang/coco/issues/7) 中，**@spacejam** 报告了几个关于推迟销毁的痛点，这让我开始思考 Crossbeam 提供的接口：

```
impl Scope {
    // 堆分配对象 `ptr` 的推迟释放。
    unsafe fn defer_free<T>(&self, ptr: Ptr<T>);

    // 堆分配对象 `ptr` 的推迟销毁和释放。
    unsafe fn defer_drop<T: Send + 'static>(&self, ptr: Ptr<T>);

    // 任意函数 `f` 的推迟执行。
    unsafe fn defer<F: FnOnce() + Send + 'static>(&self, f: F);

    // ...
}
```

三个用于安全内存回收的函数！它们之间有何区别？

- `defer_free` 只释放内存。它假设目标内存包含一个 `T` 类型的值，调用其析构函数，然后释放内存。
- `defer_drop` 先销毁值，再释放内存。它同样假设目标内存包含一个 `T` 类型的值。
- `defer` 接受任意闭包，可以用于任何事情。

注意最后一个函数与 Loom（一个无随机测试工具）的"行为模型"有直接的相似之处，但这个讨论留到以后再说。

这确实有点多。我们真的需要全部三个吗？也许 `defer` 本身就已经足够了？

仔细审视 API，会发现 `defer_free` 和 `defer_drop` 是可以由 `defer` 来表达的。下面说明如何做到：

```
// 之前的 defer_free：
scope.defer_free::<T>(ptr);

// 之后的 defer：
unsafe { scope.defer(|| drop(ptr.into_owned())); }
```

等等——首先，`defer` 是 `unsafe` 的吗？是的，它是。但是在典型使用场景中，我们会在闭包内捕获悬空指针，这确实可能是 unsafe 的……

但严格来说，`defer` 本身并不需要是 unsafe 的。只要 `defer` 在同一个作用域内执行闭包，那它就是安全的。如果它在全局某处存储闭包并在之后调用，那才是 unsafe 的，因为闭包捕获的数据可能已被释放。

等等，但 `defer` 实际上只在其作用域内执行闭包。`Scope` 会在作用域结束前执行所有推迟的闭包。`defer` 是安全的吗？是的。

```
// 所以下面的代码实际上是可以的，因为 defer 会在作用域结束前执行 a：
let a = 5;
scope.defer(|| println!("{}", a));
```

而且，如果 `defer` 接受一个 `'static` 闭包，那它总是安全的：

```
// 这也行：
let a = 5;
scope.defer(|| println!("{}", a));
```

这是因为 `a` 实现了 `Send + 'static`——它是 `i32`，所以确实如此。但是，如果闭包不是 `'static` 呢？

```
// 这也可以吗？
let a = vec![1, 2, 3];
scope.defer(|| println!("{}", a.len()));
```

这里 `a` 的类型是 `Vec<i32>`，它确实是 `'static` 的，因为不包含借用。所以没问题。

关键是——你**不能**传递捕获了非 `'static` 引用的闭包给 `defer`：

```
// 这不行——至少目前如此：
let a = vec![1, 2, 3];
scope.defer(|| println!("{:?}", a));
```

`Vec<i32>` 不是 `Send`，所以这段代码甚至无法编译。实际上需要同时满足 `Send` 和 `'static`。

没关系。我们可以把 `a` 包装到 `Arc` 中，但用 `defer` 来推迟销毁 `Arc` 中的内容是很奇怪的。典型用法其实是推迟释放像 `[usize; 100]` 这样的东西，或者简单地在指定作用域内执行一个操作。

`defer` 没有理由被标记为 unsafe，这是当初设计 `Scope` API 时的一个疏忽。所以让我们让它变成安全的。

另一个问题是当前的实现简单地将闭包 `f` 装箱到堆上。但在大多数情况下，`defer` 只是用来释放一个数组或执行简单的销毁例程，因此传入的闭包通常很小，比如可以放进 3 个字长。

为了提升性能，我们会尽可能避免分配，使用类似 `SmallVec` 的方式来存放闭包。

# Detailed design

相比原来的三个 unsafe 函数 `defer_free`/`defer_drop`/`defer`，我们只保留 unsafe 的 `defer_drop` 和安全的 `defer`：

```
impl Scope {
    // 堆分配对象 `ptr` 的推迟销毁和释放。
    unsafe fn defer_drop<T: Send + 'static>(&self, ptr: Ptr<T>);

    // 任意函数 `f` 的推迟执行。
    fn defer<F: FnOnce() + Send + 'static>(&self, f: F);

    // ...
}
```

关于 `defer` 的优化，我已经准备好了一个 `Deferred` 的实现，它能够将小闭包存储在自身内部，对于大闭包则回退到堆分配。

换句话说，`Deferred` 之于 `FnOnce() + Send + 'static`，就像 `SmallVec` 之于 `Vec`。

`Deferred` 纯粹是实现细节——不会出现在公开 API 中。

```rust
use std::mem;
use std::ptr;

/// Provides methods to dispatch a call to a `FnOnce()` from a trait object.
pub trait Callback {
    /// Calls the function from a trait object on the stack.
    ///
    /// This will copy `self`, call the function, and finally drop the copy.
    /// This method may be called only once, and `self` must not be dropped after that (tip: pass
    /// it to `std::mem::forget`).
    unsafe fn copy_and_call(&self);

    /// Calls the function from a trait object on the heap.
    fn call_box(self: Box<Self>);
}

impl<F: FnOnce() + Send + 'static> Callback for F {
    #[inline]
    unsafe fn copy_and_call(&self) {
        let f: Self = ptr::read(self);
        f();
    }

    #[inline]
    fn call_box(self: Box<Self>) {
        let f: Self = *self;
        f();
    }
}

/// The representation of a trait object like `&SomeTrait`.
///
/// This struct has the same layout as types like `&SomeTrait` and `Box<AnotherTrait>`.
///
/// It is actually already provided as `std::raw::TraitObject` gated under the nightly `raw`
/// feature. But we don't use nightly Rust, so the struct was simply copied over into Crossbeam.
///
/// If the layout of this struct changes in the future, Crossbeam will break, but that is a fairly
/// unlikely scenario.
// FIXME(stjepang): When feature `raw` gets stabilized, use `std::raw::TraitObject` instead.
#[repr(C)]
#[derive(Copy, Clone)]
struct TraitObject {
    data: *mut (),
    vtable: *mut (),
}

/// Some space to keep a `FnOnce()` object on the stack.
type Data = [usize; 3];

/// A `FnOnce()` that is stored inline if small, or otherwise boxed on the heap.
///
/// This is a handy way of keeping an unsized `FnOnce()` within a sized structure.
pub struct Deferred {
    vtable: *mut (),
    data: Data,
}

impl Deferred {
    /// Constructs a new `Deferred` from a `FnOnce()`.
    pub fn new<F: FnOnce() + Send + 'static>(f: F) -> Self {
        let size = mem::size_of::<F>();
        let align = mem::align_of::<F>();

        unsafe {
            if size <= mem::size_of::<Data>() && align <= mem::align_of::<Data>() {
                let vtable = {
                    let callback: &Callback = &f;
                    let obj: TraitObject = mem::transmute(callback);
                    obj.vtable
                };

                let mut data = Data::default();
                ptr::write(&mut data as *mut Data as *mut F, f);

                Deferred { vtable, data }
            } else {
                let mut data = Data::default();
                let b: Box<Callback> = Box::new(f);
                ptr::write(&mut data as *mut Data as *mut Box<Callback>, b);

                Deferred {
                    vtable: 0x1 as *mut (),
                    data,
                }
            }
        }
    }

    /// Calls the function or panics if it was already called.
    #[inline]
    pub fn call(&mut self) {
        let vtable = mem::replace(&mut self.vtable, ptr::null_mut());
        assert!(!vtable.is_null(), "cannot call `FnOnce` more than once");

        unsafe {
            if vtable as usize != 0x1 {
                let data = &mut self.data as *mut _ as *mut ();
                let obj = TraitObject { data, vtable };
                let callback: &Callback = mem::transmute(obj);
                callback.copy_and_call();
            } else {
                let b: Box<Callback> = ptr::read(&self.data as *const Data as *const Box<Callback>);
                b.call_box();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Deferred;

    #[test]
    fn smoke_on_stack() {
        let a = [0usize; 1];
        let mut d = Deferred::new(move || drop(a));
        d.call();
    }

    #[test]
    fn smoke_on_heap() {
        let a = [0usize; 10];
        let mut d = Deferred::new(move || drop(a));
        d.call();
    }

    #[test]
    #[should_panic(expected = "cannot call `FnOnce` more than once")]
    fn twice_on_stack() {
        let a = [0usize; 1];
        let mut d = Deferred::new(move || drop(a));
        d.call();
        d.call();
    }

    #[test]
    #[should_panic(expected = "cannot call `FnOnce` more than once")]
    fn twice_on_heap() {
        let a = [0usize; 10];
        let mut d = Deferred::new(move || drop(a));
        d.call();
        d.call();
    }

    #[test]
    fn string() {
        let a = "hello".to_string();
        let mut d = Deferred::new(move || assert_eq!(a, "hello"));
        d.call();
    }

    #[test]
    fn boxed_slice_i32() {
        let a: Box<[i32]> = vec![2, 3, 5, 7].into_boxed_slice();
        let mut d = Deferred::new(move || assert_eq!(*a, [2, 3, 5, 7]));
        d.call();
    }

    #[test]
    fn long_slice_usize() {
        let a: [usize; 5] = [2, 3, 5, 7, 11];
        let mut d = Deferred::new(move || assert_eq!(a, [2, 3, 5, 7, 11]));
        d.call();
    }
}
```

# Drawbacks

无。

# Alternatives

保留 `defer_free`。

# Unresolved questions

### 我们需要 `defer_drop` 吗？

事实证明，我们也可以用 `defer` 来替代 `defer_drop`：

```
match self.head.compare_and_set_weak(head, next, AcqRel, scope) {
    Ok(()) => unsafe {
        let raw = head.as_raw() as usize;
        scope.defer(|| drop(Box::from_raw(raw as *const Node<T>)));
        return Some(ptr::read(&h.value));
    },
    Err(h) => head = h,
}
```

但这现在有点笨重……

销毁 `head` 存在几个不太符合人体工学的障碍：

1. `Ptr<'scope>` 不能传递给 `'static` 闭包。
2. 裸指针不能传递给 `Send` 闭包（一个变通方案是将指针转换为 `usize`）。
3. 最后，裸指针必须传递给 `Box::from_raw`。

为了简化问题，让我们引入一个新的 unsafe 构造函数 `Owned::from_ptr`：

```
match self.head.compare_and_set_weak(head, next, AcqRel, scope) {
    Ok(()) => unsafe {
        let head = Owned::from_ptr(head);
        scope.defer(move || drop(head));
        return Some(ptr::read(&h.value));
    },
    Err(h) => head = h,
}
```

严格来说，这段代码可能不正确，因为同一个对象同时存在 `Owned` 和 `Ptr`。但由于在闭包执行之前 `Owned` 不会被使用，所以这没问题。不过 **@RalfJung** 的新 unsafe 代码检查器可能会拒绝这段代码，认为它是不安全的。

我们可以通过引入另一个 unsafe 辅助方法 `Ptr::to_static`（将 `Ptr<'scope>` 转换为 `Ptr<'static>`）来解决这个问题：

```
match self.head.compare_and_set_weak(head, next, AcqRel, scope) {
    Ok(()) => unsafe {
        let head = head.to_static();
        scope.defer(|| drop(Owned::from_ptr(head)));
        return Some(ptr::read(&h.value));
    },
    Err(h) => head = h,
}
```

但即使这样也还是有点粗略，我们可能更倾向于转换为 `Ptr<'unsafe>`。关于 `unsafe` 生命周期的 RFC 曾经被[提出](https://github.com/rust-lang/rfcs/pull/1918)但被推迟了。

除非我们能找到一种方法，让使用 `defer` 销毁 `Ptr` 既更符合人体工学又更安全，否则这仍然是一个未解决的问题。
