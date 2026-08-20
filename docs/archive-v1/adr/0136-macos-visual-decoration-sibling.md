# ADR 0136：macOS 独立 visual decoration sibling

## 状态

已接受（Phase 3 Track B/C；完整 visual renderer 迁移的过渡步骤）。

## 背景

产品窗口已经能把 Rust glyph RenderPlan 提交到持久 Metal surface，但 selection 和 caret 仍由
`DocumentTextView` 的绘制路径负责。直接关闭 TextKit 的全部绘制会同时破坏输入、IME candidate
定位和 Accessibility 回退，因此需要先把“装饰像素”从输入 view 拆出来，再迁移几何 provider。

## 决策

- 新增 `MacosVisualDecorationView`，作为 root view 中的独立 sibling，层级位于 Metal surface
  之上；它只保存当前 Revision 的 selection/caret rectangles 和 composition 状态。
- decoration view 的 `hitTest` 永远返回 `nil`，键盘、鼠标、IME、复制粘贴、滚动和 VoiceOver
  仍由下方 `DocumentTextView`/AppKit 接收。
- 当前阶段 geometry 使用已通过 Revision 校验的 disposable visual TextKit mirror；mirror
  只消费 Rust projected text，不能成为 source、selection、history 或 accessibility 的第二真源。
- decoration frame 有效时，`DocumentTextView` 停止自绘 visual selection/caret；当 mirror stale、
  surface detach、窗口离开或 native submit 失败时，overlay 清空并恢复 TextKit 自绘。
- composition active 时保留 marked-text 输入/语义路径，overlay 只显示当前可验证的 caret，不重新
  构造 preedit source range。

## 结果

selection/caret 像素已经不再绑定在 source TextKit view 的生产绘制路径上，Metal surface 上方有
一个可独立替换的 decoration boundary。输入、IME、Accessibility 和失败回退保持不变；下一阶段
可以只替换 geometry provider 为 Rust shaped layout/count-fill，而不再改动 view 层级或输入协议。

## 非目标

- 不在本阶段隐藏 TextKit source glyph，也不把 Metal surface 变成可交互 view。
- 不在本阶段新增第二份 Markdown、DOM 或 Rich Text model。
- 不由 Swift 根据 Markdown delimiter 推导 selection/caret range。

## 验证

```bash
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-decoration-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
