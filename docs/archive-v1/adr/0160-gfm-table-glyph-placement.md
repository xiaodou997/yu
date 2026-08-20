# ADR 0160：GFM table 的 source-backed glyph 定位

## 状态

已接受（2026-08-17）

## 背景

0159 将 GFM table 的 visual stream 收敛为 cell-only runs，隐藏 pipe、周围空白、行尾和
delimiter physical row。若继续沿用普通段落的线性 layout，cell glyph 会挤在同一条线上，
并且命中测试可能回到隐藏结构字节。table scene decoration 也必须和 glyph 使用同一份
Revision-bound geometry。

## 决策

1. `TableLayoutSnapshot` 通过 projection visible runs 测量每个 cell 的 content width，并保存
   `VisualRange`、cell bounds、alignment、content origin 和可见 row source range；不复制 cell
   文本。
2. `LayoutSnapshot` 在 table projection 上按 source cell range 重定位 visible cluster/glyph，
   使列 x、可见 row y、baseline 和 alignment 与 `TableLayoutSnapshot` 一致。delimiter row 不
   生成可见 line，header 是 row 0，body row 从 1 开始。
3. table caret/hit-test 优先使用 cell-aware source boundary resolver；结构性 zero-width run 的
   `Before`/`After` 查询都落到可见 cell source boundary，而不是 pipe、周围空白或行尾；cell
   内部位置仍交给普通 projection 保留 grapheme 精度。
4. `yu-scene::SceneBuilder` 在 viewport 批量构建时先提交 table header/selection/border
   `TablePrimitive`，再提交 source-backed glyph；selection 仍是 source `TextRange`，scene 不
   保存 cell 文本，也不解析 Markdown。
5. 当前产品仍保留 native TextKit source/IME/Accessibility fallback。完整 visual renderer
   和 macOS window 的最终 table overlay 切换另行决策。

## 结果

- table glyph 的 x/y、line、caret 和 hit-test 都由同一 source-backed layout 契约驱动，center/
  right alignment 不需要 Swift 重新测量。
- table decoration 不再覆盖 cell glyph，scene/render 可以继续把 semantic role 降级为 solid
  fill command。
- `incremental_parse`、selection、IME、undo、copy/paste 和 Revision 仍以 Markdown source
  为唯一真源；本阶段没有引入 HTML、DOM 或富文本副本。

## 验证

- `yu-layout` shaped table regression 验证 cell glyph 的列/行位置和隐藏结构 hit-test。
- `yu-workspace` viewport regression 验证 header/selection/border decoration 在 glyph 前发布。
- 全 workspace tests、clippy、format、macOS document-host block projection self-check 继续
  作为提交门槛。
