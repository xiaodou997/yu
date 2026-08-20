# ADR 0033：shaped glyph placement 到 scene 的增量边界

## 状态

已接受（Phase 1）

## 背景

`yu-layout` 已经能消费 `ShapedText` 并用 glyph advance 决定换行，但如果布局完成后丢弃
face、glyph id 和 glyph offset，scene 层就只能重新解析或重新 shaping，无法建立稳定的
`Layout → Scene → RenderPlan` 垂直切片。另一方面，metrics-only layout 没有真实 glyph id，
不能为了渲染而伪造 atlas key。

## 决策

### `yu-layout::GlyphPlacement`

shaped layout 保留 painter-order 的 `GlyphPlacement`，内容包括：

- `FontFaceId` 与 `GlyphId`；
- source/visual cluster range；
- line index；
- x 坐标和 baseline y 坐标；
- visual run style。

`y` 使用 layout 当前的约定：`VisualLine::y + line_height` 是 baseline，再叠加 shaper 提供的
`y_offset`。`x` 是当前 line pen 加 shaper 的 `x_offset`。placement 不拥有字体、bitmap 或 GPU
资源。

metrics-only layout 的 placement 列表为空。这样 renderer 只能消费明确存在的 shaped glyph，
不会把 cluster metrics 当成 glyph identity。

### `yu-scene::SceneBuilder::append_layout`

scene builder 提供一个原子桥接操作：

1. 校验 scene revision 与 layout revision 相等；
2. 校验 font size 有限且为正；
3. 用 `(face, glyph, font size)` 查询 CPU `GlyphAtlas`；
4. 全部 placement 都解析成功后，按 layout painter order 追加 glyph primitives。

缺 atlas entry、stale revision 或 primitive 预算失败时，scene 不得留下半个 layout。

### `yu-render` fake uploader test

在无窗口、无 GPU 的测试中构造真实 `FontShaper → LayoutSnapshot → GlyphAtlas → Scene →
RenderPlan`，再用 `RenderUploader` 的 fake 实现消费 `AtlasPageUpload`。测试必须验证：

- scene/render plan 保留 layout revision；
- 同一 atlas page 只产生一次 upload；
- 第二次计划构建复用 page fingerprint；
- render command 的 origin 与 layout placement 一致；
- missing atlas 和 revision mismatch 不会发布部分 scene。

## 结果

- layout、scene 和 render 可以在 CI 中完成一条可重复的端到端路径，不需要启动 GUI；
- scene 不需要知道 shaping backend 的具体实现，只消费自有 placement 和 atlas entry；
- 后续接 Metal/wgpu 时只需实现真实 `RenderUploader` 与 command encoding，不必改动 source、
  projection 或 layout 坐标契约；
- glyph atlas 缺失会在提交 render plan 前暴露，而不是在 GPU 绘制时静默显示错误字形。

## 限制

- 当前 baseline 由 `LayoutConfig::line_height` 推导，完整 font ascent/descent baseline 仍待
  native font metrics 接入；
- scene 只包含 glyph 与 solid rect，selection、caret、图片和彩色 glyph 尚未接入；
- append 操作按 layout 全部 glyph 构建，viewport-scoped primitive virtualization 属于后续
  layout/render 阶段。
