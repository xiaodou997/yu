# ADR 0126：macOS production visual pointer and caret mapping

## 状态

已接受并由 ADR 0129 细化（Phase 3 Track B；TextKit visual mirror 不再是生产 glyph 命中权威）。

## 背景

0125 让 Rust RenderPlan glyph overlay 在产品窗口可见，但覆盖层不接收输入。若继续让
`NSTextView` 直接按 source Markdown 命中，隐藏 delimiter、link destination 和 projection
换行会让鼠标位置与 Rust glyph 位置漂移。已有的 visual mirror self-check 已验证了
`visual UTF-16 ↔ source UTF-16` 的 Revision-bound reverse mapping，可以把这条路径提升到正常
输入流程。

## 决策

- `DocumentTextView` 在布局完成后启用 visual pointer adapter。它从 Rust 取得当前 projected
  UTF-8，创建匹配当前字体和宽度的 disposable `NSTextStorage`/`NSLayoutManager`，用于
  caret/selection 矩形和输入宿主；这个 mirror 不拥有 source、Revision、selection 或 history。
- 单击和拖选把 document-space point 交给 ADR 0129 定义的
  `yu_storage_session_macos_projection_hit_test`，由 Rust CoreText-shaped block layout 返回
  visual boundary，再调用 `yu_storage_session_projection_source_selection` 映射成 canonical
  source range。Revision、viewport 或映射失败时交给 AppKit 原生 source hit-test。
- 当前 source selection 的 focus/caret 通过 `yu_storage_session_projection_caret` 映射成
  visual UTF-16，再用同一 visual mirror 得到 caret rect。`DocumentTextView` 重载
  `drawInsertionPoint` 绘制该 visual caret；TextKit 仍拥有 NSTextInputClient、IME、复制粘贴和
  Accessibility surface。
- source 编辑、composition generation、窗口布局或 visual mirror 宽度变化会丢弃/重建 mirror。
  stale mirror 不得处理 pointer event；surface 或 projection 失败不弹模态错误，继续使用 source
  hit-test 和 source caret。
- 本阶段不把 overlay 变成可交互 view，不把 Swift 的临时 layout 当作最终 shaped Metal layout，
  也不实现最终 selection highlight、visual IME preedit 或上下移动的跨 line geometry。

## 结果

产品窗口现在可以通过同一 CoreText-shaped Rust layout/source mapping 处理鼠标单击、拖选和
caret 定位；canonical Markdown source、selection、composition 和 history 仍只有 Rust 一份。
TextKit source mirror 的输入和 Accessibility 兼容性不变，projection 或 published viewport
不可用时保持可编辑回退。

## 验证

```bash
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-mirror-self-check \
  experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --projection-hit-test-self-check \
  experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-ime-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
