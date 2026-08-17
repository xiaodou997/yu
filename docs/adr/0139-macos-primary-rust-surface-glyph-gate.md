# ADR 0139：macOS primary Rust surface glyph gate

## 状态

Accepted（Phase 3 Track C）

## 背景

Yu 的 macOS 产品窗口已经可以把 Rust/CoreText shaped glyph publication 提交到持久
Metal surface，并在相同 Revision 上发布 selection/caret decoration。可是
`DocumentTextView` 仍然是 NSTextInputClient、IME、复制粘贴和 Accessibility 的 native
owner。直接移除 TextKit source mirror 的绘制职责会让 surface 失败、编辑竞态或 marked
text 更新变成不可恢复的空白状态。

## 决策

增加一个仅控制绘制的 `DocumentTextView.sourceGlyphsHidden` 门控：

1. 只有 Metal surface 已经成功提交当前 Rust `Revision`、composition generation 和完整
   submit geometry，且 `MacosVisualDecorationView` 持有同一 Revision 的有效 Rust-shaped
   frame 时，才隐藏 TextKit source glyph 与 insertion point 绘制。
2. 当前 geometry 包含字体大小、内容宽度、scroll origin、viewport 尺寸、surface 尺寸和
   backing scale。滚动、resize、DPI、字体或内容变化产生的新 key 尚未提交时，旧 surface
   不足以触发隐藏。
3. 编辑、composition generation 失配、stale publication、surface detach、native submit 失败
   或 decoration 失效都会立即恢复 TextKit 绘制；active composition 在 Rust transient glyph
   和 decoration 同一 generation 成功发布时可以继续使用 Rust surface。TextKit 的 string、
   selection、IME、clipboard 和 Accessibility 状态不被复制或替换。
4. Rust surface 和 decoration sibling 仍然 `hitTest == nil`；输入、命中后的 source
   selection、VoiceOver 语义和所有 canonical state 继续由 Rust session + TextKit native
   bridge 管理。

这一步是“主视觉层”门控，不是完整 visual renderer 迁移。它只验证 Rust surface 在数据
新鲜时可以承担字形绘制，同时保留可观测、可恢复的 TextKit fallback。

## 结果

- 正常稳定帧不再重复绘制 source TextKit glyph，Rust Metal surface 成为产品窗口的主字形
  视觉层。
- 旧 frame 不会与新 scroll/geometry 下的 decoration 混用。
- IME marked text、编辑瞬间和 surface 生命周期仍有原生回退，避免输入或 VoiceOver 回归。
- 后续可以继续把 block、image、math 等 visual primitive 迁移到 Rust scene，而无需改变
  canonical source 或 TextKit input contract。

## 验证

```text
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-decoration-self-check experiments/macos-document-host/Fixtures/projection.md
```
