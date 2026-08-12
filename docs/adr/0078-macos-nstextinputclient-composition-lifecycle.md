# ADR 0078：macOS NSTextInputClient Composition 生命周期

## 状态

已接受（Phase 1 macOS bridge）

## 背景

`yu-editor-ffi` 已经能返回 generation-bound 的 transient projection 和 caret，但
`NSTextInputClient` 的回调顺序并不保证 `setMarkedText`、`unmarkText`、`insertText` 和取消
严格成对出现。尤其是 AppKit 可能先调用 `unmarkText`，随后才交付提交文本；如果 native
bridge 把 `unmarkText` 当成 Rust cancel，或者只保存当前 marked range，就会丢失 canonical
replacement range，导致 commit/cancel 破坏文本镜像。

## 决策

- `TextInputView` 将 Rust `CompositionOverlay` 视为生命周期状态源；native 只保存恢复
  canonical mirror 所需的 owned state，不创建第二份 Markdown projection。
- 一个 composition 同时保存三个彼此不同的坐标概念：
  - `compositionReplacementRange`：Rust overlay 固定的 canonical UTF-16 replacement 起点和范围；
  - `compositionNativeRange`：TextKit 当前显示的 preedit range，随每次 `setMarkedText` 长度变化；
  - `marked`：AppKit 当前是否显示 marked underline 的 presentation range。
- `setMarkedText` 首次调用创建 Rust overlay，后续调用只能 update 同一个 replacement range；每次
  成功 update 后立即读取 `YuCompositionProjection` 与 `YuCompositionCaret`，保存同一
  `Revision + generation` 的 `CompositionState`。
- `unmarkText` 只移除 native marked presentation，不修改 Rust overlay，也不清除 replacement/original
  state。后续 `setMarkedText` 或 `insertText` 仍复用当前 overlay；永久 commit 只发生在
  `insertText`。
- `cancelComposition` 不依赖 `marked` 是否仍存在。它使用保存的 `compositionNativeRange` 替换回
  `compositionOriginal`，然后调用 Rust cancel；如果 AppKit 已经 unmark，仍能完整恢复 canonical
  mirror。
- marked text 活跃时，永久 command、viewport metrics 和 native key route 都让位给
  `NSTextInputClient`；composition commit/cancel 完成后才重新发布 canonical viewport。

## 验证

Swift spike 增加 lifecycle self-check，覆盖：

1. `setMarkedText → unmarkText → setMarkedText` 不创建第二个 Rust overlay；
2. `unmarkText → insertText` 只替换原始 canonical range 一次；
3. `unmarkText → cancel` 恢复原文且不产生永久 Transaction；
4. commit/cancel 查询的 projection 与 caret 必须属于同一 generation。

这个协议仍然是单线程 macOS spike 的平台桥接边界，不承诺最终 GUI、异步输入 callback 或
跨线程调用安全性。
