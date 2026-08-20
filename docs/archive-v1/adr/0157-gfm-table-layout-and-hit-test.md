# ADR 0157：GFM table 的 source-backed layout 与 hit-test 契约

## 状态

已接受（2026-08-17）

## 背景

`TableProjection` 已经能识别 GFM table 并保存 parser-owned source ranges，但 layout 仍需要
一个不复制 cell 文本的几何模型。macOS native host 也需要稳定地获得可见 cell bounds，并将
鼠标点反查为同一 Revision 的 source range。

## 决策

1. `yu-layout::TableLayoutSnapshot` 以 `TableProjection`、`LayoutConfig` 和外部 metrics/shaper
   为输入，只保存列宽、统一行高、可见 header/body cell bounds、alignment 和 source ranges。
2. delimiter physical row 不是可见 row；它的完整 source range 继续由 `TableProjection` 保留，
   供 source selection、编辑和后续 scene 诊断使用。
3. 可见 row 使用 `row = 0` 表示 header，body rows 从 `row = 1` 开始。`hit_test` 只返回
   visible cell，命中结果携带 cell bounds、point 和 source range。
4. `yu_storage_session_table_layout_cells` 使用 count/fill ABI 返回 Revision-bound UTF-16
   source range、bounds 和 alignment；`yu_storage_session_table_cell_hit_test` 返回同一
   Revision 的命中 cell。过期 Revision、非 table block 和 table 外点都必须拒绝。
5. `yu-scene::TablePrimitive` 消费同一 `TableLayoutSnapshot` 生成 source-backed header fill、
   selection fill 和 border geometry；`yu-render` 当前把这些 semantic roles 映射成现有
   solid-fill command。该阶段仍不切换生产窗口的 visual table overlay。

## 结果

- native host 不需要扫描 `|`、解析 delimiter 或复制 cell 文本。
- table layout 可以在严格外部编辑后映射 source ranges，同时复用不变的几何；table 内容
  变化则由 projection/layout cache 重新构建。
- Rust 单元测试、storage FFI 回归、scene/render 回归和 macOS block self-check 共同验证
  delimiter 隐藏、四个可见 cell、列/行几何、center alignment、hit-test、semantic table
  primitive 和 stale Revision 行为。
