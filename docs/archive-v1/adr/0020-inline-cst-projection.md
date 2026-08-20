# ADR 0020：Projection 消费 parser-owned inline token ranges

- 状态：Accepted
- 日期：2026-08-10
- 取代：ADR 0019 中的 projection 私有 delimiter scan

## 背景

Projection 需要识别可隐藏的 Markdown delimiter，但不能维护一套与 Markdown parser 分离的
第二份语法扫描结果。否则 parser、selection 和视觉映射可能对同一个源码 revision 得出不同
的 delimiter 边界。

## 决策

`yu-markdown::parse_inline` 先从 `TextSnapshot` 和 source range 建立 lossless
`InlineDocument`。它只保存有序的 source-backed `InlineNode`：

- Text：普通源码范围；
- Escaped：反斜杠及其后续 UTF-8 scalar；
- Delimiter：连续的星号、下划线或 code delimiter 范围。

`yu-projection::Projection::from_inline` 只消费这个 parser-owned token stream，并从 token range
计算隐藏 syntax 与 source/visual mapping。`Projection::inline` 只作为便捷入口，内部先调用
`parse_inline`，不再自行扫描源码。

## 结果

- Markdown parser 与 projection 共享同一份 lossless source ranges；
- Piece Tree/Rope 输入通过 `ChunkCursor` 流式扫描，不调用 `TextSnapshot::as_str`；
- Projection 的 mapping API 保持稳定，后续可把 token layer 替换为更完整的 inline CST；
- parser-owned token stream 可以独立进行 coverage、UTF-8 boundary 和 revision 测试。

## 限制

当前 `InlineDocument` 是 flat token layer，不是完整 CommonMark semantic tree。Delimiter pairing
仍然保守，不处理完整的 flanking rules、嵌套语义、链接/图片、HTML、数学或 malformed Markdown。
这些规则应在后续 inline CST 扩展中加入，而不是重新在 projection 中实现。
