# ADR 0006：选择 Piece Tree 作为主文本后端

- 状态：Accepted
- 日期：2026-08-09

## 背景

平坦 `Arc<str>` 参考后端可以验证编辑语义，但中部编辑需要复制整个文档，无法满足大文件目标。
在真实 workload 之前直接指定 Piece Tree 或 Rope 都缺少证据，因此两个持久化候选先通过同一
套公开契约实现并比较。

## 候选

### Piece Tree

- 持久化隐式 treap；
- Piece 引用不可变原始或插入 buffer；
- split/merge 通过 path copying 保留旧 Snapshot；
- 合并引用同一 buffer 且源码范围连续的相邻 Piece。

### Persistent Rope

- 约 4 KiB UTF-8 叶片；
- 持久化平衡二叉树；
- split/concat 通过 path copying 保留旧 Snapshot；
- concat 时合并边界小叶片，单叶目标上限约 4 KiB。

## 共同契约

两者都通过同一个 `TextBuffer`、Transaction、ChangeSet、inverse 和 Snapshot API 暴露，必须
与平坦 reference backend 运行相同 model tests。`TextSnapshot` 持有结构 root，并只在旧 parser
调用 `as_str()` 时惰性展平一次。

## 证据

在 10 MiB UTF-8 Markdown、2,000 次确定性随机编辑和 8 个跨编辑历史快照的同机 release
workload 下：

| 指标 | Flat reference | Piece Tree | Persistent Rope |
| --- | ---: | ---: | ---: |
| 中部 insert + inverse | 6.11 ms | 1.25 us | 5.75 us |
| 2,000 random edits | 1.454 s | 4.27 ms | 10.41 ms |
| 8 snapshots 保留分配估算 | 79.78 MiB | 10.70 MiB | 18.18 MiB |
| 随机编辑后 chunks/leaves | 1 | 3,988 | 2,583 |

完整数据和估算边界见 `docs/experiments/storage-candidates-2026-08-09.md`。

Rope 的边界合并将随机编辑后的叶片数从 6,533 降到 2,583，但复制边界叶使多版本文本分配
高于 Piece Tree。Piece Tree 在当前 workload 中同时具有更低编辑延迟和更低历史版本成本。

## 决策

1. `PieceTree` 成为 `StorageBackend::default()` 和 `TextBuffer::new` 的产品默认后端。
2. `FlatReference` 继续作为 model test 的正确性 oracle。
3. `PersistentRope` 暂时保留为实验后端，用于防止数据结构选择过早固化和比较后续摘要成本。
4. parser 在 chunk iterator 可用后不得依赖完整 `as_str()`；惰性连续视图只作为兼容路径。

## 后果与复审条件

接下来优先为 Piece Tree 节点加入行数和 UTF-16 长度摘要、定义 chunk cursor，并建立增量解析
differential harness。若这些元数据使 Piece Tree 的节点成本或更新延迟明显恶化，或 Rope 在真实
长时间编辑 trace 上扭转快照成本结论，可以通过新的 ADR 复审本决策。

摘要实现后的 10 MiB 复测中，Piece Tree 与 Rope 的 2,000 次随机编辑分别为 26.38 ms 和
27.25 ms，8 个 Snapshot 保留估算分别为 11.22 MiB 和 18.57 MiB，未触发复审条件。坐标与
cursor 契约见 ADR 0007。
