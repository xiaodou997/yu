# ADR 0127：macOS projected selection highlight and Revision-bound caret reveal

## 状态

已接受（Phase 3 Track B；最终 shaped Metal selection/hit-test 仍后置）。

## 背景

0126 已把生产窗口的点击/拖选和 source→visual caret 映射接入 `DocumentTextView`，但选区仍
依赖 source TextKit 的默认背景，隐藏 Markdown delimiter 会让高亮范围与可见投影不一致；上下
移动或键盘选区移动也还没有消费 Rust 的 shaped caret scroll target。若在这一阶段直接切换最终
Metal 输入/布局，会同时改变输入、IME、Accessibility 和坐标契约，风险过大。

## 决策

- `ProjectionTextKitMirror` 增加只读的 visual selection line-fragment rectangles。它使用与当前
  source mirror 相同的字体/宽度，并且只在 mirror Revision 与 Rust 当前 Revision 相同时有效。
- `DocumentTextView` 将当前 Rust source selection 先通过
  `yu_storage_session_projection_selection` 映射为 visual UTF-16 range，再从 disposable visual
  TextKit layout 取得矩形；生产 adapter 启用时清空 source TextKit 的 selection background，并在
  `draw(_:)` 中绘制 projected highlight。TextKit 仍负责 glyph/text/input，Rust 仍是 selection
  正确性边界。
- 选区通知统一触发 `onCaretChange`。`DocumentViewController` 将该通知异步交给
  `MacosSurfaceHostCoordinator.revealCaretIfNeeded()`，后者先用当前 Revision/字体/宽度发布
  metrics，再查询 `yu_storage_session_macos_shaped_caret_scroll_request`，只在返回仍匹配当前
  Revision 且 `needsScroll` 时把 absolute target 应用到 `NSClipView`，并做最后的 native content
  clamp。
- scroll reveal 失败、stale、无窗口或 geometry 尚未就绪时静默回退；source TextKit、IME、复制
  粘贴和 Accessibility 不被打断。surface submit 与 caret reveal 仍共享同一 Rust session，不能
  在 Swift 复制 HeightIndex 或自行推导 caret y。
- 本阶段不实现最终 shaped Metal hit-test、跨 visual line 的 native vertical command、visual
  IME preedit，也不把透明 surface 改成可交互 view。

## 结果

生产窗口的选区高亮现在来自 Rust projection 的 visual range，隐藏语法不会再把 source delimiter
直接染进可见选区；source selection/caret 变化会请求同一 Revision 的 shaped caret reveal，并在
AppKit clip boundary 内滚动。输入与辅助功能仍使用 canonical source TextKit mirror，因此这是一
个可回退的增量边界，而不是第二套编辑器模型。

## 验证

```bash
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-mirror-self-check \
  experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --shaped-viewport-self-check \
  experiments/macos-document-host/Fixtures/block-projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-lifecycle-self-check \
  experiments/macos-document-host/Fixtures/block-projection.md
```
