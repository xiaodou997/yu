# Markdown Parser

## Full Parse

Phase 1 parser 不要求连续源码：

```text
TextSnapshot
    ↓ ChunkCursor
single-pass line scan
    ↓
LineAnalysis + composable source hash
    ↓
persistent lossless BlockSequence
```

行扫描跨 Piece/rope leaf 保留 CRLF 原始 byte range。`LineAnalysis` 只缓存 block 分类需要的有限
状态，不复制普通正文。每行在同一次扫描中计算可组合 hash，block 不需要再次查找源码。
parser 前后 retained Snapshot 的 materialized buffer 数必须不变。

`BlockSequence` 由不可变 `Arc<[BlockRecord]>` 分段组成。增量结果通常包含：

```text
old prefix allocation (delta = 0)
new middle allocation  (delta = 0)
old suffix allocation  (delta = edit byte delta)
```

suffix 的绝对 source range 通过 segment delta 延迟映射，因此文本长度变化不会迫使复制所有旧
block。相邻且来自同一 allocation 的连续 segment 自动合并。

## Block Lifetime

`retained_markdown_stats()` 对多个 `MarkdownDocument` 的 text Snapshot、segment table 和 block
allocation 按指针去重，统计 allocation 的完整长度，而不是仅统计被 segment 引用的 slice。这样
可以识别“大 allocation 被小 suffix 钉住”的保留放大。

同步增量 parser 不执行 block compaction。`BlockCompactionPolicy` 只给 idle task 提供建议：

```text
segments > 4096
    OR
reclaimable records >= 8192 AND retained records > active blocks * 4
```

`compact_blocks()` 将当前活动 record 复制到一个新 allocation，成本为 O(blocks)。调用方应在
输入空闲且可以释放旧 checkpoint 时执行；Undo/history 仍保留旧文档时频繁压实会同时保留新旧
完整 allocation，反而显著增加内存。

## Incremental Parse

输入契约：

```text
previous MarkdownDocument (revision N)
ChangeSet (N -> N+1)
new TextSnapshot (revision N+1)
```

算法使用分段内二分找到最早 old range 所在 block，并回退一个 block 处理相邻 paragraph 合并。
边界通过 `ChangeSet::map_anchor(..., Affinity::Before)` 映射到新 Snapshot。新的流式 parser 逐个
产生 block，并与最后一个 edit 之后的旧 block 比较。

```text
reused old prefix
        +
parse(new_start..convergence)
        +
reused shifted old suffix
        ↓
MarkdownDocument(N+1)
```

只有以下条件全部成立才允许收敛：

1. mapped source range 相同；
2. block kind、start state 和 end state 相同；
3. source hash 相同；
4. 旧、新 Snapshot 的对应 range 逐 byte 相同。

hash 只是快速过滤器，不是正确性依据。`MarkdownDocument` 保留其不可变 `TextSnapshot`，用于
hash 命中后的零拷贝 chunk 比较。围栏删除等状态扩散无法满足上述条件，会继续扫描到 EOF。

## Differential Validation

任何增量优化必须继续满足：

```text
incremental document == full document
lossless coverage == true
document revision == snapshot revision
```

当前 Phase 1 block state 覆盖 normal/fenced EOF 状态；完整 CommonMark 容器栈、inline 增量状态
和稳定 syntax node identity 仍待后续阶段定义。每种新状态都必须继续通过随机 differential test
和围栏类病理 edit。
