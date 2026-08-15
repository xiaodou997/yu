# Phase 3：Source Projection & Native Layout

## 目标

在不复制 Markdown source、selection、IME 或 history 的前提下，把 Rust 的 source-backed
projection、layout 和 hit-testing 接入 macOS native editor。阶段初期只建立 Revision-bound
FFI 与诊断边界，待 source↔visual↔point 映射稳定后再替换真实 TextKit mirror，最后才进入
retained scene/GPU 绘制。

## Track A：Projection bridge

- [x] `DocumentEditorSession` 暴露 inline projection 的 owned snapshot
- [x] macOS storage FFI 提供 expected-Revision projection UTF-8 count/fill
- [x] macOS storage FFI 提供 source UTF-16 ↔ visual UTF-16 caret round-trip
- [x] Swift/AppKit projection self-check 覆盖 strong/emphasis/link delimiter 与 Unicode caret
- [x] macOS storage FFI 按 parser-owned block index 暴露 source range/kind/visual lengths 与 UTF-8 snapshot
- [x] Swift block projection self-check 覆盖 heading/task/fenced-code、Unicode 和 stale/out-of-bounds
- [x] macOS storage FFI 暴露 generation-bound composition projection、visual selection 与 marked caret
- [x] Swift composition projection self-check 覆盖 Unicode preedit、update stale generation、cancel/source 保持
- [ ] 完善 heading/list/task/fence/table 的 visual delimiter 语义，并统一 block projection kind
- [ ] visual selection range、hit-testing 和 point↔source mapping
- [ ] stale Revision/generation 在 native projection callbacks 上的全路径回归

## Track B：Native layout

- [ ] block-scoped viewport projection snapshot 与惰性 layout（当前仅完成 projection metadata/UTF-8 诊断）
- [ ] CoreText shaping metrics 与 `yu-layout` line/caret contract 对齐
- [ ] TextKit mirror 仅作为过渡适配器，支持 visual/source 双向映射
- [ ] macOS 鼠标点击、拖选、上下移动和 IME 在 visual projection 下回归

## Track C：Scene and rendering

- [ ] Visual tree → retained scene primitive
- [ ] damage/viewport cache 与 Rust revision 发布协议
- [ ] macOS native GPU surface 只消费 owned scene snapshot
- [ ] heading、emphasis、code、link 的最小真实 visual render

## 约束

1. Markdown source、Revision、selection、composition 和 history 仍只由 Rust
   `DocumentEditorSession` 持有。
2. Swift 不解析 Markdown，不根据 delimiter 自行推导 source range。
3. 任何 visual snapshot、layout、scene 和 glyph cache 都必须携带 Revision；过期结果不得提交。
4. 完整 visual mirror 接入前，现有 native source mirror 必须继续可用，确保 IME、复制粘贴和
   Accessibility 有安全回退路径。
