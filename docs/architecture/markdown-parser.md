# Markdown Parser

## Full Parse

Phase 1 parser 不要求连续源码：

```text
TextSnapshot
    ↓ ChunkCursor
single-pass line scan
    ↓
LineAnalysis + source ranges
    ↓
lossless Block sequence
```

行扫描跨 Piece/rope leaf 保留 CRLF 原始 byte range。`LineAnalysis` 只缓存 block 分类需要的有限
状态，不复制普通正文。parser 前后 retained Snapshot 的 materialized buffer 数必须不变。

## Incremental Parse

输入契约：

```text
previous MarkdownDocument (revision N)
ChangeSet (N → N+1)
new TextSnapshot (revision N+1)
```

算法找到最早 old range，定位其 block，并回退一个 block 处理相邻 paragraph 合并。边界通过
`ChangeSet::map_anchor(..., Affinity::Before)` 映射到新 Snapshot；稳定前缀之后的源码解析到 EOF。

```text
reused old prefix
        +
parse(new_start..EOF)
        ↓
MarkdownDocument(N+1)
```

这种策略对 fence 状态扩散是正确的，但还不是最终复杂度。当前 `Vec<Block>` 复制 prefix，解析
也没有在状态恢复时复用 suffix。

## Differential Validation

任何增量优化必须继续满足：

```text
incremental document == full document
lossless coverage == true
document revision == snapshot revision
```

未来 block state 至少需要 fence/container 状态、source hash 和稳定 node identity。引入提前停止
前，病理编辑必须继续安全传播到 EOF。
