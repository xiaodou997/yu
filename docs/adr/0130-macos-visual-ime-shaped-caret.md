# ADR 0130：macOS visual IME shaped caret geometry

## 状态

已接受（Phase 3 Track B；preedit glyph 的最终 Metal 绘制仍后置）。

## 背景

现有 macOS composition bridge 已能返回 generation-bound projected UTF-8、marked range 和
visual selection，TextKit 也能作为 `NSTextInputClient` 接收中文/日文等 preedit。但它的
`NSTextStorage`/`NSLayoutManager` 不是 Rust CoreText layout 的权威，活动 marked caret 的
point/line-height 仍没有进入 shaped geometry 协议。

## 决策

- 新增 `yu_storage_session_macos_composition_shaped_caret`，接收 expected Revision、expected
  composition generation、canonical source UTF-16 boundary、CoreText size/width，并验证已发布
  viewport 的 width/line-height/default-advance。
- Rust 使用活动 composition 的 parser-owned block，构建未缓存的
  `block_layout_with_composition_and_shaper`；caret point/line-height 是 block-local，visual
  selection/replacement 是完整 transient projected stream 的 UTF-16 range。
- ABI 只返回 owned scalar，不保存 CoreText、layout 或 preedit source；失败时 output 清零，
  stale Revision/generation 回到 TextKit source/IME fallback。
- Swift visual IME self-check 在 begin/update/cancel 生命周期中验证 shaped geometry 与已有
  projection metadata 同代；生产 TextKit 仍负责输入、`markedRange`、`attributedSubstring` 和
  Accessibility，Metal preedit 绘制留到后续阶段。

## 结果

Yu 已有一条与 pointer/vertical layout 相同的 CoreText shaping 几何边界，活动中文、日文、emoji
preedit 的 caret 和 visual range 不再需要 Swift 复制 glyph advance。由于坐标暂时保持 block-local，
不会强行引入一套 composition-specific 全文 HeightIndex；最终 Metal renderer 可以在接入 shaped
viewport origin 后消费该协议。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_composition_shaped_caret_is_generation_bound
experiments/macos-document-host/build-rust-ffi.sh
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-ime-self-check experiments/macos-document-host/Fixtures/projection.md
```
