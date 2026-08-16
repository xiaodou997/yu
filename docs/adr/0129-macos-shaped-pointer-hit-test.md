# ADR 0129：macOS CoreText-shaped pointer hit-test

## 状态

已接受（Phase 3 Track B；visual IME preedit 与最终 renderer 迁移仍后置）。

## 背景

生产窗口已经通过 Rust projection 维护 visual selection、caret reveal 和 shaped vertical
movement，但点击/拖选仍把 point 交给临时 `NSTextStorage`/`NSLayoutManager` mirror。该 mirror
适合作为输入、IME、Accessibility 和矩形绘制宿主，却不是 glyph shaping 的权威；中文、emoji、
组合字符、fallback font 和 Markdown hidden delimiter 都可能让 TextKit 近似边界与 Rust visual
projection 不一致。

## 决策

- `yu_storage_session_macos_projection_hit_test` 接收 expected Revision、document-space point、
  CoreText font size/width，并验证已发布的 viewport line-height/default-advance。
- Rust 通过当前 `ViewportLayout` 找到 parser-owned block，再用 caller-owned `CoreTextShaper`
  构建该 block 的 shaped `LayoutSnapshot`，调用 `LayoutSnapshot::hit_test`；source/visual UTF-16
  结果再经完整 lossless projection round-trip。
- 返回的 x/y 与输入 point 都是 document-space，ABI 只暴露 owned scalar；不会跨 FFI 保存
  `LayoutSnapshot`、CoreText 对象、glyph atlas 或第二份 source。
- `DocumentTextView` 的 production pointer adapter 只消费该 endpoint 的 visual offset；Revision、
  viewport 或映射失败时回退 AppKit canonical source hit-test。TextKit visual mirror 继续服务
  caret/selection 矩形、输入、IME、Accessibility 和回退，不再猜测生产 glyph boundary。
- `MacosSurfaceHostCoordinator` 接收 TextKit 扣除 `textContainerOrigin` 后的 content width，并将
  该值与 CoreText metrics、surface layout 和 pointer endpoint 一起发布；surface 的外层 viewport
  宽度不得悄悄成为另一套换行宽度。

## 结果

鼠标点击/拖选和 Rust projection 使用同一字体 shaping、block origin、line wrap 与 hidden syntax
映射，减少“看见的位置”和实际 source selection 不一致的情况。该端点是最终 Metal surface
接管 pointer 之前的稳定协议；透明 Metal surface 仍不接收输入，visual IME preedit 仍沿用既有
Revision + composition generation 过渡协议。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_shaped_projection_hit_test_is_revision_bound
experiments/macos-document-host/build-rust-ffi.sh
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --shaped-projection-hit-test-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
