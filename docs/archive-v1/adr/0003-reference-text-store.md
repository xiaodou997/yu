# ADR 0003：先用参考存储固定编辑契约

- 状态：Superseded by ADR 0006
- 日期：2026-08-09

## 背景

Piece Tree、B-tree Rope 和持久化 Rope 的选择需要真实 workload、Snapshot 成本、Anchor 更新和
内存数据，而 Transaction 语义不应被某个候选结构绑死。

## 决策

Phase 1 的 `TextBuffer` 使用平坦 `Arc<str>` 参考后端，优先固定 Snapshot、Transaction、
ChangeSet、inverse 和 Anchor 行为。它不作为性能实现，也不得用于证明大文件指标。

## 替换条件

候选存储必须：

1. 通过与参考后端相同的 model tests；
2. 支持廉价不可变 Snapshot；
3. 支持行、UTF-16 与边界摘要扩展；
4. 在固定编辑 workload 上提供可重复 benchmark；
5. 不改变上层 Transaction API。
