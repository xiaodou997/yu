# ADR 0114：macOS shaped viewport storage FFI

## 状态

已接受（Phase 3 Track B，跨 block viewport 诊断边界）。

## 背景

单 block 的 CoreText layout/caret 已经可以由 unified storage handle 返回，但 native host 仍不能
在不复制 `HeightIndex` 或 block traversal 的情况下取得可见窗口。若 Swift 根据 block 文本自行计算
`y`/`height`，滚动、Unicode wrapping 和后续 source edit 会与 Rust viewport policy 分叉。

## 决策

- `yu-storage` 暴露 `ViewportConfig`、metrics/shaper-backed visible block query 和 config setter；
  viewport estimates、measured heights 与 HeightIndex 继续由 `DocumentEditorSession` 持有。
- `yu_storage_session_set_viewport_config` 让 host 先发布与 CoreText 相同的 width、line height、
  default advance、estimate 和 overscan；它只改变 layout policy，不推进 source Revision。
- `yu_storage_session_macos_shaped_viewport_blocks` 使用同一 session 的
  `visible_blocks_with_shaper`，以 count/fill 返回 Revision、可见 block index range、content height、
  source UTF-16 range、document-space origin/height、measured 和稳定 kind tag。
- output/header 在校验前清空；容量不足只写 header/count，不写部分 block；非 macOS 保留稳定 symbol
  并返回 `YU_STORAGE_CORE_TEXT_UNAVAILABLE`。

## 结果

- macOS host 可以用一个 storage handle 建立 CoreText-shaped viewport metadata，自检 block 顺序、
  source range、document coordinates 和 stale Revision，而不创建第二套 Markdown/layout 状态。
- count/fill 只分配可见窗口大小；后续 `yu-scene` 可以直接消费这些 owned scalar，继续在 Rust 侧
  完成 scene/layout 原子性验证。
- 当前仍不创建完整 GUI、TextKit visual mirror、glyph scene 或 GPU frame；这些保持在后续 Track B/C。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_shaped_viewport_is_count_fill_and_revision_bound
experiments/macos-document-host/.build/arm64-apple-macosx/debug/YuMacDocumentHost \
  --shaped-viewport-self-check experiments/macos-document-host/Fixtures/block-projection.md
```
