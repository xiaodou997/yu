# ADR 0121：Revision-bound retained scene glyph bridge

## 状态

已接受（Phase 3 Track C，macOS 诊断/opt-in 边界；生产窗口仍保留 source TextKit mirror）。

## 背景

ADR 0120 建立了 persistent `CoreTextViewportFrameBuilder` 和
`MetalViewportHostSession`，但 macOS document host 仍只能观察 frame、atlas 和 damage 的计数。
下一步需要确认 Swift/AppKit view 能消费真正来自 retained scene 的 glyph primitive，同时不把
Markdown 解析、layout、atlas 像素或 GPU 对象复制到 native 层。

## 决策

- 新增 `yu_storage_session_macos_visual_scene_glyphs` count/fill ABI。它复用同一次 persistent
  host publication，先发布/验证当前 frame，再把 retained scene 中的 glyph primitive 导出为
  Rust-owned scalar 数组。
- 每个 glyph 只携带 Revision、所属 block index、该 block 的 source UTF-16 range、atlas page/矩形、
  origin、bearing、advance、bounds 和颜色。source range 的语义是 block-backed metadata，允许同一
  block 的多个 glyph 重复该 range；native 不据此重建逐 glyph Markdown 语法。
- header 同时携带 frame Revision、surface generation、frame serial、block range、glyph count 和
  viewport/content geometry。count/fill 容量不足时返回 `BUFFER_TOO_SMALL` 且不写入部分数组；
  stale Revision、回退 surface generation 或无法消费的非 glyph primitive 都整体失败。
- Swift 只缓存当前调用返回的 owned scalar primitive，并在 self-check 中验证 source range、block
  顺序、有限几何、atlas placement 与编辑后的 stale Revision 拒绝。它不持有 `GlyphAtlas`、
  `Scene`、`LayoutSnapshot`、CoreText 对象或 Metal handle。
- 该 ABI 是未来 Metal surface submit 的接线点，但本阶段不改变生产 TextKit mirror，不启动可视化
  演示模式，也不把 glyph primitive 直接绘制到窗口。

## 结果

Rust 到 native 的边界现在可以验证从 source-backed layout 到 retained glyph scene 的完整 owned
metadata 路径，并且继续复用同一 Revision/frame/surface lifecycle。后续真实 Metal view 可以直接
消费这些 primitive 或在 backend 内部使用同一 publication，而无需引入第二套 Markdown/document
model。由于当前 scene builder 只保证 glyph primitive，fill/image/decoration 的跨边界协议仍需
单独定义，不能在本 ADR 中假定为已支持。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_visual_scene_glyphs_are_retained_and_source_backed
cargo clippy -p yu-storage-ffi --all-targets -- -D warnings
swift build --package-path experiments/macos-document-host
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --visual-scene-glyph-self-check experiments/macos-document-host/Fixtures/block-projection.md
```
