# 原始 EBR 算法（Keir Fraser, 2004）

源自 Fraser 的博士论文 *Practical Lock-Freedom*。

---

## 核心思想

epoch 只循环 3 个值：**0 → 1 → 2 → 0 → 1 → 2 → …**

对应 **3 个退役列表** `retire_list[0]`、`retire_list[1]`、`retire_list[2]`：

| epoch | 含义 |
|-------|------|
| 当前 epoch | 正在活跃的世代，新退役的节点放这里 |
| 前一 epoch | 上一世代退役的节点，**还不能释放** |
| 前两 epoch | 两代前的节点，**现在可以安全释放** |

---

## 两个关键观察

**1. 为什么要 3 个 epoch？**

```
epoch:       0 —→ 1 —→ 2 —→ 0 —→ 1 …
           释放      释放      释放
          epoch-2   epoch-2   epoch-2
```

每次推进 epoch，就把 `当前 - 2` 的那个队列清空。因为 3 个值轮转，`(e - 2) mod 3` 总是唯一的。

**2. 推进条件保证了什么？**

推进到 `e` 的前提是：**所有活跃线程的本地 epoch 都等于 e**。这意味着：

- 在 epoch **0** 时退役的节点 → 放入 `retire_list[0]`
- 推进到 epoch **1** 时，所有活跃线程的本地 epoch 等于 1 或 0 → 它们可能还在访问 `retire_list[0]` 里的节点
- 推进到 epoch **2** 时，所有活跃线程的本地 epoch 都不可能等于 0 → 两轮之后，`retire_list[0]` 里的节点**肯定没人访问了** → 释放

---

## 原始伪代码

```
// 共享状态
global epoch: 0..2, 初始 0
per thread: active(bool), local_epoch(0..2)
per thread: retire_list[3]

// 开始访问共享数据
function pin():
    local_epoch = global_epoch
    active = true

// 结束访问
function unpin():
    active = false

// 退役一个节点（从数据结构中移除）
function retire(node):
    retire_list[local_epoch].push(node)

// 尝试推进 epoch
function try_advance():
    for each thread:
        if thread.active && thread.local_epoch != global_epoch:
            return          // 有人还固定在旧 epoch，不能推进
    // 所有活跃线程都追上了
    global_epoch = (global_epoch + 1) % 3
    // 清空两代前的那一批
    free_all(retire_list[(global_epoch + 2) % 3])
```

---

### 正确性直觉

假设你在 epoch **0** 时退役了节点 N：

```
时间:    epoch 0         epoch 1         epoch 2
        N被退役
        放入list[0]      推进到1         推进到2
                        ↓               ↓
线程A:   pin在0,读N       unpin 或 repin   必不在0
线程B:   pin在0,读N       unpin 或 repin   必不在0
                                       list[0] 可释放 ✅
```

- epoch **0**：N 被退役，可能有人在读 N
- epoch **1** 推进：所有活跃线程的本地 epoch 必须 = 0，可能还有人在读 N
- epoch **2** 推进：所有活跃线程的本地 epoch 必须 = 1 → 说明 epoch 0 的访问者已经全部退出，`retire_list[0]` 安全释放 ✅

**3 个 epoch 就够了**，因为只需要区分"当前"、"上一代"、"上两代"——上两代的访问者肯定已经不存在了。
