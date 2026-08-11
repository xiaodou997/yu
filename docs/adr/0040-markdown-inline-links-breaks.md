# ADR 0040：Markdown inline links, images and line breaks

## 状态

已接受（Phase 1）

## 背景

0039 只固定了 block-level source ranges。现有 inline parser 可以识别 delimiter token 和简单的
emphasis/strong/code span，但 link/image 的 `[]()` 结构、hard break 和 delimiter flanking 仍然
会被当作普通文本或过度配对。Projection 需要这些范围才能隐藏 link syntax，而 layout 需要明确
的 line-break node 才能保持 source/visual mapping。

## 决策

- `InlineNodeKind` 增加 `Punctuation` 和 `LineBreak { hard }`；`!`, `[]`, `()` 保持独立 source
  token，CRLF 作为一个 line-ending range。
- `InlineNode` 的 delimiter token 保存 parser 内部的 `can_open/can_close` flanking 结果；`_`
  在字母/数字之间不参与 emphasis pairing。code delimiter 仍按相同 run length 配对。
- `InlineSpanKind` 增加 `Link` 与 `Image`。span 保存 opening、label/alt content、closing tail 和
  destination range；destination 可以为空，title 语法先包含在 closing tail，后续再单独建模。
- 只有完整的 `[](...)` 结构产生 link/image span；未闭合结构、escaped bracket 或 code span 内的
  bracket 不产生 semantic span。
- 两个尾随空格或反斜杠加 line ending 生成 hard `LineBreak`，普通 LF/CRLF 生成 soft `LineBreak`。
  所有 bytes 仍由 node ranges 覆盖，parser 不做格式化或 HTML 转换。

## 结果

- `yu-projection` 可以只依赖 parser-owned span 隐藏 link/image syntax；source buffer 保持不变。
- layout 后续可以消费 `LineBreak`，不需要再次扫描反斜杠、尾随空格或 CRLF。
- chunk-aware parser、Piece Tree/Rope snapshot 和 full/incremental differential test 保持同一入口。

## 限制

当前 link parser 只覆盖 inline destination，不覆盖 reference link、完整 title AST、自动链接、HTML、
嵌套链接规则和完整 Unicode punctuation flanking；这些规则必须在后续 parser 阶段增加，而不是由
projection 私自补扫。
