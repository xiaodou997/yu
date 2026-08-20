# ADR 0076：Composition-aware Projection/Layout

## 状态

已接受（Phase 1 诊断；跨 block 规则由 ADR 0133 扩展）

## 背景

`CompositionOverlay` 已经由 `EditorDocument` 拥有，但此前 projection/layout 只能看到
canonical `TextSnapshot`。macOS `NSTextInputClient` 因此无法让 preedit 同时参与视觉换行、
shaping 和 caret 几何，而又不能把 marked text 写入 source、Revision 或 Undo。

## 决策

- `Projection::with_composition` 在现有 revision-bound projection 上生成 transient projection：
  替换范围外复用 parser-owned runs，替换范围由 `Composition` visual run 表示；preedit 永远按
  plain text 处理，不重新解析 Markdown。
- composition run 的 shaping 坐标使用从零开始的临时 `TextRange`，因为 preedit 的 UTF-8 长度
  与 canonical replacement range 长度可能不同；layout 在 glyph/cluster 产生后把它们映射回
  canonical replacement range，同时保留 visual range。
- `Projection` 提供 composition text、temporary shaping range、source/visual slice mapping
  和 preedit selection visual range；`LayoutSnapshot` 的 metrics/shaped 两条路径统一消费这些
  边界，不在 layout 内复制 overlay 状态。
- `EditorDocument::block_layout_with_composition*` 返回未缓存的 transient layout。composition
  更新不推进 canonical Revision，因此不能进入普通 `LayoutCache`，也不能通过 source edit
  remap；commit/cancel 后下次查询自然回到 canonical layout。
- block-local projection 是单 block fast path；跨 block replacement 的 span 投影与 viewport working
  state 规则由 ADR 0133 定义，不能把它误缩减成一个 block-local index。

## 结果

- IME preedit 可以参与换行、glyph shaping 和 visual caret，但 `TextSnapshot`、Markdown CST、
  history 和 Revision 保持不变。
- parser 不会尝试解释未完成拼音、假名或 dead-key 组合，避免 transient text 造成 Markdown
  结构污染。
- composition glyph 的 canonical source cluster 可以是同一个 replacement range；其 visual
  cluster 仍按临时文本字节边界增长，hit-test 不会伪造不存在的 source bytes。
- 后续 GUI 只需把 native composition selection/caret 接到 transient layout，不需要维护第二
  份 TextKit 文本模型。
