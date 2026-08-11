# ADR 0053：Revision-bound Caret Scroll Request

## 状态

已接受（Phase 1）

## 背景

编辑命令只知道 source selection，不知道平台当前 viewport 的 `scroll_y` 和高度。若由
AppKit 根据 UTF-16 selection 自己推导 block、视觉行和滚动目标，平台就会重新维护一套
Markdown/layout 状态，并且在 source Revision 改变后可能应用过期几何。

## 决策

- `EditorDocument::caret_scroll_request` 接收当前 `ViewportRect` 和 caret margin，返回一个
  revision-bound `CaretScrollRequest`。`caret_scroll_request_with_shaper` 为 shaped viewport
  使用相同的 `ShapingProvider`。
- 请求包含 focus source offset、block index、document-space caret geometry、输入的
  `scroll_y`、绝对 `target_scroll_y`、实际 margin 和 `needs_scroll`。caret 已在可见区域时仍
  返回完整几何，但 `needs_scroll=false` 且 target 等于 current。
- Rust 负责 focus block 的 layout、block 前缀高度和 content height；未测量的其他 block 继续
  使用显式 estimate。目标滚动会被限制在 `[0, content_height - viewport_height]`，margin
  会限制到 viewport 高度的一半。
- FFI 查询必须提供 expected Revision。平台只在请求 Revision 仍是当前 Revision 时应用
  `target_scroll_y`，不能从 command 名称或 UTF-16 range 自行猜测滚动目标。
- 该协议只计算请求，不直接修改 viewport。macOS spike 的 `YuNativeViewportAdapter` 负责将
  请求消费为 `NSClipView` bounds，真实 `TextInputView` 的 host attachment 与 native/Rust
  单位换算见 ADR 0055；GPU viewport attachment 和最终 composition overlay 几何仍留在后续
  产品 GUI 阶段。

## 结果

- Up/Down、Shift selection、鼠标和 Accessibility 写回后都可以复用同一 caret reveal 查询。
- Rust/FFI/macOS spike 可以验证跨 block、顶部/底部 reveal、可见 no-op 和 stale Revision，
  不需要先引入完整 GUI。
- 当前 geometry 使用 block-local layout 的 line height；平台字体、overlay 高度和真实
  scroll container 的 inset 仍由后续平台层接入。
