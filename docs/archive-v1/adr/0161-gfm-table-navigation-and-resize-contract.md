# ADR 0161：GFM table 的 source-backed 导航与 resize 命中契约

## 状态

已接受（2026-08-17）

## 背景

Yu 已经能从同一 Markdown source 生成 table projection、可见 cell layout、cell glyph 和
source hit-test。下一步需要为编辑器快捷键和 macOS pointer adapter 固定坐标边界，但不能
因为表格 UI 而引入第二份 cell 文本或在 native 层重新解析 `|`。

## 决策

1. `yu_markdown::TableCellAddress` 是 visible-only 的 row-major 坐标。`row = 0` 表示 header，
   body 从 `row = 1` 开始；delimiter physical row 保留为 parser source range，但不占用 visible
   address。`TableBlock::visible_cell_for_source`、`next_visible_cell` 和
   `previous_visible_cell` 只返回 canonical snapshot 中的 `TableCellRange`。
2. `EditorCommand::MoveTableCellNext/Previous` 仅在 caret 位于当前 table 的 visible cell 时
   生效。命令把 focus/caret 移到目标 cell 的 source 起点，不写入 `TextBuffer`，不改变
   Revision、selection history 或 Undo group。当前表格首/尾没有目标 cell 时返回 `Unhandled`；
   自动追加行、删除行和格式化由后续 transaction 设计负责。
3. `TableLayoutSnapshot::resize_hit_test` 和
   `yu_storage_session_table_resize_hit_test` 是 Revision-bound 的只读几何查询。它们只报告
   内部 column/row divider、divider 前的 index 和 table-local axis position；outer edges、
   table 外点、非法 tolerance、非 table block 和 stale Revision 都拒绝。输出不包含文本，
   查询不改变 source、selection、history 或 layout cache。
4. macOS/native adapter 可以用 resize hit 启动后续 pointer gesture，但本 ADR 不定义 drag
   的 intermediate state、column width 的 Markdown 序列化或自动补 row。真正的宽度/行高
   transaction 必须另行定义 source representation 和 Undo 语义后才允许进入产品窗口。

## 结果

- Tab/Shift-Tab 和 pointer resize 都复用 parser/layout 的 source-backed ranges，不会产生
  table mirror。
- Swift 不需要扫描 Markdown delimiter，也不会把 delimiter row 当成可编辑或可见 row。
- FFI 可以在 stale Revision 时安全拒绝旧点，native caller 不会把过期几何提交给后续编辑命令。
- 当前阶段仍不切换完整 visual table editor；TextKit source/IME/Accessibility fallback 保持
  不变。

## 验证

- `yu-markdown` 验证 visible navigation 跳过 delimiter row。
- `yu-editor` 验证 Tab/Shift-Tab 只移动 source caret，且 table 尾部保持 `Unhandled`。
- `yu-layout` 验证内部 column/row divider、tolerance 和 outer edge 行为。
- `yu-storage-ffi` 验证 ABI kind/index/position、非命中清零和 stale Revision 拒绝。
