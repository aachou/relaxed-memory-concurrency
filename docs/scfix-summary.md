# RC11 (Repaired C11) 论文总结

**论文**：*Repairing Sequential Consistency in C/C++11*  
**作者**：Ori Lahav, Viktor Vafeiadis, Jeehoon Kang, Chung-Kil Hur, Derek Dreyer  
**会议**：PLDI 2017  
**原文**：<https://plv.mpi-sws.org/scfix/>

---

## 问题一：SC 原子操作语义过强

### 症状

C11 要求 SC 全局 total order `sc` 包含 `hb ∪ mo ∪ rb`（happens-before, modification order, reads-before），这个条件过于严格，导致之前声称正确的两种向 Power 的编译方案（leading sync 和 trailing sync）实际上是 **unsound** 的（Power 硬件允许的行为被 C11 禁止，编译无法忠实反映语义）。

### 修复

将 SC 条件从 `sc ⊇ (hb ∪ mo ∪ rb)` 弱化为：

> `scb`（sc-before）= `(rf ∪ mo ∪ rb)^+` 在 SC 位置上的限制  
> 要求 `[E^sc]; (sb ∪ rf ∪ mo ∪ rb); [E^sc]` 无环

### 效果

- 不改变纯 SC 程序的 DRF-SC 保证
- 不影响不使用 SC 访问的程序
- 恢复向 Power/ARM 编译的正确性
- 若 SC 和 non-SC 访问不混用到同一位置，则与原 C11 等价

---

## 问题二：SC fence 语义过弱

### 症状

即使在每条指令之间都插入 `atomic::fence(SeqCst)`，也不足以恢复顺序一致性。Batty et al. 的条件 `[F^sc]; sb; (hb ∪ mo ∪ rb); [F^sc]` 无环存在一个漏洞——`rb; rf` 组合可以绕过 SC fence 的限制形成环。

### 修复

- `scb` 定义为全 `(rf ∪ mo ∪ rb)^+`（而非 `hb ∪ mo ∪ rb`）
- 加强 `psc` 条件：在 SC fence 序列的组合上要求无环

### 效果

修复后，两个 SC fence 之间的所有访问都与 SC total order 一致；在所有访问间插入 SC fence 即可恢复 SC。

---

## 问题三：Thin-air 读取

### 症状

Relaxed 访问允许因果循环（out-of-thin-air reads），例如 `x=1; y=1; r1=y; r2=x` 可能出现 `r1=r2=1` 这种自我满足的结果。

### 修复

要求 `(po ∪ rf)` 无环。

### 编译方案

首次形式化证明 Boehm & Demsky 的保守方案（relaxed read 后插入 fake control dependency）在 Power 和 ARMv7 上是正确的。

---

## RC11 模型总览

| 缺陷 | C11 原问题 | RC11 修复 |
|------|-----------|-----------|
| SC 原子语义 | `sc ⊇ (hb ∪ mo ∪ rb)` 导致 Power 编译失败 | `scb = (rf ∪ mo ∪ rb)^+` 的 total order |
| SC fence 语义 | `rb; rf` 组合可绕过 SC 限制 | `scb` 包含全 `(rf ∪ mo ∪ rb)^+` |
| Thin-air reads | relaxed 允许因果循环 | `(po ∪ rf)` 无环 |

---

## 形式化证明

- 向 x86-TSO 的编译正确性（§4）
- 向 Power 的编译正确性（§5）
- 向 ARMv7 的编译正确性（§6）
- 编译器优化的可靠性（§7）
- DRF-SC 保证 + SC fence 可恢复 SC

---

## 关联工具

- **RC11 `.cat` 文件**：用于 herd7 模型检测的可执行公理语义规范
- **RCMC**：基于 RC11 的有界模型检测工具
- **c11comp**：C11 程序变换的推理工具
