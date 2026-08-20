# ADR 0041：Markdown line-break projection and layout

## 状态

已接受（Phase 1）

## 背景

0040 固定了 inline parser 的 `LineBreak { hard }` token，但 projection 仍把换行当作普通可见
源码，layout 还需要从每个 visible run 的字符串中重新寻找 `LF`。这样 hard break 的两个尾随
空格/反斜杠会错误地产生宽度，也让 CRLF 的 source/visual mapping 依赖 layout 的二次扫描。

## 决策

- `VisualRunKind` 增加 `LineBreak { hard }`。run 的 source/visual range 只覆盖 LF 或 CRLF，
  因而 source 与 visual byte length 保持一致。
- hard-break token 的两个尾随空格或反斜杠成为零宽 `HiddenSyntax` run；canonical TextSnapshot
  不改变，Before/After bias 仍可在 marker 两侧解析 caret。
- `Projection::from_inline` 只消费 parser-owned `InlineNodeKind::LineBreak`，不重新扫描普通
  文本来判断换行。fenced code 继续使用独立 `CodeProjection`，其 body 保持字面量行为。
- metrics 与 shaped layout 都直接消费 `LineBreak` run，产生一个零宽 line-break cluster、结束
  当前 `VisualLine`，并从 ending 后的 source/visual boundary 开始下一行。

## 结果

- soft LF、hard LF 和 hard CRLF 使用相同的 source ↔ visual/caret/hit-test 语义。
- hard-break marker 不参与宽度、grapheme wrapping、shaping 或 glyph placement。
- layout 不再需要为 inline Markdown 重新解析尾随空格、反斜杠或 CRLF；code projection 的
  literal newline 兼容路径仍保留。

## 限制

当前 visual byte projection 仍保留 line-ending bytes 的 visual width；它们是控制 layout 换行的
source-backed boundary，并不是最终 GPU 字形。soft-break 的产品级折叠/空格策略、paragraph
合并规则和完整 CommonMark line-break 语义留到后续 layout/renderer 阶段。
