# Summary

引入 `crossbeam-skiplist` crate，包含一个无锁跳表（skip list）。

该 crate 提供类似于 `BTreeMap` 和 `BTreeSet` 的有序映射和有序集合。将来，我们还可能基于跳表构建其他数据结构，例如优先队列。

# Motivation

这是 Crossbeam 中首批并发映射和集合数据结构。

跳表常被认为是一种较容易实现并发的数据结构——至少相对于其他映射/集合来说是如此。然而，在没有 GC 的语言中支持删除操作，并提供一个不过于受限的 API，是极其困难的。

本 crate 旨在提供一个并发的映射/集合，目标是在功能和易用性上媲美 `BTreeMap`/`BTreeSet`。API 必须相当容易使用，并且无需绕弯路就能提供用户对任何映射/集合所期望的所有操作。

一个很好的例子是 Java——你可以简单地将任何使用 [`TreeMap`](https://docs.oracle.com/javase/8/docs/api/java/util/TreeMap.html) 的地方替换为 [`ConcurrentSkipListMap`](https://docs.oracle.com/javase/8/docs/api/java/util/concurrent/ConcurrentSkipListMap.html)，代码就能*直接运行*。Java 中的并发映射几乎是非并发映射的完美替代品。遗憾的是，我认为在 Rust 中无法完全做到同样的事情——原因在于 Java 和 Rust 的内存模型有很大不同（例如，Java 没有移动语义，并且普遍在堆上分配对象）。话虽如此，我相信我们仍然可以设计出一个与 `BTreeMap` 相当接近的无锁映射。

关于性能，跳表相比于 B 树天生处于劣势。跳表中的每个节点都在堆上独立分配，而 B 树则在大块中分配节点，从而大幅提高缓存利用率。跳表节点在内存中分散的问题可以通过自定义分配器来部分缓解（尽量将跳表中相邻的节点分配在内存中靠近的位置），但这通常难度很大且效果不佳。

可以将 B 树看作一种压缩式垃圾回收器。考虑当 B 树块变满时会发生什么：可能会分配新块，并根据需要重新分布元素。这类似于压缩式垃圾回收器。注意，在 Rust 中，在内存中移动元素会使并发更加困难：如果一个线程可能同时将元素移动到内存中的不同位置，那么另一个线程就不能持有对该元素的引用。

然而，跳表将每个节点独立分配在堆上。一个节点包含键、值和一组 next 指针构成的塔。节点永远不会被移动到内存中的不同位置——一旦分配，它就会一直留在那里直到被销毁。这虽然使缓存利用率变差，但也使得在存在并行修改操作的情况下借用元素变得更加容易。

简而言之，无锁跳表在扩展性上将远优于互斥锁保护的 `BTreeMap`，但在单线程场景中，由于缓存利用率较差，它无法与 `BTreeMap` 竞争。

### 先前的工作

其他语言中值得注意的并发跳表实现：

1. [java.util.concurrent](http://grepcode.com/file/repository.grepcode.com/java/root/jdk/openjdk/8u40-b25/java/util/concurrent/ConcurrentSkipListMap.java#ConcurrentSkipListMap) (Java)：API 最为丰富——感觉像是任何其他映射的完美替代品。但是，实现效率并不是特别高，并且有一些有趣的怪癖。例如，塔中的每个指针都是独立分配的。再比如——它不通过标记指针来表示节点已删除，而是分配一个虚拟后继节点。
2. [libcds](https://github.com/khizmax/libcds/blob/19af81b7c61480ed705b91b4d01ee5d717a97cd2/cds/intrusive/skip_list_rcu.h) (C++)：相当完整和通用的 API（甚至可以在无 GC、EBR 和 HP 之间选择）。
3. [RocksDB](https://github.com/facebook/rocksdb/blob/68829ed89cec64186557dc0860fc693c118ff1c6/memtable/skiplist.h) (C++)：该跳表不支持删除或并发插入。然而，正在进行的插入操作不会阻塞其他线程读取跳表。一旦跳表变满，它会被刷新到磁盘存储中，并构建一个新的跳表来替换旧的。
4. [Folly](https://github.com/facebook/folly/blob/98d1077ce0603b0713353d638cb1436a28827af6/folly/ConcurrentSkipList.h) (C++)：这是一个并发跳表，但不是无锁的：它使用每节点锁。此外，删除的节点在跳表被销毁之前不会释放。
5. [libgee](https://github.com/GNOME/libgee/blob/da95e830524ffa309eb57925320666e5085b9d66/gee/concurrentset.vala) (Vala)：一个基于 hazard pointer 的跳表。看起来非常有趣。

还有几个用 Rust 实现的并发跳表，但到目前为止都没有发布到 crates.io 上，看起来都还在开发中：

1. [danburkert/pawn](https://github.com/danburkert/pawn/blob/8b6806d944d830f552d496cd3ee605d1707fdc51/src/util/skip_list.rs) (Rust)：一个相当古老的仅插入无锁跳表。看起来是一个已废弃的项目。
2. [Vtec234/lists-rs](https://github.com/Vtec234/lists-rs/blob/f83e516039dc4a421172af1cdbdcec85b0e73d74/src/epoch_skiplist.rs) (Rust)：一个支持删除并使用 Crossbeam 进行内存回收的无锁跳表。有趣的是，键始终是哈希后的，因此它实际上是一个哈希映射。
3. [boats/skiplist](https://gitlab.com/boats/skiplist/tree/master/src/skiplist) (Rust)：由 withoutboats 编写的仅插入无锁跳表。最近刚发布。

# Detailed design

提议的实现目前存放在 [stjepang/skiplist](https://github.com/stjepang/skiplist) 中，但之后会迁移到新仓库 `crossbeam-rs/crossbeam-skiplist`。这是一个使用 `crossbeam-epoch` 的基于 epoch 的内存回收机制的无锁跳表。

该实现基于以下工作：

1. [Practical lock-freedom](https://www.cl.cam.ac.uk/techreports/UCAM-CL-TR-579.pdf)（参见 *4.3.3 CAS-based design*）
2. [Linked Lists: Locking, Lock-Free and Beyond...](http://janvitek.org/events/TiC06/B-SLIDES/mh2.pdf)

代码库由三个主要源文件组成：

- [`base.rs`](https://github.com/stjepang/skiplist/blob/master/src/base.rs) — 包含跳表的基础实现细节。该文件并未试图暴露易用的接口，而是旨在提供一个跳表"引擎"，供上层包装为更友好的接口。
- [`map.rs`](https://github.com/stjepang/skiplist/blob/master/src/map.rs) — 将基础实现包装为类似于 `BTreeMap` 的映射接口。
- [`set.rs`](https://github.com/stjepang/skiplist/blob/master/src/set.rs) — 将基础实现包装为类似于 `BTreeSet` 的集合接口。

**注：** 这些映射和集合包装器只是暂定接口——它们已经完成，但我们可能会彻底修改它们。目前，请将它们视为概念验证。

## 暂定的映射 API

以下是一个快速演示。下面的代码取自 `BTreeMap` 文档的[第一个示例](https://doc.rust-lang.org/std/collections/struct.BTreeMap.html#examples)，只是将 `BTreeMap` 替换为了 `SkipMap`。为使其编译通过，还做了其他一些小的修改，但总体而言与原始代码差别不大：

```
// 类型推断使我们不必显式写出类型签名（在此示例中会是 `SkipMap<&str, &str>`）。
let movie_reviews = SkipMap::new();

// 评论一些电影。
movie_reviews.insert("Office Space",       "Deals with real issues in the workplace.");
movie_reviews.insert("Pulp Fiction",       "Masterpiece.");
movie_reviews.insert("The Godfather",      "Very enjoyable.");
movie_reviews.insert("The Blues Brothers", "Eye lyked it alot.");

// 检查某一部。
if !movie_reviews.contains_key("Les Misérables") {
    println!("We've got {} reviews, but Les Misérables ain't one.",
             movie_reviews.len());
}

// 哦，这篇评论有很多拼写错误，我们删掉它。
movie_reviews.remove("The Blues Brothers");

// 查找一些键对应的值。
let to_find = ["Up!", "Office Space"];
for book in &to_find {
    match movie_reviews.get(book) {
       Some(entry) => println!("{}: {}", book, entry.value()),
       None => println!("{} is unreviewed.", book)
    }
}

// 遍历所有内容。
for entry in &movie_reviews {
    let movie = entry.key();
    let review = entry.value();
    println!("{}: \"{}\"", movie, review);
}
```

请查看 [map.rs](https://github.com/stjepang/skiplist/blob/master/src/map.rs) 以了解 `SkipMap` 的完整接口。

与 `BTreeMap` 的一个有趣区别是，`insert` 和 `get` 等方法返回一个 `Entry<'a, K, V>`，它本质上只是一个指向跳表中某个条目的引用计数指针。注意，可以同时持有一个条目并删除它（你甚至可以调用 `entry.remove()`），但在最后一个引用被释放之前，条目的实际内容不会被销毁。

### 性能

前面已经提到，在单线程场景下 `SkipMap` 很难与 `BTreeMap` 竞争。让我们通过一个简单的基准测试来看看——该测试向映射中插入一百万个伪随机数。这不是一个科学严谨的基准测试，但至少能让我们对不同映射实现的表现有所了解。

机器：Intel Core i7-5600U（2 个物理核心，4 个逻辑核心）

首先是 `BTreeMap` 在三种不同场景下的表现：

- [`BTreeMap` (1 线程)](https://gist.github.com/stjepang/9b1bf73c2fdb0309cefda66b91f633dd)：315 ms
- [`Mutex<BTreeMap>` (1 线程)](https://gist.github.com/stjepang/437b82134b401d3fa2c9c439a003c1ea)：321 ms
- [`Mutex<BTreeMap>` (2 线程)](https://gist.github.com/stjepang/66000dfae15c8046b91ff3612c7d881f)：752 ms

注意，如果只使用一个线程，加锁的开销非常小。然而，一旦增加线程数，锁竞争就会带来巨大的性能惩罚。

但 `SkipMap` 不存在同样的问题。事实上，增加线程数反而提高了性能：

- [`SkipMap` (1 线程)](https://gist.github.com/stjepang/1980ab811009e94f2adfe8b230c20047)：1028 ms
- [`SkipMap` (2 线程)](https://gist.github.com/stjepang/a3f8f6dddac56d43e7dbfb2928cd3bfe)：561 ms

让我们再看看 C++ 中互斥锁保护的 `std::map`：

- [`std::map` (1 线程)](https://gist.github.com/stjepang/6aa80020b6edac1f6ea9af518e4ad989)：881 ms
- [`std::map` (2 线程)](https://gist.github.com/stjepang/b172a4259c0439d2855bc68fd47b3ab7)：1127 ms

以及 Java 中互斥锁保护的 `TreeMap` 和 `ConcurrentSkipListMap`：

- [`TreeMap (1 线程)`](https://gist.github.com/stjepang/3bc21528f5cf82ecd564778f8a861b11)：1211 ms
- [`TreeMap (2 线程)`](https://gist.github.com/stjepang/da69ad273ea2cf2e13b4322c0ea6bd74)：1409 ms
- [`ConcurrentSkipListMap (1 线程)`](https://gist.github.com/stjepang/f6f289c07759f47a72b0565fd6b992c7)：2181 ms
- [`ConcurrentSkipListMap (2 线程)`](https://gist.github.com/stjepang/74d1abc7230ad6e6dd0c4aec1f4cab4b)：1353 ms

结论：在单线程场景下，`SkipMap` 的性能应与任何典型的二叉搜索树相当（尽管不如 B 树）。随着线程数增加，它似乎具有良好的扩展性。我没有高核心数的机器来更充分地测试扩展性，但目前的数据仍然令人鼓舞。

### 迭代

跳表支持方便的迭代。注意，在遍历 `SkipMap` 时，我们会为其中的每个条目分发一个 `Entry`。创建一个 `Entry` 需要增加其引用计数，而在从一个条目移动到另一个条目时，还需要固定当前线程。这涉及大量的引用计数更新和固定操作。

以下是在一百万个随机插入的条目上迭代的基准测试数据：

- `BTreeMap` (Rust)：18 ms
- `SkipMap` (Rust)：113 ms
- `std::map` (C++)：93 ms
- `TreeMap` (Java)：41 ms
- `ConcurrentSkipListMap` (Java)：32 ms

有趣的发现：

- `BTreeMap` 上的迭代非常快——这并不意外，因为相邻元素被分组到块中。
- `SkipMap` 是这里最慢的映射。我尝试测量了没有引用计数更新和固定操作时的迭代时间，结果大约是 95 ms，与 C++ 的 `std::map` 非常接近。此外，引用计数和固定操作确实带来了可衡量的开销，但并非*巨大*的开销。
- Java 很快——甚至 `TreeMap` 都比 C++ 的 `std::map` 快。这怎么可能？答案在于 Java 的 GC 会时不时触发，在内存中移动分配好的节点（它是一种压缩式 GC），并尽量将链接的节点布局得尽可能近，从而优化缓存效率。
- 让我们试试用 `-XX:NewSize=1024m` 选项调整 Java 的 GC。该选项将新生代大小设置为 1024 MB（一个巨大的数字），这意味着压缩永远不会触发。确实，迭代时间现在大不相同了——`TreeMap` 需要 124 ms，`ConcurrentSkipListMap` 需要 110 ms。现在这与 `SkipMap` 和 `std::map` 更加接近了。

### `Entry` 中引用计数的成本

在遍历跳表时，我们使用 `Entry`，它本质上是指向跳表节点的引用计数指针。这意味着遍历 100 个元素需要付出 200 次原子递增和 200 次原子递减的成本。

插入、删除或搜索元素的方法也返回 `Entry`，这意味着它们也需要花费一些成本来递增和递减节点的引用计数。

当前的跳表实现没有提供避免引用计数（即避免使用 `Entry`）的替代方法，但将来我们应该讨论如何添加它们。大体上，有三种通用的替代方案：克隆、guard 和闭包。以下用 `SkipMap::get` 方法来说明：

```
// 引用计数：返回一个 `Entry`。
//
// 这是我们当前的方法签名。
fn get(&self, k: &K) -> Option<Entry<K, V>>;

// 方案 #1：返回元素的克隆。
//
// 这意味着我们要付出克隆的代价，但如果克隆很廉价，这不是问题。
fn get_clone(&self, k: &K) -> Option<V> where V: Clone;

// 方案 #2：返回元素的一个 guard 引用，同时保持线程固定（pinned）。
//
// 主要的缺点是用户必须小心不要让 guard 存活太久，否则垃圾回收会被阻塞。
fn get_guard(&self, k: &K) -> Option<Guard<K, V>>;

// 方案 #3：接受一个闭包，在线程仍被固定时对找到的元素执行操作。
//
// 同样，缺点是用户必须小心不要让闭包运行太久，否则垃圾回收会被阻塞。
fn get_with<F: FnOnce(&V)>(&self, k: &K, f: F);
```

# Drawbacks

跳表在性能方面并不令人兴奋。哈希表、B 树（Bw-Tree 是一种无锁 B 树变体）和基数树（ART——自适应基数树可以实现并发）通常性能更好。然而，这些更快速的数据结构不如跳表通用，必须通过限制支持的操作集或降低 API 的易用性来做出牺牲。

# Alternatives

一些可能类似但可选的数据结构包括：

1. 自适应基数树（键只能是字节数组）。
2. Skip tree（在内存中移动元素，从而限制了 API）。
3. Bw-Tree（在内存中移动元素，从而限制了 API）。

# Unresolved questions

- `Entry` 是否应该重命名为 `Cursor`？
- 如何通过避免引用计数来使迭代更快？
- 我们需要哪些 `Entry` API 的替代方案，以及如何将它们整合进来？
