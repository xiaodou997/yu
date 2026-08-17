# ADR 0158：GFM table 的 source-backed scene primitive

## 状态

已接受（2026-08-17）

## 背景

`TableLayoutSnapshot` 已提供可见 cell 的 source range 与 block-local bounds。scene 层需要
消费这些 geometry，但不能重新解析 Markdown，也不能让表格装饰变成第二份文本模型。当前
生产窗口仍显示 source/projection fallback，因此这一阶段先建立可保留、可渲染、可丢弃的
diagnostic scene contract。

## 决策

1. `yu-scene::TablePrimitive` 保存 `TextRange`、scene bounds、color 与
   `TablePrimitiveRole`（`HeaderFill`、`SelectionFill`、`Border`），不保存 cell 文本。
2. `SceneBuilder::append_table_with_selection` 验证 layout Revision，按 origin 平移几何，
   由 source selection 命中 cell 后生成 selection fill，并从列宽/行高生成外围和内部网格。
3. `yu-render` 暂时将 table role 降级为现有 `FillRect` command，避免为诊断阶段引入新的
   GPU backend ABI；scene 中的 semantic role 仍可供未来 native selection/accessibility 层消费。
4. 生产窗口暂不调用 table scene overlay，直到 visual projection 改为不再绘制 Markdown pipe
   文本；否则 header fill/border 可能覆盖仍显示的 source glyph。

## 结果

- 表格装饰完全绑定 layout/source Revision，stale layout 在 scene 构建前被拒绝。
- header、selection、border 的 source-backed geometry 可独立测试、可映射到 render plan，
  但没有引入 HTML/DOM 或复制 cell 文本。
- 后续 production visual table 接入只需在正确的 painter order 调用现有 scene API，不必
  改动 Markdown parser 或 native FFI 的 source coordinate contract。
