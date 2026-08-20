# Block Retention 与 Compaction：2026-08-09

## Workload

```bash
cargo run --release -p yu-bench -- \
  --size-mib 10 \
  --iterations 10 \
  --random-edits 1000 \
  --retained-snapshots 8
```

每次 `TextBuffer` edit 后立即运行 `parse_incremental()`；block compaction 单独计时，不并入 parse
time。按相同步长保留 8 个 `MarkdownDocument`，统计去重后的 text、block allocation 和 segment
table。环境为开发机 macOS arm64、Rust 1.97 release build。

## 默认 4096 Segment Policy

| Backend | Session parse total | Parse mean | Max/final segments | Compactions | Retained Markdown |
| --- | ---: | ---: | ---: | ---: | ---: |
| Flat reference | 113.15 ms | 113.15 us | 1993 / 1993 | 0 | 123.06 MiB |
| Piece Tree | 49.10 ms | 49.10 us | 1993 / 1993 | 0 | 53.83 MiB |
| Persistent Rope | 51.73 ms | 51.73 us | 1993 / 1993 | 0 | 57.89 MiB |

三个 backend 的 block retention 相同：1001 个 block allocations、701,262 records、44,880,768
record bytes；8 个 segment tables 共 7,987 segments、383,376 bytes。Piece Tree 的 text retained
allocation 约 10.66 MiB，因此总估算约 53.83 MiB。

## 512 Segment 对照

Piece Tree 在 512 阈值下执行 3 次 compaction，共重写 2,091,638 blocks：

```text
parse mean                 18.65 us
compaction total           48.40 ms
max single compaction      17.38 ms
retained block records     2,792,332
retained block bytes       178,709,248
retained Markdown total    181.17 MiB
```

更低阈值减少查找时间，却让历史版本同时钉住多代完整 record allocation。segment metadata 节省
不足 300 KiB，而 retained 总量增加约 127 MiB，因此不采用。

## 结论

1. compaction 必须显式安排在 idle/background 阶段，不能进入同步 parser；
2. 默认 4096 是 soft recommendation，不是 parser 内部强制行为；
3. 大删除导致 allocation slice 保留放大时，ratio policy 可以不依赖 segment 数触发；
4. 后续优化优先级是 persistent segment directory 和更紧凑的 record layout，而不是更频繁压实。
