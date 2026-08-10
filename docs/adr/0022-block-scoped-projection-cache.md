# ADR 0022：以 Markdown block range/kind 管理 editor projection

- 状态：Accepted
- 日期：2026-08-10

## 背景

任意 source range 可以验证 Projection API，但产品编辑器的可见内容首先由 Markdown block
决定。若 cache 只按裸 range 管理，block 合并、拆分或 fenced-code 分类变化时，旧 entry 可能
被错误地当成可复用内容。

## 决策

`EditorDocument` 同时持有与当前 TextBuffer Revision 对齐的 `MarkdownDocument`。每次永久
Transaction 成功后，使用 `yu_markdown::parse_incremental` 更新 block sequence，再执行
ProjectionCache 的 range remap，并按新的 block range/kind 保留 entry。

`ProjectionCache` 的产品路径使用 `(TextRange, BlockKind)` 作为 key：

- `block_projection(index)` 从当前 `MarkdownDocument` 取得 Block，不重新猜测范围；
- 同一 block range/kind 在当前 Revision 命中 cache；
- prefix/suffix edit 先通过 ChangeSet 映射 range，再由新 block sequence 验证仍匹配；
- block 内容、边界或 kind 变化会失效；
- fenced code block 不通过 inline Projection，后续由独立的 CodeProjection 负责；
- 原有裸 `projection(range)` 保留为底层实验入口，后续产品调用优先使用 block API。

## 结果

- block parser、projection 和 editor selection 共享同一个 Revision 边界；
- block 合并/拆分和 fenced-code 分类变化不会复用错误的 visual runs；
- 未受影响 block 可以继续复用 projection，便于后续测量 block 级增量成本。

## 限制

当前 block projection 已能区分 parser-owned semantic inline spans 与 fenced code projection，
但尚未建立完整的 inline tree、layout 或 viewport virtualization。CodeProjection 也尚未包含
语法高亮或代码编辑 overlay。
