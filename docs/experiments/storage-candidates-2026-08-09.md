# Text Storage Candidates：2026-08-09

## Workload

```bash
cargo run --release -p yu-bench -- \
  --size-mib 10 \
  --iterations 10 \
  --random-edits 2000 \
  --retained-snapshots 8
```

环境为开发机 macOS arm64、Rust 1.97 release build。数字用于同机候选对比，不是产品性能
承诺，也不应跨硬件直接比较。

## 首轮结果：合并前

| 指标 | Flat reference | Piece Tree | Persistent Rope |
| --- | ---: | ---: | ---: |
| 构造 10 MiB | 2.87 ms | 0.68 ms | 1.94 ms |
| Snapshot median | 83 ns | 42 ns | 41 ns |
| 首次连续视图 | ~0 | 0.97 ms | 0.62 ms |
| Full block scan | 16.50 ms | 17.27 ms | 17.76 ms |
| 中部 insert + inverse | 4.95 ms | 0.88 us | 2.25 us |
| 2,000 random edits | 1.50 s | 4.46 ms | 6.95 ms |
| Random edit mean | 752 us | 2.23 us | 3.47 us |

Snapshot 时间相近是预期结果：三者都只克隆共享 root/`Arc` 并创建轻量 Snapshot wrapper。Piece
Tree/Rope 第一次调用 `as_str()` 仍需完整展平；后续 parser 应直接遍历 chunk。

10 MiB 初始 Rope 有 2,561 个叶片、高度 13。2,000 次随机编辑后：

| 候选 | Chunks/Leaves | Nodes | Height |
| --- | ---: | ---: | ---: |
| Piece Tree | 3,988 | 3,988 | 34 |
| Persistent Rope | 6,533 | 13,065 | 16 |

这轮结果要求两个树候选都加入局部合并后再测，不能据此直接选择后端。

## 第二轮结果：局部合并与历史快照

Piece Tree 现在会合并引用同一 `Arc<str>` 且范围连续的边界 Piece。Rope 会在 concat 时合并
总长不超过约 4 KiB 的边界叶。相同 10 MiB workload 的结果为：

| 指标 | Flat reference | Piece Tree | Persistent Rope |
| --- | ---: | ---: | ---: |
| 构造 10 MiB | 2.05 ms | 0.58 ms | 1.99 ms |
| Snapshot median | 42 ns | 42 ns | 42 ns |
| 首次连续视图 | ~0 | 0.68 ms | 0.61 ms |
| Full block scan | 16.38 ms | 16.67 ms | 16.34 ms |
| 中部 insert + inverse | 6.11 ms | 1.25 us | 5.75 us |
| 2,000 random edits | 1.454 s | 4.27 ms | 10.41 ms |
| Random edit mean | 727 us | 2.14 us | 5.20 us |

反复 insert + inverse 后，Piece Tree 回到 1 个 Piece，Rope 回到初始 2,561 个叶片。2,000 次
随机编辑后：

| 候选 | Chunks/Leaves | Nodes | Height |
| --- | ---: | ---: | ---: |
| Piece Tree | 3,988 | 3,988 | 34 |
| Persistent Rope | 2,583 | 5,165 | 14 |

Rope 的局部合并有效解决了首轮最明显的叶片爆炸，但每次合并边界叶需要复制最多约 4 KiB
文本。这一代价在保留历史 Snapshot 时会累积。

### Retained Snapshot 成本

benchmark 在随机编辑的整个时间线上均匀保留 8 个 Snapshot，并按 `Arc` 地址去重统计仍存活的
Snapshot、树节点和文本 buffer：

| 指标 | Flat reference | Piece Tree | Persistent Rope |
| --- | ---: | ---: | ---: |
| Snapshots | 8 | 8 | 8 |
| 唯一树节点 | 0 | 9,088 | 16,792 |
| 唯一文本 buffers | 8 | 2,001 | 4,523 |
| 保留分配估算 | 79.78 MiB | 10.70 MiB | 18.18 MiB |

这个估算包括 Snapshot struct、节点 struct 和实际文本字节，但不包括 allocator/`Arc` header、
HashSet 自身或容器预留容量，因此是候选之间的相对指标，不是 RSS。两个树候选的保留快照均未
调用 `as_str()`；如果兼容 parser 展平每个历史版本，还要额外增加对应文档大小的连续缓存。

1 MiB workload 也保持同样排序：Flat 7.78 MiB、Piece Tree 1.69 MiB、Rope 5.92 MiB。

## 第三轮结果：节点摘要与 Chunk Cursor

两个树候选加入 UTF-16/LF 摘要；Piece buffer 每约 4 KiB 增加稀疏 prefix checkpoint。benchmark
同时测量 byte/UTF-16/line 往返、初始 chunk 遍历和 2,000 次编辑后的碎片化 chunk 遍历。

| 指标 | Flat reference | Piece Tree | Persistent Rope |
| --- | ---: | ---: | ---: |
| 构造 10 MiB | 2.60 ms | 14.45 ms | 8.80 ms |
| 初始 chunk scan | 41 ns | 83 ns | 15.96 us |
| 坐标 round-trip | 25.97 ms | 10.13 us | 3.63 us |
| 中部 insert + inverse | 5.22 ms | 12.96 us | 14.38 us |
| 2,000 random edits | 1.470 s | 26.38 ms | 27.25 ms |
| 碎片化后 chunk scan | 41 ns | 48.75 us | 31.75 us |
| 8 snapshots 保留分配估算 | 79.78 MiB | 11.22 MiB | 18.57 MiB |

Flat 坐标转换仍需扫描源码。Rope 因固定小叶在坐标查询和碎片化遍历上更快；Piece Tree 初始
文档通常只有一个 Piece，历史快照保留成本更低，中部编辑与随机 workload 仍略快。Piece 的
checkpoint 和增大的节点使其 8-Snapshot 估算从 10.70 MiB 增至 11.22 MiB，未改变 ADR 0006
的主后端结论。

新增 model test 在每次随机 Unicode edit/inverse 后比较完整内容和 `TextSummary`，并周期性验证
byte/UTF-16 往返。独立测试覆盖 CRLF、空末行、emoji surrogate split、跨 Piece 行定位和从
任意 UTF-8 边界 seek 的 cursor 重建。

## 决策

1. 平坦后端继续作为正确性 oracle，不参与产品选择。
2. Piece Tree 成为产品默认后端：它在本 workload 中编辑更快，历史快照保留成本也更低。
3. Persistent Rope 暂时保留为实验对照，不再阻塞后续 Piece Tree 元数据和 chunk cursor 工作。
4. 行/UTF-16 摘要和 chunk cursor 已完成；下一轮让 Markdown parser 消费 chunk，并建立完整/
   增量解析 differential harness。
5. Snapshot 估算还需要用进程级 RSS/allocator instrumentation 交叉验证。
