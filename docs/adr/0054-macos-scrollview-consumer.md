# ADR 0054：macOS `NSScrollView` Caret Request Consumer

## 状态

已接受（Phase 1）

## 背景

ADR 0053 定义了 Rust 计算的 absolute document-space scroll target。macOS 还需要一个平台边界
把它消费为 AppKit clip view 的 bounds，而不能让 `NSScrollView` 反过来成为 source/layout 的
第二个真源。

## 决策

- `YuNativeViewportAdapter` 只持有 `NSScrollView`、当前文档 Revision 和 native content height。
  它不持有 Markdown、selection 或 layout snapshot。
- adapter 只接受 Rust `CaretScrollRequest` 和调用方确认的 current Revision；Revision 不匹配时
  返回 stale 并且不触碰 clip view。
- `target_scroll_y` 是绝对 document-space y。adapter 只按 native content height 与 clip height
  做最后的 `[0, maxScrollY]` clamp，然后设置 `NSClipView.bounds.origin.y` 并调用
  `reflectScrolledClipView`。
- `needs_scroll=false` 或目标已经等于 native 当前 y 时返回 no-op；它不生成新的 Rust 查询或
  source 变化。
- 当前实现位于 macOS IME spike，并用无窗口 `NSScrollView` self-check 覆盖 scrolled、stale 和
  no-op。正式产品 host 接入时复用同一边界，不把 AppKit 对象穿过 Rust FFI。

## 结果

- macOS scroll container 的坐标转换只有一处，Rust 与 AppKit 不会各自推导 caret y。
- stale source/layout 请求会在 native side 被丢弃，避免旧命令在新文档上滚动。
- 真实 document view 的 content height、inset、scrollbar 和动画策略仍由后续产品 GUI 决定。
