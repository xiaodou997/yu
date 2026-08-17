# ADR 0159：GFM table 的 cell-only visual projection

## 状态

已接受（2026-08-17）

## 背景

`TableLayoutSnapshot` 和 `TablePrimitive` 已经能从 parser-owned cell ranges 生成表格几何，
但旧的 `TableProjection` 仍把 header/body 的 Markdown pipe 和行换行作为普通 visual text。
如果现在把 table overlay 接到生产 scene，网格会和重复的 source glyph 重叠；如果复制 cell
文本到另一份富文本模型，又会破坏 Markdown source 是唯一真源的约束。

## 决策

1. `TableProjection` 继续消费同一个 lossless inline CST 和 `TableBlock`，不创建 cell 文本副本。
2. header/body 每个物理行只保留 `TableCellRange` 覆盖的 source bytes；pipe、cell 周围空白、
   CRLF/LF 行尾和其他行结构字节都作为 parser-owned zero-width `HiddenSyntax` runs。
3. delimiter physical row 继续作为完整 zero-width hidden range，delimiter cell ranges 仍保留在
   `TableBlock`，供 alignment、source selection 和 FFI 查询使用。
4. visible run 的 source range 必须仍然是 canonical snapshot range；`source_to_visual` 与
   `visual_to_source` 在 cell 边界使用同一 projection bias 契约。严格位于 table 之前或之后的
   edit 继续只映射 ranges，触及 table 内容则重新解析。
5. 当前阶段不把 cell glyph 自动放入生产 scene。下一阶段由 layout/scene 消费 cell-only runs，
   按 `TableLayoutSnapshot` 的列、行和 alignment 定位 glyph，并保持 selection/border 的
   painter order。

## 结果

- table visual stream 不再包含 Markdown pipe、delimiter 文本或物理行尾，避免 scene overlay
  与 source glyph 重叠。
- 每个可见字节仍能通过 projection 双向映射到原始 cell source range；编辑、复制、IME 和
  undo 继续走统一 Transaction/Revision 路径。
- 在 cell glyph 定位完成前，通用 layout 只能把这些 runs 当作连续 source-backed text；生产
  窗口继续保留现有 fallback，不会因为诊断 projection 提前显示错误的表格几何。
