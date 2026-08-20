# Persistent Block Sequence：2026-08-09

## Workload

```bash
cargo run --release -p yu-bench -- \
  --size-mib 10 \
  --iterations 10 \
  --random-edits 1000 \
  --retained-snapshots 8
```

fixture 为 10,485,810 bytes、约 699,000 个 Phase 1 blocks。分别在 1%、50%、99% 位置插入
heading。环境为开发机 macOS arm64、Rust 1.97 release build；数字只用于同机相对比较。

## 结果

| Backend | Full parse | Near start | Middle | Near end | 扫描 bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| Flat reference | 24.95 ms | 7.00 us | 5.42 us | 5.13 us | 42–74 |
| Piece Tree | 26.46 ms | 7.33 us | 5.13 us | 4.92 us | 42–74 |
| Persistent Rope | 25.35 ms | 9.42 us | 9.00 us | 6.88 us | 42–74 |

三种位置均共享 699,052 个旧 block。增量文档由三个 segment、两个底层 allocation 组成：一个
旧 allocation 同时提供 prefix/suffix，一个新 allocation 保存重解析 middle。

## 优化过程

初版已能在 42–74 bytes 后收敛，但 near-end 仍需线性扫描 692,000 个 block，耗时约 4.3 ms；
完整解析又因逐 block 建立 `ChunkCursor` 计算 hash 增至 64–138 ms。修正后：

1. block offset 在每个 immutable segment 内二分，三个编辑位置不再随位置线性增长；
2. line scanner 在读取源码时计算可组合 hash，block hash 由 line hash 合并；
3. hash 命中后才建立 range cursor 做逐 byte 正确性确认。

完整 parse 目前约 25–26 ms，相比无 hash 版本约 18 ms 有可测成本；局部增量从上一轮中部
12–16 ms 降至 5–9 us。后续 benchmark 需要覆盖长期 session 的 segment 数量和 retained block
allocation bytes，而不是继续扩展 Phase 1 grammar。
