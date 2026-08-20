# ADR 0118：Shaped glyph RenderPlan publication

## 状态

已接受（Phase 3 Track C，macOS 诊断边界；不是生产渲染路径）。

## 背景

`yu-scene` 和 `yu-render` 已经能够把 shaped layout、CPU `GlyphAtlas` 和 damage 组装成
Revision-bound `RenderPlan`。macOS host 需要验证真实 glyph、atlas page 和 render-plan 的所有权
边界，但此时仍不应把 `CTFontRef`、atlas 像素、Metal texture 或完整 production view 接入 Swift。
另一个风险是 CoreText shaper 的数值 `FontFaceId` 必须在布局缓存和后续 rasterizer 之间保持稳定。

## 决策

- `yu-storage-ffi` 新增
  `yu_storage_session_macos_visual_render_plan` count/fill ABI。Rust 使用
  `CoreTextShaper` 取得 shaped block layout，使用 `CoreTextGlyphRasterizer` 生成 owned CPU
  `GlyphAtlas`，再调用 `yu_workspace::assemble_viewport_render_frame` 生成 scene/render plan。
- native 侧只接收 owned scalar：snapshot header、glyph command 的 source range/geometry/page、
  page 尺寸与 fingerprint，以及 damage rectangles。atlas alpha 像素、`GlyphAtlas`、`RenderPlan`、
  CoreText 对象和 GPU handle 仍由 Rust/backend 持有。
- 所有数组使用 count/fill 两阶段 ABI。Rust 先完整构造并验证 plan；任一容量不足、stale Revision、
  无效 viewport 或未知 command 都不得写入部分数组。command、page、damage 必须来自同一 Revision，
  command 顺序保持 painter order。
- `platform/macos/yu-font-macos` 的 `FaceTable` 使用进程内共享的稳定 catalog。新建 shaper
  可以复用布局缓存中的 face id，避免诊断查询通过清空 layout state 来掩盖身份不匹配。
- Swift 仅通过 `--visual-render-plan-self-check` 消费这些 owned scalars，验证 glyph command、
  atlas page fingerprint、damage、有序 source range 和 stale rejection；不切换 production TextKit
  mirror，也不创建 Metal surface。

## 结果

真实 CoreText glyph 已经穿过 `TextBuffer → Markdown/layout → scene → RenderPlan → FFI` 的完整
准备链路，同时保持 canonical Markdown source、layout cache、atlas bytes 和 GPU 资源不跨边界。
以后接入 `yu-render-macos` 时可以复用同一 `RenderPlan` 和 page identity，不需要 Swift 重建 glyph
布局或 Markdown 语义。

当前 bridge 仍是诊断协议：它每次调用建立临时 atlas 并复制轻量 metadata，未承诺长期 atlas cache、
Metal submission 或产品窗口性能。下一步应在 Rust backend 内连接持久 atlas/device，并让真实 native
surface 消费 shared render frame。

## 验证

```bash
cargo test -p yu-font-macos
cargo test -p yu-storage-ffi ffi_macos_visual_render_plan_is_glyph_atlas_bound_and_atomic
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-render-plan-self-check experiments/macos-document-host/Fixtures/block-projection.md
```
