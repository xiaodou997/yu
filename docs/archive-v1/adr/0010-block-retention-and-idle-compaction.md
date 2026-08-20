# ADR 0010：Block Retention 与 Idle Compaction

- 状态：Accepted
- 日期：2026-08-09

## 背景

持久化 `BlockSequence` 让一次局部 edit 只增加少量 segment 和 block allocation，但连续 session
会逐步增加 segment table。直接在 segment 数超过小阈值时压实，需要复制当前文档的全部
`BlockRecord`。对 10 MiB fixture 而言约有 699,000 records，复制成本和历史版本保留成本都不可
忽略。

仅统计活动 slice 也会低估内存：一个很小的 suffix slice 可能仍通过 `Arc` 保留完整旧 allocation。

## 决策

新增 `retained_markdown_stats()`，对一组不可变文档保留的下列 allocation 按指针去重：

- text Snapshot 及其持久化节点/文本 buffer；
- `Arc<[BlockSegment]>` table；
- `Arc<[BlockRecord]>` allocation 的完整 record 数和字节数。

同步 `parse_incremental()` 不自动压实。`MarkdownDocument` 提供显式：

```text
needs_block_compaction(policy)
compact_blocks()
compact_blocks_if_needed(policy)
```

默认 policy 在以下任一条件成立时建议 idle compaction：

```text
segments > 4096

reclaimable records >= 8192
AND retained records > active blocks * 4
```

压实把所有 resolved record 写入一个 delta 为零的新 allocation。它不改变文档语义、Revision、
hash 或 source range，但成本为 O(blocks)，因此调用方必须把它安排在输入空闲阶段。旧 Undo 或
checkpoint 文档仍被保留时，上层应延后压实或先缩减历史，因为新旧完整 allocation 会同时存在。

## 阈值实验

10 MiB、1000 次随机增量 edit、保留 8 个 Markdown revisions，Piece Tree 后端：

| 策略 | Parse mean | 最大 segments | Compaction | 最大单次 compaction | Retained estimate |
| --- | ---: | ---: | ---: | ---: | ---: |
| 512 segments | 18.65 us | 514 | 3 次 | 17.38 ms | 181.17 MiB |
| 4096 segments | 49.10 us | 1993 | 0 次 | 0 | 53.83 MiB |

512 阈值虽减少 segment lookup，但三次完整复制令 retained block bytes 从约 42.8 MiB 增至约
170.4 MiB，并引入超过一帧的空闲任务。4096 下 segment table 在 8 个版本中合计约 374 KiB，
远小于 record allocation；49 us 的 session parse mean 仍有充分输入延迟余量。因此选择 4096。

## 正确性

- 共享空 edit revision 的 block allocation 和 segment table 只统计一次；
- 删除 90% 文档后，policy 检测 10 倍 retention amplification 并压回活动 record 数；
- 三种文本后端各执行 500 次连续增量 edit，小阈值测试中 segment 始终有界；
- 每 25 次 edit 与 full parse 比较，compaction 前后文档完全相等。

## 后果

当前 flat segment directory 的查找成本仍随 segment 数线性增长。若真实 session 超过数千
segments 且 49 us 级成本开始显著，下一步应把 directory 改为持久化 B-tree，而不是降低阈值并
频繁复制全部 records。`BlockRecord` 当前为 64 bytes，也是后续 CST 内存设计必须收紧的指标。
