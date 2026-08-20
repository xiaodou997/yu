# Source Projection Spike：2026-08-10

## 目标

验证 Markdown delimiter 隐藏后，source range、visual range 和 caret mapping 仍能保持可逆，
同时不物化 Piece Tree/Rope 的完整 Snapshot。

## 当前模型

```text
TextSnapshot(Revision)
        │
        ▼
yu-markdown::parse_inline(source range)
        │
        ▼
Projection::from_inline
        │
        ├── Visible(source range → visual range)
        └── HiddenSyntax(source range → zero width)
```

parser-owned token stream 覆盖双星号、双下划线、单星号、单下划线和等长度 code delimiter；
unmatched、escaped delimiter 保持可见。ProjectionBias Before/After 用于 hidden syntax 两侧的
source caret。

## 验证

```text
7 projection tests passed
3 inline token tests passed
strong delimiter: 4 source bytes → zero visual width
source/visual mapping: Before/After boundary round-trip
subrange scan: does not include delimiters before requested range
empty range: single source boundary maps to visual zero
Piece Tree: materialized_buffers = 0
```

## 限制

VisualOffset 仍是 projected UTF-8 byte offset；InlineDocument 仍是 flat token layer，尚未处理
完整 CommonMark inline 语义、style、layout、glyph、links/images 或 projection 后的 IME
surrounding text。
