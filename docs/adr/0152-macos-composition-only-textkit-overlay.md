# ADR 0152：macOS TextKit projected overlay 仅用于 composition

## 状态

Accepted（Phase 3 Track B；移除非 composition TextKit visual 绘制职责）。

## 背景

`projectedTextKitOverlay` 是为了在 Rust 普通 decoration 查询明确返回
`NO_OVERLAY` 时呈现 active marked text 而保留的暂态路径。但如果普通 Revision、stale
geometry 或 surface 尚未提交也走这条路径，TextKit projection 就会在生产窗口中变成第二套
caret/selection renderer，并可能与旧 Rust surface 的坐标混合。

## 决策

1. `DocumentViewController.updateVisualDecorationsFromTextKit` 只允许在
   `bridge.composition.active` 时执行。它消费 generation-bound visual mirror，只绘制暂态
   composition caret/selection，并保持 Rust surface 隐藏。
2. 普通状态下的 Rust decoration count/fill 失败、Revision/geometry 不匹配、surface 尚未提交
   或 CoreText 查询异常全部进入 `sourceFallback`，清除 sibling decoration 并隐藏旧 Rust
   surface；下一次同 Revision 的完整 publication 成功后再回到 `rustSurface`。
3. `DocumentTextView` 的 visual mirror、projection hit-test 和 reverse mapping 仍可用于输入
   与 self-check，但不再作为普通生产 caret/selection 的绘制回退。

## 结果

- 非 composition 场景只有两种完整视觉所有权：Rust surface/decoration，或 canonical TextKit
  source fallback，不再有混合 projected overlay。
- IME preedit 仍可在 Rust surface 暂态不可用时显示 generation-bound caret，保留现有中文、
  日文、dead key 和组合字符回退路径。
- stale/resize/submit failure 的视觉行为更容易诊断，后续可以安全删除非 composition 的
  TextKit visual paint 代码。

## 验证

```bash
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-decoration-self-check \
  experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-ime-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
