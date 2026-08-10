# ADR 0016：Chunk-aware Unicode 边界与 EditorDocument Accessibility 快照

- 状态：Accepted
- 日期：2026-08-10

## 背景

`TextSnapshot::as_str()` 会为 Piece Tree/Rope 建立连续缓存。它适合需要完整源码的 parser，
但不适合每次左右移动、Backspace 或 Delete 时调用。与此同时，Accessibility 查询如果只接收
裸 `TextRange`，平台层仍可能把 selection 保存成另一份状态。

## 决策

`yu-text::TextSnapshot` 增加 `chunk_before`，与已有 `chunk_cursor` 配对。各后端在自己的
树结构中定位前一个 chunk，避免通过 `offset - 1` 猜测（该值可能落在 UTF-8 scalar 中间）。

`yu-editor` 使用 `unicode-segmentation::GraphemeCursor` 在 chunk 之间流式推进：

```text
TextSnapshot chunks
        │
        ├── next_boundary
        ├── prev_boundary
        └── provide pre-context when a piece boundary is crossed
```

组合重音、ZWJ emoji、区域指示符等边界不会因为 Piece Tree 分段而改变，命令路径不物化整个
Snapshot。

`AccessibilityTextSnapshot::from_document(&EditorDocument)` 成为 canonical 入口。它一次性
获取 source、selection 和 Revision，并拒绝不属于同一 Revision 的 selection；已有的
`new(TextSnapshot, TextRange)` 保留给 parser/layout 或测试等显式快照场景。

## 结果

- Unicode command 的单次移动/删除不再隐式复制完整文档；
- Flat、Piece Tree、Persistent Rope 共享 chunk-before 行为契约；
- Accessibility 的 selected range 直接来自 EditorDocument selection；
- FFI、平台 view 和后台任务仍只能使用 caller-owned/immutable 坐标与 Snapshot。

## 限制

GraphemeCursor 仍按需扫描相邻 chunks；它不是完整的 grapheme boundary index。大规模连续
光标导航若成为瓶颈，再增加按 block/line 的边界缓存，而不是恢复全文物化。
