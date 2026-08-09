# Incremental Markdown：2026-08-09

## Workload

```bash
cargo run --release -p yu-bench -- \
  --size-mib 10 \
  --iterations 10 \
  --random-edits 2000 \
  --retained-snapshots 8
```

fixture 为约 10 MiB、699,000 个以上 Phase 1 blocks 的重复 Markdown。增量 workload 在文档中部
插入 heading，保守算法从前一个 block 重解析到 EOF。环境为开发机 macOS arm64、Rust 1.97
release build；结果只用于同机相对比较。

## 结果

| 指标 | Flat reference | Piece Tree | Persistent Rope |
| --- | ---: | ---: | ---: |
| Full block parse | 18.99 ms | 17.80 ms | 18.23 ms |
| Incremental block parse | 12.28 ms | 12.32 ms | 16.37 ms |
| Reparsed bytes | 5,242,960 | 5,242,960 | 5,242,960 |
| Reused prefix blocks | 349,525 | 349,525 | 349,525 |

所有 tree parser workload 在解析前后都保持 `materialized_buffers == 0`，因此结果不包含隐式
`as_str()` 连续化。Piece Tree 的中部增量 parse 比 full parse 降低约 31%，但没有随重解析字节
减半而减半。

主要剩余成本是：

1. `Vec<Block>` 仍复制约 349,525 个 prefix blocks；
2. suffix 需要重新分配 line/block vectors；
3. 尚未通过 block end-state/hash 在状态收敛后复用旧 suffix。

下一轮 benchmark 应分别测 near-start、middle、near-end edit，并在持久化 block sequence 完成后
记录 prefix/suffix 的共享节点数，而不只看 reparse range。

## 正确性

- 三个存储后端各运行 1,000 次随机 Unicode/Markdown edit differential test；
- full 与 incremental document、Revision、range 和 block kind 完全相同；
- opening/closing fence 删除显式验证向 EOF 传播；
- full parser 跨 Piece 读取 delimiter、CRLF 和 Unicode 空白且不 materialize Snapshot。
