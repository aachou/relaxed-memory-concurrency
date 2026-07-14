# MCS 锁（Mellor-Crummey and Scott 队列锁）

## 为什么叫这个名字

1991 年由 John M. Mellor-Crummey 和 Michael L. Scott 提出，论文题为 *"Algorithms for Scalable Synchronization on Shared-Memory Multiprocessors"*。目的是解决传统自旋锁在多核 CPU 上的可扩展性问题。

## 传统自旋锁的问题

```rust
use std::sync::atomic::{AtomicBool, Ordering};

struct SpinLock(AtomicBool);

impl SpinLock {
    fn lock(&self) {
        while self.0.swap(true, Ordering::Acquire) { /* spin */ }
    }
    fn unlock(&self) {
        self.0.store(false, Ordering::Release);
    }
}
```

**问题：**
1. **缓存一致性风暴。** 每个等待的 CPU 都在同一个 `flag` 变量上自旋。释放锁时设置 `flag = 0`，所有等待 CPU 的缓存行失效，一起去抢——但只有一个能成功，其余 99 个再次失效、再次自旋。这称为 **thundering herd**（惊群）问题。
2. **不公平。** 可能有 CPU 饿死——一直抢不到锁的线程永远被后来者抢占。
3. **CAS 在远端。** 每次 CAS 都要跨 NUMA 节点通信，开销大。

## MCS 锁的核心思想

**每个等待线程自旋在自己的局部变量上，而不是共享变量上。** 等待线程组织成队列，每个线程只需观察自己前驱的 `next` 指针是否完成。

数据结构：

```rust
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::ptr;

struct MCSNode {
    next: AtomicPtr<MCSNode>,
    locked: AtomicBool,
}

impl MCSNode {
    fn new() -> Self {
        MCSNode {
            next: AtomicPtr::new(ptr::null_mut()),
            locked: AtomicBool::new(true),
        }
    }
}

struct MCSLock {
    tail: AtomicPtr<MCSNode>,
}

impl MCSLock {
    fn new() -> Self {
        MCSLock {
            tail: AtomicPtr::new(ptr::null_mut()),
        }
    }
}
```

### 加锁

```rust
fn lock(&self, node: &MCSNode) {
    let node_ptr = node as *const MCSNode as *mut MCSNode;
    let prev = self.tail.swap(node_ptr, Ordering::AcqRel);
    if prev.is_null() {
        return;  // 没人排队，直接拿到锁
    }
    // SAFETY: prev 是之前入队的节点，必然有效
    unsafe { (*prev).next.store(node_ptr, Ordering::Release); }
    while node.locked.load(Ordering::Acquire) { /* spin */ }
}
```

### 解锁

```rust
fn unlock(&self, node: &MCSNode) {
    let node_ptr = node as *const MCSNode as *mut MCSNode;
    if node.next.load(Ordering::Acquire).is_null() {
        if self.tail.compare_exchange(
            node_ptr,
            ptr::null_mut(),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ).is_ok() {
            return;  // 我是最后一个，直接清空 tail 走人
        }
        // 新线程正在入队，但还没设 my.next
        while node.next.load(Ordering::Acuire).is_null() { /* spin */ }
    }
    // SAFETY: next 此时必然已设置
    unsafe {
        (*node.next.load(Ordering::Acquire)).locked.store(false, Ordering::Release);
    }
}
```

**关键优势：**

1. **没有惊群。** 释放锁时只通知一个后继，后继在自己的 `locked` 上自旋，其它等待者不受影响。
2. **NUMA 友好。** 每个线程自旋在自己的 `locked` 字段上，该字段在本地内存，不需要跨节点访问共享变量。
3. **公平。** 先入先出，不会饿死。
4. **O(1) 解锁。** 解锁只需操作一个后继节点，不依赖等待队列长度。

## MCS 锁与 BoC 的关联

论文说 BoC 的依赖图 **"adapts the MCS-queue lock data structure"**，做了两个关键改造：

| MCS 锁 | BoC | 变化 |
|--------|-----|------|
| `tail` 指向队尾 | `CownBase.last` 指向链尾 Request | 名称变了，功能一样 |
| `node.locked` 自旋等待 | `Behaviour.count` 异步通知 | **阻塞 → 异步** |
| `prev.next = node` 单 cown 入队 | `prev.next = behaviour` 多 cown 两阶段入队 | **扩展为原子地在多个链上同时入队** |
| 解锁设 `node.next.locked = false` | 完成时调 `next.ResolveOne()` 减 count | 同样是从前驱通知后继 |

核心区别一句话：**MCS 锁是阻塞的——自旋等锁释放；BoC 是非阻塞的——behaviour 注册到图中就返回，前驱完成后通过 count 异步调度后继执行。** 但队列化的等待结构和"前驱通知后继"的模式完全一致。
