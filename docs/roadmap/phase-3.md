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
- [x] composition projection metadata 暴露 visual replacement range，供 visual IME overlay 绑定
- [x] Swift composition projection self-check 覆盖 Unicode preedit、update stale generation、cancel/source 保持
- [ ] 完善 heading/list/task/fence/table 的 visual delimiter 语义，并统一 block projection kind
- [x] visual selection range、metrics hit-testing 和 point↔source mapping 的 Revision-bound 诊断契约
- [ ] stale Revision/generation 在 native projection callbacks 上的全路径回归

## Track B：Native layout

- [x] parser-owned block-scoped projection snapshot、惰性 layout metadata 与 block-local caret
- [x] macOS CoreText shaping metrics 与 `yu-layout` line/caret contract 对齐
- [x] shaped viewport snapshot、block origin/height 与可见窗口 count/fill
- [x] TextKit 过渡镜像自检支持 visual/source 双向映射（生产 view 尚未切换）
- [x] `DocumentTextView` opt-in visual pointer adapter 与 source-mirror fallback self-check
- [x] `DocumentTextView` opt-in visual IME composition mirror、marked range 与 attributed substring self-check
- [ ] 生产 visual view 的鼠标点击、拖选、上下移动和 IME 回归

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
5. visual IME overlay 只能消费同一 Revision + composition generation 的 Rust projected text 和
   visual replacement range；generation 失效时必须回到 canonical source mirror。
