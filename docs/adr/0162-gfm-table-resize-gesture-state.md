# ADR 0162：GFM table resize gesture 的 source-neutral 状态

## 状态

已接受（2026-08-17）

## 背景

`yu_storage_session_table_resize_hit_test` 已能返回 Revision-bound 的内部 column/row divider。
如果 native `mouseDown`/`mouseDragged`/`mouseUp` 直接把点位写回 Markdown，会把暂时的 pointer
几何、不可表达的 row height 和未来的 column width serialization 混在一起，也会让 stale
surface 继续修改新 Revision。

## 决策

1. `yu_layout::TableResizeGesture::begin` 捕获 layout Revision、block index、`TableResizeTarget`、
   命中的 divider position 和 mouse-down pointer position。pointer tolerance 造成的偏移保留
   在 anchor 中，不由 native 重新推算。
2. `update` 只接受同一 Revision 的有限 pointer 位置，并计算 pointer delta 与临时
   `proposed_position`；它不改 TextBuffer、selection、history、projection 或 layout cache。
3. `finish` 返回 `TableResizeCommit`，其中只有 Revision、block、target、初始/最终 divider
   position 和 delta。`cancel` 丢弃状态。commit 不是 editor transaction，也不能直接序列化
   为 Markdown；后续阶段必须单独定义列宽表示、最小宽度、row height 语义和 Undo 行为。
4. 任意 update/finish/cancel 的 Revision 不匹配都返回 stale error；NaN/无穷 pointer 被拒绝。
   document-host self-check 通过现有 FFI hit-test 验证 native 能消费同一命中结果，但产品窗口
   暂不启动真实 table drag。

## 结果

- native pointer adapter 可以安全地拥有短生命周期 gesture，而不会制造 table source mirror。
- stale surface 或旧 pointer 事件不会写入当前文档。
- “拖到哪里”和“如何把宽度/高度表达回 Markdown”成为两个独立问题，便于后续测试和设计。

## 验证

- `yu-layout` 覆盖 anchor offset、临时 proposed position、commit、stale update 和非有限点。
- `yu-storage-ffi` 覆盖 column/row divider ABI、tolerance、source 不变和 stale Revision。
- macOS document-host block projection self-check 消费同一 FFI 命中查询并验证这些边界。
