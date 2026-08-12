# ADR 0077：Composition Projection FFI

## 状态

已接受（Phase 1 诊断）

## 背景

`CompositionOverlay` 的每次 preedit 更新都必须保持 canonical source 和 Revision 不变，
但 AppKit 的 `NSTextInputClient` 仍需要看到包含 marked text 的 projected 文本、UTF-16
selection 和 caret。只暴露 overlay 原文会迫使 native mirror 自行复制 Markdown projection，
也无法让 native 结果判断“同一 Revision 下的旧 preedit”是否已经过期。

## 决策

- `YuCompositionSession` 在 canonical Revision 之外维护单调的 transient
  `composition_generation`。begin、update、commit、cancel 和 reset 成功后推进 generation；
  失败操作不推进状态。
- `yu_composition_session_projection` 返回 revision-bound 的 `YuCompositionProjection`：
  canonical replacement UTF-16 range、preedit selection、projected visual selection，以及
  projected UTF-8/UTF-16 长度。它只返回 owned scalar，不跨 ABI 暴露 Projection、CST 或 layout
  对象。
- `yu_composition_session_copy_projection` 使用 count/fill ABI 复制 Rust 生成的 projected
  UTF-8。调用方必须同时提供 `expected_revision` 与 `expected_generation`；Revision 正确但
  generation 过期时返回 `YU_FFI_STALE_COMPOSITION`。
- `yu_composition_session_composition_caret` 先验证传入的 canonical UTF-16 boundary，再以
  preedit selection 的 active end 作为 visual caret。这样 preedit 内部的 emoji、假名或组合字符
  不会被错误折叠到 replacement 起点；返回的 round-trip source 仍按 projection bias 回到
  canonical replacement range。
- macOS 的
  `yu_macos_composition_session_block_composition_shaped_caret` 使用未缓存的
  `block_layout_with_composition_and_shaper`，返回 block-local point、line、line height、
  visual selection 和 generation。它不写入 LayoutCache、ViewportLayout、source、history 或
  Revision；非 macOS 保留稳定 C ABI 并返回 unavailable。
- Swift spike 只负责保存两个版本标识、装配 projected TextKit mirror 和消费 owned geometry，
  不复制 Markdown parser，也不把 transient 文本写回 canonical `NSTextStorage`。

## 结果

- native adapter 可以原子地丢弃“同一 source Revision、旧 preedit generation”的结果。
- projected 文本与 visual selection 的 UTF-16 坐标由 Rust 统一计算，AppKit 不需要猜测 hidden
  delimiter、UTF-8 byte 或 surrogate 映射。
- composition layout 的 transient shaping 坐标和 canonical source mapping 只存在 Rust；后续
  GUI 可以直接将 geometry 接到 retained scene，而不会产生第二套 marked-text 文档模型。
- 当前 ABI 是诊断和平台桥接边界，不承诺完整 GUI 或 TextKit 与 Yu layout 的全部排版等价性。
