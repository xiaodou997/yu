# ADR 0007：持久化文本摘要与 Chunk Cursor

- 状态：Accepted
- 日期：2026-08-09

## 背景

macOS/AppKit 使用 UTF-16 坐标，parser 使用 UTF-8 byte offset，编辑器还需要按源码行定位。
如果每次转换都展平并扫描完整 Snapshot，大文件输入、IME surrounding text 和增量解析都会退化。

## 决策

每个持久化文本树节点维护可相加的 `TextSummary`：

```text
UTF-8 bytes
UTF-16 code units
LF line breaks
```

Piece 引用的不可变 buffer 每约 4 KiB 建立一个稀疏 summary checkpoint。Piece split 通过两个
prefix summary 相减获得子 Piece 摘要，最多扫描相邻 checkpoint 区间，不重新扫描整个原始
buffer。

源码行只由 LF byte 分隔。CRLF 因包含一个 LF 而计为一个行界；孤立 CR、Unicode LS/PS 暂不
作为源码行界。空 Snapshot 和以 LF 结尾的 Snapshot 都保留末尾逻辑行。

`TextSnapshot` 提供：

- byte、UTF-16 与零基 `LineIndex` 查询；
- surrogate pair/UTF-8 scalar 中间位置的显式错误；
- 从任意 UTF-8 byte boundary 开始的零拷贝前向 `ChunkCursor`。

Cursor 的第一个结果是包含 seek offset 的完整 chunk，并携带绝对源码起点；seek 到 EOF 返回
空 cursor。Cursor 借用 Snapshot，因此 chunk 不得跨 Snapshot 生命周期保存。

## 不包含

Grapheme、word boundary、BiDi 和 visual line 不进入 `TextSummary`。这些数据取决于编辑命令、
语言和布局上下文，应由上层按可见范围缓存。

## 后果

Piece Tree 的创建和 split 增加 summary/checkpoint 成本，但坐标查询不再随文档长度线性增长。
checkpoint 与节点字节计入 retained Snapshot benchmark。Markdown parser 已按 ADR 0008 直接
消费 Chunk Cursor，并用 full parse 对增量结果做 differential validation；当前下一步是持久化
block sequence 与 suffix reuse。
