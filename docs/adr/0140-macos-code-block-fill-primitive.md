# ADR 0140：macOS code block fill primitive

## 状态

Accepted（Phase 3 Track C）

## 背景

Rust 的 macOS surface 已经可以显示 shaped glyph，但 product RenderPlan 之前只产生 glyph
primitive。于是 fenced code 的文字虽然由 Metal 绘制，代码块背景仍依赖 TextKit/窗口背景，
不能体现 block-level visual projection，也无法验证 solid pipeline 与 glyph pipeline 的
painter order。

## 决策

1. `yu-scene::SceneBuilder::append_viewport_with_fills` 接受与可见 block 一一对应的
   `Option<Rgba8>` style 数组。Scene 不解释 Markdown kind，只验证 fill geometry，并在每个
   block 的 glyph 前插入对应 `FillRect`。
2. `yu-workspace` 在 editor-to-scene 边界把 `BlockKind::FencedCodeBlock` 映射为稳定的浅色
   code background；其他 block 暂不产生 fill。颜色和 mapping 不进入 parser、source、layout
   cache 或 native host。
3. backend-neutral `RenderPlan` 与现有 Metal solid pipeline 原样消费 `FillRect`，无需新的
   texture 或 GPU ownership。macOS storage render-plan FFI 同时支持 fill/glyph command；旧的
   glyph-only scene bridge 只过滤 solid primitive，不把它伪装成 glyph。
4. fill 与 glyph 必须在同一 Revision、viewport 和 composition generation 中原子发布；任一
   layout、atlas、geometry、容量或 stale 错误都不得发布部分 scene/plan。

## 结果

- fenced code block 的背景现在由 Rust scene/RenderPlan/Metal surface 绘制，TextKit source
  glyph gate 不再依赖窗口背景来掩盖 block-level 缺口。
- solid/glyph painter order、FFI count/fill 和 stale 行为都有可测试的契约。
- 图片、SVG、数学和表格仍留在后续阶段；它们需要独立的资源缓存、命中测试和导出策略，
  不在本 ADR 中偷偷引入。

## 验证

```text
cargo test -p yu-scene -p yu-workspace -p yu-render -p yu-storage-ffi
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-render-plan-self-check experiments/macos-document-host/Fixtures/render-code.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-surface-self-check experiments/macos-document-host/Fixtures/render-surface.md
```
