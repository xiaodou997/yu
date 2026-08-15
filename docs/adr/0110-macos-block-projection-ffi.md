# ADR 0110：macOS parser-owned block projection FFI

## 状态

已接受（Phase 3 Track A）。

## 背景

inline projection 已经可以通过 macOS storage FFI 返回 visual UTF-8 和 source↔visual caret。
下一步需要让 native layout 以 block 为单位工作，但不能让 Swift 复制 Markdown parser、block
range 或 projection cache，也不能把 Rust 借用跨过 FFI 生命周期。

## 决策

`DocumentEditorSession` 继续拥有唯一 source、Revision 和 projection cache，并以两个
Revision-bound C ABI 查询提供 block 诊断快照：

1. `yu_storage_session_projection_block_count` 返回当前 revision 的 parser-owned block 数量；
2. `yu_storage_session_projected_block` 通过 block index 返回 source UTF-16 range、稳定 parser
   kind、stable projection kind、visual UTF-8/UTF-16 长度和 owned visual UTF-8；空 buffer/零容量
   形式只用于长度查询，但仍填充完整 metadata。

block index 和 metadata 只对 expected Revision 有效。stale revision、越界 index、转换失败或
projection 构建失败必须在写入 metadata 前返回错误。Swift 只消费 owned scalar/bytes，不解析
Markdown，也不持有 `Block`、`Projection` 或 `TextSnapshot` 指针。

## 后果

- native host 可以先做 block-local projection、长度和 source-range 校验，为后续 viewport/lazy
  layout 准备边界；
- 当前 `TextKit` source mirror 仍是生产回退路径，block projection 还不是最终 visual renderer；
- heading/list 等 block 的完整 delimiter 隐藏语义、composition selection、point hit-testing 和
  GPU scene 仍需后续阶段定义，不能由 Swift 在本接口上猜测。
