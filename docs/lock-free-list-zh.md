# 无锁有序单向链表的设计与实现

> 对应代码：`cs431/src/lockfree/list.rs`
>
> 基于 Harris 原版论文，并提供了三种遍历策略。

## 1. 总览

本实现是一个**无锁有序单向链表**，支持并发查找、插入和删除。节点在链表中按 key 的 `Ord` 排序排列。无锁性通过 **CAS（compare-and-swap）** 和 **tag 标记位** 来实现，内存回收依赖 **crossbeam-epoch**。

## 2. 核心数据结构

### 2.1 `Node<K, V>` —— 链表节点

```rust
pub struct Node<K, V> {
    pub next: Atomic<Node<K, V>>,
    pub key: K,
    pub value: V,
}
```

- `next` 是一个 `Atomic` 指针（内部是 `AtomicUsize`），其 tag 位存储逻辑删除标记
- `key` 一旦创建不可变，用于排序和查找
- `value` 一旦插入不可变（更新需要删除再插入新节点）

> **设计注意**：字段本身是私有的，但 `Node` 类型本身是 `pub` 的，外部代码可以通过 `Cursor` 拿到 `Shared<Node>` 后在 unsafe 代码中操控指针。这是一个已知的脆弱点——正确的设计应该是把 `Node` 完全隐藏在模块内部。详见 TODO 注释。

### 2.2 `List<K, V>` —— 链表容器

```rust
pub struct List<K, V> {
    head: Atomic<Node<K, V>>,
}
```

只包含一个指向头节点的 `Atomic` 指针。空链表时 `head` 为 `null`。

**`Sync` 安全性**：需要 `K: Sync` 和 `V: Sync`。与栈和队列不同，这里 `K` 和 `V` 分别在 `find` 和 `delete` 中被并发读取。

### 2.3 `Cursor<'g, K, V>` —— 操作游标

```rust
pub struct Cursor<'g, K, V> {
    prev: &'g Atomic<Node<K, V>>,
    curr: Shared<'g, Node<K, V>>,
}
```

游标是操作链表的核心入口。它持有：
- `prev`：前一个节点的 `next` 指针的引用（用于 CAS unlink 和 insert）
- `curr`：当前节点的 `Shared` 指针

`curr` 的 tag 在 `Cursor::new` 中被强制清零（`curr.with_tag(0)`），防止带 tag 的指针通过 CAS 落入链表，导致后续遍历误判。

## 3. 所有权模型与类型体系

本节涉及的类型来自 `crossbeam_epoch`，理解它们之间的所有权关系是理解代码的关键。

### 3.1 `Owned<T>` —— 独占所有权

- 语义接近 `Box<T>`，拥有 `T` 的唯一所有权
- 不是 `Clone`，不可共享
- Drop 时释放 `T`
- 典型用途：插入前持有新节点

### 3.2 `Shared<'g, T>` —— 共享只读引用

- 可 `Copy`，多线程可同时持有同一节点的 `Shared`
- 借用了节点，不负责释放
- 通过 `unsafe { shared.deref() }` 获得 `&T`
- 典型用途：遍历链表时看到的节点指针

### 3.3 `Atomic<T>` —— 链表内存链的"指针"

- 内部是 `AtomicUsize`，存储了"tagged pointer"
- 提供 `load`、`store`、`compare_exchange`、`fetch_or` 等原子操作
- 是 `Owned` 和 `Shared` 之间的桥梁——只读时 load 出 `Shared`，写入时通过 CAS 消费 `Owned`

### 3.4 三种状态的转换

```
Owned<T> ──store/CAS──→ Atomic<T> ──load──→ Shared<'g, T>
   ↑                                                 │
   └──── try_into_owned() ←──────────────────────────┘
```

## 4. 逻辑删除与 Tag 位

### 4.1 为什么需要标记位

如果直接物理删除节点，其他线程可能还持有该节点的 `Shared` 引用，导致 use-after-free。epoch GC 能解决内存安全的问题，但仍然不能防止**插入丢失**。

插入丢失的场景：

1. 线程 1 准备删除 B，被抢占前读到了 `B.next = C`
2. 线程 2 在 B 和 C 之间插入 X，`CAS(B.next, C, X)` 成功
3. 线程 1 恢复，`CAS(A.next, B, C)` 也成功——跳过了 X

如果没有标记位，插入节点被静默丢弃。

### 4.2 标记位的实现

标记位利用**指针对齐**产生的空闲低位。在 64 位系统上，所有指针至少 4 字节对齐，最低 2 位始终为 0，可以用于存储额外信息。

**标记**：`fetch_or(1, AcqRel)` 将 `next` 指针的最低位置为 1

**检测**：`load(Acquire).tag() != 0`

**清零**：`with_tag(0)` 获得真实地址

### 4.3 为什么 `Cursor::new` 必须清零 tag

如果带 tag 的指针被存入 `prev.next`，后续遍历线程看到 tag=1 就会误以为该节点已被标记删除，错误地将其 unlink 并回收。`curr.with_tag(0)` 确保 tag 永远不持久化到链表中。

## 5. 三种遍历策略

`List` 实现了三种不同的遍历（find）策略，核心区别在于遇到标记节点时的**清理策略**。

### 5.1 `find_harris` —— 批量清理

```
算法：查找阶段只前进 curr，记录 prev_next（prev.next 的原值）
      查找完成后，如果 prev_next != curr，说明之间有被跳过的标记节点
      CAS 跳过整段，然后逐个 defer_destroy
```

