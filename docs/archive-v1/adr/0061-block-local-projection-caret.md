# ADR 0061：Block-local Projection Caret Query

## 状态

已接受（Phase 1 诊断）

## 背景

ADR 0060 的完整文档 projection caret 查询固定了 Revision 和 hidden syntax affinity，但它仍
以整份 source range 为 projection 输入。对大文档而言，caret 通常只需要当前 Markdown block
的 visual 坐标；若每次输入、命中测试或 reveal 都扫描整篇文档，projection cache 和 block
sequence 的局部性就没有被平台边界利用。

## 决策

- `yu-editor::EditorDocument` 暴露 `block_index_for_source`，集中维护 source boundary 到
  parser-owned block 的选择规则，native adapter 不复制 block range 遍历。
- `yu-editor-ffi` 增加独立的 `yu_composition_session_block_projection_caret`，不改变 ADR 0060
  的全局 visual offset ABI。
- 新 ABI 返回 owned 的 `revision`、`source_utf16`、`block_index`、block-local
  `visual_utf16`、`round_trip_source_utf16` 和 `affinity`。Projection 使用当前
  `EditorDocument::block_projection`，因此复用定义索引、task-list/code projection 和
  revision-bound cache。
- source UTF-16 boundary、Revision 和 affinity 的错误规则与 ADR 0060 相同；block 不存在或
  projection 映射失败时拒绝请求且清空 output。查询不修改 source、selection、composition、
  history 或 Revision。
- macOS spike 在第二个 Markdown block 的 `**block**` delimiter 上运行 block-local self-check，
  验证 visual offset 从该 block 的 0 开始，而不是继承前一个 block 的 visual length。

## 结果

- native caret/reveal 可以先确定 block，再把局部 visual 坐标交给 block layout，避免把整份
  Markdown 当作一次 caret 查询的输入。
- 完整文档 projection query 仍保留用于跨 block diagnostic；block-local ABI 是产品路径的
  性能边界，不暴露 Projection、TextSnapshot、layout cache 或平台句柄。
- 当前结果仍不包含 line/point 或 shaped geometry；后续 shaped block layout 可以在相同
  `block_index + local source/visual` 契约上添加独立查询。

