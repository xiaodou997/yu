# ADR 0019：Source → Visual Projection 使用 source-backed runs

- 状态：Accepted
- 日期：2026-08-10
- 后续实现：ADR 0020

## 背景

Yu 的 Markdown 源码必须保持唯一真源，但 Typora 风格编辑又需要暂时隐藏 Markdown delimiter。
若把投影视图变成另一份可编辑文本，selection、Undo 和 Markdown round-trip 会重新出现双重状态。

## 决策

新增 yu-projection crate。第一版只接受一个 revision-bound TextSnapshot 和一个 source range，
输出 source-backed VisualRun：

```text
Source Range
    │
    ▼
yu-markdown::InlineDocument
    │
    ├── Visible(source range, visual range)
    └── HiddenSyntax(source range, zero visual width)
```

Projection 不保存可编辑的 projected string。可见 run 仍引用 source range；隐藏 delimiter 只在
run 表中占据零视觉宽度。source_to_visual 与 visual_to_source 使用 ProjectionBias Before/After
解决隐藏 syntax 两侧的 caret 边界。

首版 inline token layer 只做保守的对称 delimiter 配对：

- 双星号、双下划线、单星号、单下划线；
- 等长度的 code delimiter；
- unmatched、escaped delimiter 保持可见；
- code span 内的 emphasis delimiter 不参与配对。

这不是 CommonMark parser，也不承担 style、layout、glyph 或最终 editor command。

## 结果

- Markdown source 不会因投影而重新序列化；
- hidden delimiter 前后可以映射到明确的 source boundary；
- projection 只保留 runs 和 Snapshot 引用，不物化整个 Piece Tree/Rope Snapshot；
- 后续 yu-layout 可以在不改变 source model 的情况下消费 VisualRun；
- projection crate 通过 `Projection::from_inline` 消费 yu-markdown 的 inline token ranges，mapping
  API 不因 parser 接入而变化。

## 限制

VisualOffset 当前表示 projected UTF-8 byte offset，不是 glyph、grapheme 或屏幕 x 坐标。
inline token layer 和 delimiter pairing 仍是风险验证，不应宣称完整 Markdown 语义；后续仍需
处理嵌套、链接、图片、HTML、数学和 malformed Markdown。
