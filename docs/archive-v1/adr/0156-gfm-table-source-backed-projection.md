# ADR 0156：GFM table 的 source-backed projection 契约

## 状态

已接受（2026-08-17）

## 背景

Yu 的 block scanner 目前把 GFM table 保留为 `Paragraph`，但产品层仍需要知道它
是一个 table，才能在后续 layout 阶段绘制网格、实现 cell hit-test 和做局部编辑。
如果直接把 cell 文本复制成新的 document model，就会破坏 Markdown source 是唯一真源
以及 projection/layout 必须绑定同一 Revision 的约束。

## 决策

1. `yu-markdown::parse_table_in_snapshot` 只 materialize 候选 paragraph 的临时字符串；
   返回的 `TableBlock` 只保留 source byte ranges，不拥有 cell 文本。
2. `TableBlock` 暴露 source range、header、delimiter、body rows、physical row ranges 和
   alignment；所有范围都指向同一 `TextSnapshot`。
3. `yu-projection` 增加 `TableProjection` 与 `BlockProjectionKind::Table`。header/body 仍
   使用 source-backed inline projection，但 parser-owned delimiter physical row 已隐藏；
   delimiter 仍只通过 source range 保留，不在 projection 中伪造 HTML 或富文本。
4. table projection 经过 strictly-outside edit 时映射自身 cell/row ranges；触及 table
   内容则由 block projection cache 重新构建。
5. macOS storage FFI 增加稳定 tag `YU_STORAGE_PROJECTION_TABLE = 7`，并通过
   `yu_storage_session_projected_table_cells` 以 count/fill ABI 暴露 UTF-16 cell ranges。
   ABI 中 `row = 0` 为 header，`row = 1` 为 delimiter，`row >= 2` 为 body rows。

## 结果

- Rust、Swift 和未来 layout 都可以消费同一组 parser-owned cell ranges；没有第二份
  canonical 文本。
- 原有 block kind ABI 不变，table 仍报告为 paragraph；新的语义只在 projection kind
  层出现，降低增量 parser 与旧 host 的兼容风险。
- `TableLayoutSnapshot` 已按外部 metrics 生成可见 cell 的列宽、行高、bounds 和 source-backed
  hit-test；macOS storage FFI 通过 count/fill ABI 暴露 geometry，scene 仍需后续负责边框、
  selection overlay 和表格重排。
