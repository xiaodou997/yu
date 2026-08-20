# ADR 0151：macOS source mirror presentation roles

## 状态

Accepted（Phase 3 Track B/C；完整 visual renderer 迁移的过渡契约）。

## 背景

`DocumentTextView` 必须继续实现 `NSTextInputClient`、Accessibility、复制粘贴和原生输入
回退，但它不应再通过几个独立的布尔值同时承担 Rust surface 的字形、TextKit projected
decoration 和 canonical source 的 native 绘制。独立开关容易在回退竞态中留下旧 caret/selection
绘制，或者在 Rust surface 已隐藏时继续使用错误的 selection attributes。

## 决策

1. source mirror 只有三个 paint role：`sourceFallback`、`projectedTextKitOverlay` 和
   `rustSurface`。每次 role 变更原子地更新 source glyph、外部 decoration 和 selection paint
   attributes；输入、selection、IME 与 Accessibility 状态完全不变。
2. Rust surface/decoration 成对接受时使用 `rustSurface`，TextKit 不贡献任何 source glyph 或
   caret/selection 像素。Rust 帧尚未提交或不匹配时使用 `sourceFallback`，完整恢复 native
   source 绘制；只有 composition 或严格受控的视觉回退才使用 `projectedTextKitOverlay`。
3. production coordinator 通过 role 方法切换 presentation，不再分别写入两个绘制布尔值。
   原有布尔字段只作为 `draw`/`drawInsertionPoint` 的派生内部状态，并保留 self-check 兼容性。
4. 该 role contract 不改变 canonical Markdown、Rust selection、composition generation、history
   或 Metal resource ownership；它只是把“谁负责像素”变成显式边界，为后续移除 TextKit source
   mirror 的生产绘制职责提供安全回退。

## 结果

- Rust active frame 下 TextKit 明确退化为 input/IME/Accessibility host。
- surface 等待、stale、resize、submit 失败和 active composition 都能原子回到 source fallback，
  不会遗留 projected selection attributes。
- TextKit visual mirror 仍存在，但其 projected overlay 角色被限制在回退路径；完整 visual
  renderer 迁移仍待后续逐块验证。

## 验证

```bash
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-mirror-self-check \
  experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-ime-self-check \
  experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-lifecycle-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