特点：
- 一次性跳过整段已标记节点，CAS 次数少
- 如果 `Less` 子句让 prev 前移了，原 prev 之前的标记节点会被遗留
- 适合标记节点成段出现的场景

### 5.2 `find_harris_michael` —— 逐个清理

```
算法：每遇到一个标记节点，立即 CAS 跳过它并 defer_destroy
      然后继续检查下一个节点
```

特点：
- 不留尾巴，所有遍历过的标记节点都会被清理
- 每个标记节点都需要一次 CAS，高竞争下 CAS 反复失败的代价更大
- 清理更积极、更平滑

### 5.3 `find_harris_herlihy_shavit` —— 不清理

```
算法：遇到标记节点跳过，遇到 key < 搜索键前进
      永不触发 CAS unlink 或 defer_destroy
```

特点：
- 不会失败（总是返回 `Ok(boolean)`）
- 只读，仅用于 `lookup`
- 速度最快，但清理完全惰性

### 5.4 对比总结

| | Harris | Harris-Michael | Harris-Herlihy-Shavit |
|--|--------|---------------|----------------------|
| 清理粒度 | 整段 | 单个 | 不清理 |
| CAS 次数 | 每段 1 次 | 每个标记节点 1 次 | 0 |
| 能否失败 | 可能（CAS 竞争） | 可能（CAS 竞争） | 永不失败 |
| 适用操作 | insert, delete | insert, delete | lookup 专用 |

## 6. CRUD 操作实现

### 6.1 查找策略模式

`List` 中的 `find`、`lookup`、`insert`、`delete` 都接受一个查找函数作为参数：

```rust
fn find<'g, F>(&'g self, key: &K, find: &F, guard: &'g Guard) -> (bool, Cursor<'g, K, V>)
where
    F: Fn(&mut Cursor<'g, K, V>, &K, &'g Guard) -> Result<bool, ()>,
```

参数 `F` 是 `Cursor` 上三种 `find_*` 方法中的一个。通过函数指针传递策略，复用同一套 CRUD 逻辑。

`F` 的 trait 约束是 `Fn` 而不是 `FnOnce`，因为需要在循环中多次调用（CAS 失败时重试）。

### 6.2 查找 (find / lookup)

```rust
fn lookup() -> Option<&'g V> {
    loop {
        let mut cursor = self.head(guard);
        if let Ok(r) = find(&mut cursor, key, guard) {
            if r { return Some(cursor.lookup()) }
            else { return None }
        }
        // CAS 失败，从头重试
    }
}
```

`lookup` 返回的 `&'g V` 的生命周期和 guard 绑定——这就是为什么使用未受保护的 guard 会导致 use-after-free。

### 6.3 插入 (insert)

```rust
fn insert() -> bool {
    let mut node = Owned::new(Node::new(key, value));
    loop {
        let (found, mut cursor) = self.find(&node.key, &find, guard);
        if found { return false; }  // key 已存在

        match cursor.insert(node, guard) {  // CAS 插入
            Ok(()) => return true,
            Err(n) => node = n,  // CAS 失败，取回 Owned 重试
        }
    }
}
```

`Cursor::insert` 让新节点的 `next` 指向 `self.curr`，然后 `CAS(prev.next, curr, new_node)`。失败时 `e.new` 返回 `Owned` 的所有权，避免重新分配。

### 6.4 删除 (delete)

删除分三步：

1. **标记**：`fetch_or(1, AcqRel)` 标记 `curr.next`——逻辑删除
   - 返回旧值；如果 `tag() == 1`，说明已被其他线程标记，返回 `Err(())`
2. **unlink**（可选）：`CAS(prev.next, curr, next)` 尝试物理跳过
   - 成功：`defer_destroy(curr)`，释放权交给 epoch GC
   - 失败：下一个遍历线程会顺带清理
3. **返回**：`Ok(&curr_node.value)`

标记和 unlink 分离的好处：标记是删除的"原子提交点"，unlink 和回收可以延迟。

## 7. Drop 与内存回收

```rust
impl<K, V> Drop for List<K, V> {
    fn drop(&mut self) {
        let mut o_curr = mem::take(&mut self.head);
        while let Some(curr) = unsafe { o_curr.try_into_owned() }.map(Owned::into_box) {
            o_curr = curr.next;
        }
    }
}
```

`&mut self` 的独占借用保证了调用 drop 时没有其他线程持有该链表的引用。因此可以安全地 `try_into_owned()` 逐个回收节点，不需要走 epoch GC 的 `defer_destroy`。

## 8. memory ordering 讨论

代码中所有 `Acquire`/`Release` ordering 的主要用途：

| 操作 | 实际使用 | 必要性 |
|------|---------|--------|
| `next.load(Acquire)` | 遍历读 | 必要 |
| `compare_exchange(Release)` | unlink（find_harris/find_harris_michael) | 不必要（我自己的看法，不保证正确性） |
| `fetch_or(1, AcqRel)` | 标记 | 不必要（我自己的看法，不保证正确性） |
| `compare_exchange(Release)` | insert | 必要 |

其他几处的保守 ordering 在 x86（TSO）上没有性能代价，在 ARM 上有微小额外同步开销，但不影响正确性。

## 9. 安全保证总结

- **内存安全**：epoch GC 保证节点在无人引用后才释放
- **并发正确性**：标记位防止插入丢失
- **Drop 安全**：`&mut self` 保证没有并发残留引用
- **类型安全**：`Owned`/`Shared`/`Atomic` 的类型系统隔离确保正确使用
