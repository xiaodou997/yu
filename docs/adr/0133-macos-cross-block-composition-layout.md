# ADR 0133：macOS 跨 block composition 的 transient layout

状态：Accepted

## 背景

macOS `NSTextInputClient` 的 marked-text replacement 不保证局限在一个 Markdown block。原先
`composition_block_index` 只能表达 block-local replacement，因此跨 block preedit 只能留在
native source mirror；这会让 Rust projection、viewport 高度和持久 Metal publication 看不到同一
个 IME 状态。

## 决策

`EditorDocument::composition_block_range` 使用 source replacement range 得到半开 block span，
并只遍历受影响的 block。跨 block 时：

1. 首个 affected block 的 transient projection 使用完整 preedit，并从 replacement 起点覆盖到该
   block 末尾；
2. 后续 affected block 的 transient projection 使用空文本，覆盖其被 replacement 命中的 source；
3. `visible_blocks_with_composition_and_shaper` 在 viewport working state 中按 transient layout
   重新测量受影响 block 的高度，稳定后再生成 viewport snapshot；
4. workspace scene、CoreText viewport builder、visual glyph/render-plan FFI 和 persistent host
   统一消费该 span；canonical source、Markdown CST、Revision、LayoutCache 与 Undo 不变。

`composition_block_index` 保留为 block-local compatibility helper：只有 span 长度为 1 时返回
index，跨 block 时返回 `None`，避免旧调用方误把跨 block replacement 当成单 block。

## 结果

- 日文、emoji、组合字符等 marked text 跨越段落、空行或列表时仍可进入同一 shaped glyph/Metal
  publication；
- viewport content height 和后续 block y 坐标使用 transient working state，不污染 canonical cache；
- source/Revision 不因 preedit 改变，cancel 通过 generation 重新发布 canonical scene；
- 无法建立合法 source span 时仍保留 native source mirror 回退，不发布部分 scene；
- 跨 block 的完整 visual renderer、编辑命中与 selection 语义仍是后续阶段，不在本 ADR 中扩展。

## 验证

```bash
cargo test -p yu-markdown -p yu-editor -p yu-workspace -p yu-storage-ffi
./experiments/macos-document-host/build-rust-ffi.sh
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-self-check experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-surface-self-check experiments/macos-document-host/Fixtures/projection.md
```
