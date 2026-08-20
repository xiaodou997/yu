# ADR 0164：table transient geometry 的 storage FFI

## 状态

已接受（2026-08-17）

## 背景

ADR 0163 已决定列宽调整只存在于当前会话/帧，不写回 GFM Markdown。Rust editor 已能从
`TableResizeCommit` 构造 transient layout，但 macOS host 还只能读取 canonical cell geometry，
无法验证一次拖动后的 cell rectangles。

## 决策

新增 `yu_storage_session_table_layout_cells_with_resize`，沿用现有 table layout count/fill
ABI，并额外接受：

- expected Revision 和 parser block index；
- `YU_STORAGE_TABLE_RESIZE_COLUMN`；
- column divider index；
- pointer-derived column delta。

Rust 在单次调用中取得 canonical block layout，应用 `LayoutSnapshot::apply_table_column_resize`，
再返回 owned `YuStorageTableLayoutCell` 数组。调用结束后 transient snapshot 被丢弃；session、
source、selection、history 和 layout cache 不变。ROW kind、非法 index、非有限 delta、stale
Revision 都拒绝。

Swift bridge 只做 count/fill 和 scalar assertions，不保存 override，不扫描 Markdown。真实
drag 的 pointer gesture 生命周期仍由上一阶段的 `TableResizeGesture` 管理；本 ABI 是诊断/布局
消费边界，不是 source transaction。

## 结果

- native host 可以在不启动完整 GUI 的情况下验证“拖动后几何变化、canonical 几何不变”。
- Rust 继续拥有表格解析和布局，Swift 不会产生第二份 table model。
- variable-row、跨会话持久化和 Markdown 写回继续后置。

## 验证

- Rust FFI 测试覆盖 count/fill、`[3,3] → [4,2]`、canonical 查询不变、ROW/NaN/stale 拒绝。
- document-host block projection self-check 消费 transient geometry，并验证 source 与 canonical
  layout 仍保持原值。
