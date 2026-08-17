# ADR 0150：macOS visual surface 与 decoration 成对发布

## 状态

Accepted（Phase 3 Track B/C；完整 visual renderer 迁移的第一条生产绘制边界）。

## 背景

持久 Metal surface 和 `MacosVisualDecorationView` 原本可以在不同的回退路径中各自保持可见：
surface 可能仍显示上一帧 Rust glyph，而 decoration 已经切到 TextKit visual mirror。这样在
Revision、scroll 或 active composition 的瞬态窗口里，source mirror、旧 Rust frame 和新的
decoration 可能同时参与绘制，既难诊断，也不能把 TextKit 明确定义为安全回退。

## 决策

1. `rustDecorationFrameAccepted` 记录当前 decoration sibling 是否确实来自 Rust/CoreText-shaped
   查询；TextKit visual mirror 产生的 decoration 永远不能打开 source-glyph gate。
2. 只有以下条件全部满足时，产品窗口才同时显示 Rust surface、Rust decoration，并隐藏 TextKit
   source glyph：surface publication、decoration frame、Revision、composition generation 和
   submit geometry 全部匹配，且没有 active composition。
3. 任一条件失配时，surface 与 source-glyph gate 成对关闭；TextKit source mirror 恢复绘制，
   visual decoration 仍可作为输入/IME 的暂态 fallback，但不能与旧 Rust glyph surface 叠加。
4. `MacosSurfaceHostCoordinator` 的 Rust 资源、canonical source、selection 和 IME 契约不变；
   这一步只改变 product window 的可见性门控，为后续逐块移除 TextKit source 绘制保留回退路径。

## 结果

- Rust glyph 与 Rust-shaped caret/selection 不再跨来源混合；旧 surface 不会在回退时留在 source
  mirror 下方。
- active composition、stale geometry、surface submit 失败会完整回到 TextKit，而不是显示一半
  Rust、一半 native 的不一致帧。
- 当前仍保留 TextKit visual mirror 的输入/IME/Accessibility 和失败回退职责；完整 visual
  renderer 迁移仍是后续工作。

## 验证

```bash
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-render-state-self-check
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-lifecycle-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
