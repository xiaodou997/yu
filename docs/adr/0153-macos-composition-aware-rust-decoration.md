# ADR 0153：macOS composition-aware Rust decoration geometry

## 状态

Accepted（Phase 3 Track B/C；active composition 的正常 Rust 绘制路径）。

## 背景

Rust surface 已经可以使用 `EditorDocument` 的 transient composition layout 发布 preedit
glyph，但 `yu_storage_session_macos_visual_decorations` 仍把 active composition 视为
`NO_OVERLAY`，导致 glyph 在 Rust、caret/selection 在 TextKit。这个分裂会让两套布局在日文、
emoji、组合字符和跨 block replacement 上产生不同坐标，也阻止 source-glyph gate 在 IME 输入
期间保持单一视觉所有权。

## 决策

1. visual decoration ABI 继续携带 expected Revision 和 composition generation；不新增第二个
   selection/caret 协议，也不把 preedit 写入 canonical source。
2. composition active 时，Rust 端使用
   `visible_blocks_with_composition_and_shaper` 与
   `block_layout_with_composition_and_shaper` 的未缓存 transient layout。受影响 block span
   的 document-space `y`、line、glyph cluster 和 height 与 RenderPlan/Metal surface 共用同一
   composition generation。
3. caret 使用 preedit UTF-16 selection 的 active end，经 block-local composition projection
   计算；selection rectangle 使用同一 projection 的 visual selection。跨 block replacement
   只在拥有 preedit 的起始 block 发布非空 composition selection，其余 block 仍参与 transient
   height/layout。
4. Swift 成功取得同一 Revision/generation 的 Rust caret 后，active composition 也进入
   `rustSurface` role；TextKit 保留输入、`NSTextInputClient`、Accessibility 和 source mirror，
   但不再贡献 glyph/caret/selection 像素。只有 Rust decoration 查询失败或 surface publication
   失配时，才使用 ADR 0152 定义的 projected TextKit fallback。
5. composition cancel/commit、stale generation、resize、scroll 和 surface detach 仍按现有
   成对 gate 处理；任何失配都清空 Rust decoration、隐藏旧 surface，并回到 canonical source
   fallback。

## 结果

- 正常中文、日文、emoji、dead key 和组合字符 preedit 由同一 Rust layout/render pipeline
  提供 glyph 与 caret/selection，消除 active composition 的双布局绘制。
- preedit 仍是 transient overlay；source、Revision、Markdown CST、history 和 undo contract
  不变。
- Rust geometry 失败仍有明确的 TextKit 回退，不会因为切换正常路径而牺牲 IME 可用性。
- count/fill ABI 在 stale generation、selection rectangles 和 caret geometry 上有 macOS Rust
  单测，Swift decoration self-check 覆盖 active composition 的 role/geometry。

## 验证

```bash
cargo test -p yu-storage-ffi --lib \
  tests::ffi_macos_visual_decorations_are_shaped_count_fill_and_generation_bound -- --exact
swift build --package-path experiments/macos-document-host
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --visual-decoration-self-check \
  experiments/macos-document-host/Fixtures/projection.md
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --visual-ime-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
