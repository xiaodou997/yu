# ADR 0009：持久化 Block Sequence 与状态收敛

- 状态：Accepted
- 日期：2026-08-09

## 背景

ADR 0008 的增量 parser 能保证 fence 状态安全传播，但每次编辑都会复制 `Vec<Block>` 前缀并从
重解析边界扫描到 EOF。大文件的局部编辑成本仍与编辑位置相关，无法满足“只处理变化范围”的
目标。

只比较 block hash 不足以成为正确性边界：hash 存在碰撞，Markdown 状态也可能使相同源码产生
不同解释。文本长度变化还会使未变化 suffix 的绝对 source range 整体平移。

## 决策

`MarkdownDocument` 使用不可变分段 `BlockSequence`。每个 allocation 是
`Arc<[BlockRecord]>`，segment 保存 allocation slice 和 lazy byte delta。增量解析组合共享旧前缀、
新 middle 和共享旧 suffix；suffix range 在读取时应用 delta，不复制 block record。

每个 record 保存：

```text
BlockKind
SourceRange
StartState
EndState
SourceHash
```

行扫描同时计算可组合 rolling hash，block hash 由行 hash 合并，不为每个 block 再次建立源码
cursor。hash 只用于过滤；命中后必须借助旧、新 `TextSnapshot` 的 `ChunkCursor` 逐 byte 确认。

增量 parser 从最早受影响 block 的前一个 block 开始，直到一个 edit 之后的旧 block 同时满足：

```text
mapped range equal
kind equal
start/end state equal
hash equal
source bytes equal
```

满足后停止流式扫描并共享该 block 及其全部 suffix。否则继续到下一 block 或 EOF。block offset
定位在 segment 内二分，避免 near-end edit 线性遍历整个 block sequence。

## 正确性

- 三种 text backend 各执行 1,000 次随机 edit，incremental 与 full parse 完全相同；
- 普通局部 edit 明确验证 prefix/suffix allocation 共享及 shifted range；
- 插入或删除 fence 明确验证不会因重复正文/hash 提前收敛；
- hash 命中仍做源码 byte equality，因此碰撞不改变结果。

## 后果

10 MiB、约 699,000 blocks 的 fixture 上，near-start/middle/near-end edit 都只扫描 42–74 bytes，
三个 storage backend 的增量解析中位数约 5–9 us。一次结果共享 699,052 个旧 block，通常只有
三个 segment、两个 allocation。

完整扫描因同步 hash 从上一轮约 18 ms 增至约 25–26 ms。该成本换取稳定的微秒级局部重解析，
后续可通过更快的 rolling hash 或向量化扫描继续优化，但不能移除 byte equality 的正确性确认。

ADR 0010 已补充长期 session benchmark、去重 retained bytes 统计与显式 idle compaction 策略。
