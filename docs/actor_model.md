# Actor 模型详解

## 1. 核心概念

Actor 模型由 Carl Hewitt 在 1973 年提出，是一种并发计算模型。它的核心概念极其简单：

**Actor 是并发计算的通用原语。** 一个 Actor 就是一个有邮箱的独立计算单元。

每个 Actor 有三个要素：
1. **状态（State）** — Actor 内部私有的数据，其他 Actor 不能直接访问
2. **行为（Behavior）** — 收到消息后做什么
3. **邮箱（Mailbox）** — 消息队列，其他 Actor 发来的消息先入队，Actor 依次处理

## 2. 三个基本操作

Actor 能做且仅能做三件事：

| 操作 | 说明 |
|------|------|
| **创建** | `create()` — 创建新的 Actor |
| **发送** | `actor ! message` — 向目标 Actor 发送消息（异步，不阻塞） |
| **改变状态** | 更新自己的内部状态，以处理下一条消息 |

没有共享内存，没有锁，没有条件变量。**通信的唯一方式是消息传递。**

## 3. 关键特性

**封装性：** Actor 的内部状态完全私有。不存在"两个 Actor 同时读写同一份数据"的情况——数据竞争从根本上被消除。

**逐条处理：** Actor 一次只处理一条消息，处理完才取下一条。所以**单个 Actor 内部是顺序的（sequential）**，但不同 Actor 之间完全并行。

**位置透明：** 发送消息时你不关心目标 Actor 在哪个线程、哪台机器上。系统负责路由。

**容错：** "Let it crash" 哲学——监督者（supervisor）监控子 Actor，崩溃时按策略重启。

## 4. 工作示例（伪代码）

```typescript
// 定义一个 Counter Actor
actor Counter {
    var count = 0

    // 处理消息
    on message Increment {
        count += 1
    }
    on message GetValue(sender) {
        sender ! count  // 把当前值发回去
    }
}

// 使用
var c = Counter.create()
c ! Increment          // 异步加 1
c ! Increment          // 异步加 1
c ! GetValue(self)     // 查询值
```

这里 `Increment` 和 `GetValue` 会进入 Counter 的邮箱，按**发送顺序**依次处理。不会有"两个 Increment 同时执行"的情况。

## 5. 和 BoC 的对比

| 维度 | Actor 模型 | BoC |
|------|-----------|-----|
| 基本单元 | Actor（封装状态+行为） | Cown（纯状态）+ Behaviour（纯行为） |
| 通信方式 | 消息传递（Actor → Actor） | `when` 声明资源集合（声明式） |
| 跨资源操作 | ❌ 困难，需要额外机制（如 Saga、Orchestration） | ✅ `when(a, b, c)` 原子获取多个 cown |
| 顺序保证 | 同一 Actor 的消息按入队顺序执行 | 重叠 cown 集合的 behaviour 按 happens-before 排序 |
| 并行粒度 | 按 Actor 粒度 | 按 behaviour 粒度（更细） |

核心区别一句话：**Actor 的"消息"绑定到单个接收者；BoC 的 behaviour 声明式地绑定到一组资源。**

## 6. 实际应用

- **Erlang/OTP** — Actor 模型的鼻祖，电信级容错
- **Akka** — JVM 上的 Actor 实现
- **Orleans** — Microsoft 的 Virtual Actor
- **Pony** — 类型安全的 Actor 语言（参考了 BoC 论文相关研究）

## 7. Actor 的痛点（也是 BoC 想解决的核心问题）

假如你有一个**转账**操作：从 A 账户扣钱，加到 B 账户。在 Actor 模型里：

```typescript
actor Account {
    var balance: U64
    
    on message Withdraw(amount, replyTo) {
        if (balance >= amount) {
            balance -= amount
            replyTo ! Succeeded
        }
    }
    
    on message Deposit(amount) {
        balance += amount
    }
}
```

转账函数：

```typescript
function transfer(from, to, amount) {
    from ! Withdraw(amount, self)
    // 问题：何时知道 Withdraw 成功了？什么时候执行 Deposit？
    // 需要等回复，然后 to ! Deposit(amount)
    // 但这破坏 Actor 模型：你在用同步方式等异步结果
}
```

常见解法：
1. **Ask 模式** — 发消息等待回复（本质上是阻塞，不推荐）
2. **回调用 Actor** — Withdraw 成功后由目标 Actor 发消息触发下一步（链式回调，复杂）
3. **Saga 模式** — 分布式事务补偿机制（重量级）

而在 BoC 里就是一句：

```typescript
when(src, dst) {
    if (src.balance >= amount) {
        src.balance -= amount
        dst.balance += amount
    }
}
```
