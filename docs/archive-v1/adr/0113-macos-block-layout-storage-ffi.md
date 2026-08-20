# ADR 0113：macOS block layout storage FFI

## 状态

已接受（Phase 3 Track B，单 block 诊断边界）。

## 背景

Projection FFI 已能按 parser-owned block 返回 visual UTF-8，但 native host 仍无法验证一个
block 的换行、行高和真实字体 caret。如果 Swift 按 block 文本自行测量，会复制 Markdown range、
Unicode cluster 和 delimiter 映射；如果把 `LayoutSnapshot` 或 CoreText 句柄穿过 FFI，又会破坏
所有权边界。

## 决策

- `yu-storage` 为统一 `DocumentEditorSession` 增加 owned `block_layout` 与
  `block_layout_with_shaper`，layout 只在查询期间构造/复用 editor cache，不成为 storage/native
  state。
- `yu_storage_session_block_layout` 接收显式 metrics 配置，返回 Revision、parser-owned source
  range、block-local visual UTF-16 length、line count、width/height 和 metrics 参数。
- macOS 专用 `yu_storage_session_macos_block_layout` 使用 `CoreTextShaper::from_system_ui` 与
  `yu-layout::LayoutSnapshot::from_projection_with_shaper`；
  `yu_storage_session_macos_block_caret` 用同一 shaped layout 返回 block-local visual caret、
  round-trip source、line index、point 和 CoreText line height。
- 所有结果都是 owned scalar，并清空 output 后再做 Revision/block/参数校验；非 macOS 保留稳定
  symbol 并返回 `YU_STORAGE_CORE_TEXT_UNAVAILABLE`。

## 结果

- macOS host 可以在一个 storage handle 上验证 metrics/CoreText 的 source→visual→caret 闭环，
  不创建并列 editor session。
- block layout 与全局 projection 共用 Rust parser、Unicode、hidden delimiter 和 Revision
  语义；Swift 不解析 Markdown，也不保存 layout/font 对象。
- 当前仍是单 block 诊断边界；可见 viewport 的 block origin/height count/fill、鼠标 point hit-test
  和 TextKit visual mirror 留待下一阶段。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_block_layout_and_caret_are_revision_bound
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --block-layout-self-check experiments/macos-document-host/Fixtures/block-projection.md
```
