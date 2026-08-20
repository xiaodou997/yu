# Text Buffer

## 数据所有权

`TextBuffer` 是一个 Revision 的可变入口，永久编辑只能通过 Transaction。`TextSnapshot` 持有
不可变持久化 root，可以跨线程读取；默认后端是 Piece Tree。

```text
Transaction
    ↓
Piece Tree root(revision N)
    ├── Piece → immutable buffer range
    ├── subtree TextSummary
    └── persistent children
             ↓ snapshot()
       immutable root handle
```

Flat backend 只作为正确性 oracle，Persistent Rope 是实验对照。三者必须通过同一套 model、
坐标和 chunk cursor 测试。

## Text Summary

`TextSummary` 是严格可相加的三个指标：UTF-8 bytes、UTF-16 code units、LF line breaks。Piece
Tree 节点保存整个子树摘要；不可变 Piece buffer 每约 4 KiB 保存 prefix checkpoint，支持快速
计算任意子 Piece 摘要。

逻辑行数定义为 `LF count + 1`。源码不会把 CRLF 归一化，CR byte 仍保留在 Piece 中。

## 坐标查询

所有查询都绑定一个 Snapshot Revision：

```text
ByteOffset  ──► Utf16Offset
ByteOffset  ──► LineIndex
Utf16Offset ──► ByteOffset
LineIndex   ──► line start ByteOffset
```

反向 UTF-16 查询不会把 surrogate pair 中间位置修正到附近边界，而是返回
`TextPositionError::Utf16InsideScalar`。同理，byte 查询拒绝拆分 UTF-8 scalar。

## Chunk Cursor

`snapshot.chunks()` 从源码开头遍历；`snapshot.chunk_cursor(offset)` 在 O(tree height) 内定位到
包含 offset 的 chunk。每个 `TextChunk` 只借用后端中的源码，并暴露绝对 `start/end` 与 `text`。

这允许 parser、搜索和导出逐 chunk 工作而不触发 `as_str()` 的 O(document bytes) 连续副本。
兼容代码仍可调用 `as_str()`，结果只在该 Snapshot 中惰性缓存一次。

## 当前复杂度

| 操作 | Piece Tree |
| --- | --- |
| Snapshot | O(1) |
| 整体 summary | O(1) |
| 中部 replace | O(log pieces + checkpoint scan) |
| byte/UTF-16/line 查询 | O(log pieces + checkpoint scan) |
| cursor seek | O(log pieces) |
| cursor 前向遍历 | O(chunks) |
| `as_str()` 首次连续化 | O(document bytes) |
