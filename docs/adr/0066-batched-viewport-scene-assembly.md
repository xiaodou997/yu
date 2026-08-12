# ADR 0066：Batched Viewport Scene Assembly

## 状态

已接受（Phase 1 诊断）

## 背景

ADR 0065 只定义了一个 block 的 `ViewportBlockGeometry` 到 scene 的边界。如果调用方
逐个调用 `append_layout_at_block`，前一个 block 可能已经发布到 `SceneBuilder`，后一个
block 才发现 atlas 缺失、source range 不匹配、Revision 过期或 primitive budget 不足。
这会让一次 viewport frame 变成不可重试的部分 scene。

## 决策

- `SceneBuilder::append_viewport` 接收一个已验证的 `ViewportSceneInput` 和按 block 顺序排列的
  block-local `LayoutSnapshot` 引用。
- 该方法先对所有 block 做 Revision、source range、font size、atlas entry、glyph bounds 和
  primitive budget 预检；预检结果全部成功后才一次性把 glyph primitives 追加到 scene。
- layout primitive 的收集与提交分为两个内部步骤。提交阶段复制并更新 `DamageSet`，因此 atlas
  查找、几何检查或 damage 失败都不能留下 primitive 前缀。
- 单 block 的 `append_layout_at_block` 保留，用于局部测试和未来按需重绘；完整 viewport frame
  默认使用批量接口。
- layouts 只借用调用方的 block-local snapshots，scene 仍只拥有 primitive、颜色、viewport、
  damage 和 source `Revision`，不拥有 layout、source text、atlas pixels 或 GPU handle。

## 结果

- 一个 viewport 的多个 block 要么全部进入 retained scene，要么一个都不进入，方便 stale frame
  丢弃和下一 Revision 重试。
- block-local layout 仍只通过 `ViewportBlockGeometry::y` 平移到 document space；scene 不复制
  HeightIndex，也不按 Markdown kind 重新布局。
- 当前 API 仍只生成 glyph primitive；图片、selection/caret overlay 和嵌入 block renderer
  需要在相同的批量预检/提交边界上扩展。
