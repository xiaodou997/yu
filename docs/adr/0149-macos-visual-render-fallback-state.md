# ADR 0149：macOS visual render fallback state machine

## 状态

Accepted（Phase 3 Track B/C；完整 visual renderer 迁移前的安全边界）。

## 背景

产品窗口已经在满足条件时隐藏 TextKit source glyph，并让持久 Rust/Metal surface 与
独立 decoration sibling 负责当前帧的视觉输出。此前这条门控由
`externalVisualDecorationsEnabled`、`sourceGlyphsHidden` 以及若干隐式 Revision 判断共同
表达。编辑、滚动、resize、marked text 和 drawable 生命周期发生在相邻 main-thread 回调时，
仅观察布尔值无法回答“为什么没有隐藏 source glyph”，也无法区分旧 Revision、旧 composition
generation、尚未提交的新几何与真正的 surface submit 失败。

## 决策

1. macOS document host 增加独立的 `VisualRenderStateMachine`。它只控制 source-glyph gate
   的状态，不拥有 source、selection、composition、history 或 Metal 资源。
2. fallback 状态携带稳定原因：`detached`、`missingGeometry`、`waitingForSurface`、
   `staleRevision`、`staleComposition`、`decorationUnavailable`、`compositionActive`、
   `surfaceSubmitFailed`、`visualMirrorUnavailable` 等。满足全部条件时才进入 active 状态。
3. active 状态绑定 `revision`、`compositionGeneration`、`surfaceGeneration` 和 `frameSerial`。
   只有相同 submit frame 同时被 surface coordinator 接受、decoration sibling 持有有效 caret
   frame 时，`DocumentTextView` 才隐藏 source glyph；active composition 也必须使用同一
   generation 的 transient Rust glyph/decoration，不能以 TextKit projected overlay 伪造 active
   frame。
4. 状态机只改变绘制门控和诊断；TextKit 仍然是输入、IME、复制粘贴、Accessibility 以及失败
   回退宿主。状态变化不得复制或改写 canonical Markdown。
5. Swift host 提供无窗口 `--visual-render-state-self-check`，验证重复 fallback 不制造虚假
   transition、active frame identity 可追踪、stale fallback 能恢复。

## 结果

- 每次回退都有可读原因，后续可以把状态接入日志/诊断面，而不猜测两个布尔字段的组合。
- 新 Revision、composition generation、viewport geometry 或 surface generation 未重新发布
  前，旧 frame 不会被错误地标记为 active。
- 真实 visual renderer 迁移仍未完成；这一步为移除 TextKit source 绘制职责提供可回滚的
  状态边界。

## 验证

```bash
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-render-state-self-check
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-decoration-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
