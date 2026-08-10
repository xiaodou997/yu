# ADR 0023：语义 inline span 与独立 fenced-code projection

- 状态：Accepted
- 日期：2026-08-10

## 背景

`yu-projection` 早期只消费 flat delimiter tokens，并在 projection 内完成配对。这样虽然能
验证 source/visual mapping，却会让 parser 和 projection 各自拥有一份 Markdown 语义判断；
fenced code 也不能直接复用 inline projection，因为代码正文中的 `**`、反引号等字符都是字面量。

## 决策

`yu-markdown::InlineDocument` 在 lossless token stream 之上产生 parser-owned `InlineSpan`：

- `Emphasis`、`Strong`、`CodeSpan` 保存 source、opening、content、closing ranges；
- unmatched 或 escaped delimiter 不生成 span；
- code span 内的 delimiter 不参与外层 emphasis/strong pairing。

`yu-projection::Projection` 只消费这些 span，生成 hidden syntax runs 和带
`Plain/Emphasis/Strong/Code` style 的 visible runs，不再重新配对 delimiter。

fenced code 使用独立的 `CodeProjection`：

- opening/closing fence line 是 hidden syntax；
- info string、content 和 closing fence 都保留 source range；
- body 只生成 Code-styled visible run，不经过 inline parser；
- unclosed fence 的 content 延伸到 block EOF。

`BlockProjection` 统一暴露普通 block 的 inline projection 与 fenced code projection，
`ProjectionCache` 按现有 `(TextRange, BlockKind)` key 复用和映射。

## 结果

- Markdown 语义的唯一来源回到 parser-owned spans；
- 后续 layout 可以直接消费 styled runs，而不需要再次识别 delimiter；
- 代码块不会把正文误渲染成 Markdown emphasis；
- prefix/suffix edit 可以同时映射 inline 和 code projection。

## 限制

当前 spans 仍是保守的 Phase 1 配对模型，不是完整 CommonMark inline tree；CodeProjection 只定义
source/visual 与 fence/content 边界，尚未实现语法高亮、代码编辑 overlay 或 layout。
