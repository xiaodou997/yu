# ADR 0131：macOS Visual IME Metal Preedit Glyph Publication

状态：已接受

## 背景

Yu 已经能够把 macOS marked text 转换为 Revision + composition generation-bound 的
CoreText caret geometry，但持久 RenderPlan 仍只使用 canonical block layout。这样同一
Revision 的 preedit 更新不会改变 Metal frame，且 Swift surface submit key 也可能复用旧帧。

## 决策

当活动 `CompositionOverlay` 存在时，block-local replacement 使用单 block fast path；跨 block
replacement 使用 ADR 0133 定义的受影响 block span：

1. `EditorDocument::composition_block_range` 负责确认受影响范围；首 block 承载完整 preedit，后续
   block 清除被替换 source，不构造第二份 canonical 文档。
2. `CoreTextViewportFrameBuilder` 对 span 内每个 block 使用不进入 `LayoutCache` 的
   `block_layout_with_composition_and_shaper`，先为临时 glyph placement 补齐 CPU atlas。
3. `yu_workspace` 的 viewport scene 自动使用同一 transient layout，因而普通
   `RenderPlan`、`MetalAtlas`、retained target 和 Metal glyph pipeline 都不需要第二套 IME ABI。
4. diagnostic glyph/render-plan bridge 使用相同 span 选择规则，避免 publication 与 count/fill
   metadata 的 glyph 数量不一致。
5. Swift `MacosSurfaceHostCoordinator.SubmitKey` 增加 Rust-owned composition generation。preedit
   update、unmark/cancel 即使不推进 canonical Revision，也会触发新一帧提交。

这一步只迁移 shaped preedit glyph 的 publication 和绘制；TextKit 仍然保留为
`NSTextInputClient`、Accessibility 和失败回退宿主，完整 visual renderer 替换和最终输入接管
留到后续阶段。

## 结果

- 日文、emoji 等 marked text 使用与正文一致的 CoreText shaping、glyph atlas 和 Metal pipeline。
- preedit 更新不会污染 source、Revision、history 或 canonical layout cache。
- 同一 Revision 下 generation 变化不会被 native submit cache 忽略。
- cancel 会再次发布 canonical glyph scene；无法建立合法 span 的 composition 不会发布不完整 scene。

## 验证

```text
cargo test -p yu-editor -p yu-workspace -p yu-render-macos -p yu-storage-ffi
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-self-check experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-surface-self-check experiments/macos-document-host/Fixtures/projection.md
```
