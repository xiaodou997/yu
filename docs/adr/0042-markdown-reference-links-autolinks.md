# ADR 0042：Markdown reference links and autolinks

## 状态

已接受（Phase 1）

## 背景

0040 只支持 inline destination 的 `[]()` link/image。Markdown 文档中常见的显式 reference
形式 `[label][id]`、collapsed 形式 `[label][]`，以及 `<https://…>`/`<email>` autolink 仍被
当作普通 punctuation；projection 无法隐藏它们的语法，同时也容易把 HTML-like `<div>` 误当成
链接。

## 决策

- `InlineSpanKind` 增加 `ReferenceLink`、`ReferenceImage` 和 `Autolink`。`InlineSpan` 增加
  `reference: Option<TextRange>`；reference span 的 `destination` 为空，autolink 的
  `destination` 指向 angle brackets 内的原文范围。
- reference parser 只识别 `[label][id]`、`[label][]` 及其 image 形式；第一个 `]` 到第二个
  `]` 的完整 tail 作为 `closing`，避免 projection 把 label 后的 `]` 留在视觉文本中。
- `InlinePunctuation` 增加 `<`/`>` angle token。只有 ASCII URL scheme 或保守的 ASCII email
  形状通过 autolink 校验；未闭合 angle、HTML-like tag、line break 和 code span 内的 angle
  不生成 semantic span。
- projection 继续统一隐藏 span 的 `opening`/`closing`，新 span 默认使用 Plain label/text style；
  parser 不生成 normalized URL、definition table 或 HTML AST。

## 结果

- reference label/alt 与 autolink text 保持 source-backed 可编辑文本，syntax range 可双向映射。
- `<div>`、未闭合 reference、shortcut reference 和 code 内的 link-like text 不会被错误隐藏。
- 未来 block-level definition resolver 可以在不改变 inline source range 契约的情况下补充 shortcut
  reference 与 destination 解析。

## 限制

本阶段不解析 reference definition block、shortcut reference、完整 CommonMark URI/email Unicode
规则、HTML 标签或 destination 规范化；这些语义必须由后续 Markdown definition/extension 层明确
提供，不能由 projection 猜测。
