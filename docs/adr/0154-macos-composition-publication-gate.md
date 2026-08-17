# ADR 0154：macOS composition publication 与 visual gate 竞态契约

## 状态

Accepted（Phase 3 Track C；composition-aware Rust surface 的发布与可见性门控）。

## 背景

IME preedit 不推进 canonical Revision，但每次 begin/update/cancel 都会推进
composition generation。RenderPlan、Metal surface 和 Rust decoration 因此不能只用 Revision
判断新鲜度；count/fill 调用之间、surface submit 与 decoration 查询之间都可能保留上一代
transient frame。

## 决策

1. RenderPlan/scene-glyph/decoration 的 count/fill header 都携带 composition generation。fill
   capacity 非零时，Rust 先验证调用方 header 与当前 Revision/generation；generation 变化返回
   `YU_STORAGE_STALE_COMPOSITION`，清空 header/written 并禁止部分写入。
2. active composition 的新 count/fill 必须使用 transient block layout 和当前 preedit；update 或
   cancel 后旧 header 永远不能复用，必须重新 count/fill。cancel 即使不改变 Revision，也必须
   重新发布 canonical glyph scene。
3. macOS host 的 source-glyph gate 只接受一个完整 publication：surface snapshot 已提交、
   Revision/generation 与当前 session 相同、surface key 仍是当前 geometry，且 decoration sibling
   持有同 Revision 的 Rust-shaped caret。该 predicate 在 Swift 中保持纯函数并独立自检。
4. gate 失败时 surface 与 decoration 成对隐藏，TextKit 恢复 canonical source 绘制；active
   composition 只有在同一 generation 的 Rust glyph/caret/selection 都可用时才进入 `rustSurface`。

## 结果

- preedit update、cancel、scroll、resize 和 surface generation 变化不会让旧 Rust frame 与新
  caret/selection 混合。
- Rust surface 和 decoration 的正常路径仍无第二套 TextKit visual renderer；TextKit 只承担
  输入、IME、Accessibility 和明确的失败回退。
- 端到端自检同时验证 RenderPlan、decoration generation handoff 与 source-glyph gate，后续
  才适合继续迁移完整 visual selection/render primitive。

## 验证

```bash
cargo test -p yu-storage-ffi --lib \
  tests::ffi_macos_visual_render_plan_is_glyph_atlas_bound_and_atomic -- --exact
cargo test -p yu-storage-ffi --lib \
  tests::ffi_macos_visual_decorations_are_shaped_count_fill_and_generation_bound -- --exact
swift build --package-path experiments/macos-document-host
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --visual-render-state-self-check
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --visual-render-plan-self-check \
  experiments/macos-document-host/Fixtures/block-projection.md
```
