# ADR 0135：macOS Metal render plan 使用 document-space viewport 原点

## 状态

已接受（Phase 3 Track C；为后续完整 visual renderer 迁移提供前置坐标契约）。

## 背景

`yu-layout` 和 CoreText block layout 的 `y` 坐标始终属于当前文档的 document space。
macOS Metal native bridge 会把 RenderPlan 中的 command 与 damage 减去
`RenderPlan::viewport().y()`，再提交到 surface-local drawable。此前 macOS render host 和
visual render plan 都把 scene viewport 的 `y` 固定为 `0`，因此滚动后的 frame 没有把当前
document-space scroll origin 传到这条变换边界。

这会让首屏看起来正常，但使滚动后的 glyph、damage 和 surface-local 坐标失去同一坐标原点，
也会让后续隐藏 TextKit glyph、由 Metal 承担生产渲染时更容易出现跳行或残留 damage。

## 决策

- `macos_render_host_config` 使用 `ViewportRect::scroll_y()` 作为 scene viewport 的 `y` 原点。
- `macos_visual_render_plan` 使用调用方 `scroll_y` 作为 scene viewport 的 `y` 原点。
- scene primitive、block geometry、caret 与 damage 仍然保留 document-space 坐标；只有 native
  render boundary 做 document-space → surface-local 的平移。
- viewport 的 width/height 仍然表示当前 surface 的可见 logical size，不改为整篇文档高度。
- 增加 macOS-only regression，直接验证 render-host config 的 scene viewport 与 scroll origin
  一致；完整 Metal 硬件 self-check 继续验证真实 drawable 提交。

## 结果

滚动、glyph command、damage 和后续 visual decoration 可以共享同一 document-space 原点。
本 ADR 不改变 TextKit 的输入、IME、Accessibility 或回退职责，也不在这一阶段关闭 source
mirror 的字形绘制；下一阶段再引入独立 decoration 层后，才会逐步切换生产渲染权威。

## 验证

```bash
cargo test -p yu-storage-ffi macos_render_host_config_tracks_document_scroll_origin
cargo test --workspace
```
