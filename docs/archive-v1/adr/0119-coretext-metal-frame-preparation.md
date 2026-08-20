# ADR 0119：CoreText 到 Metal 的持久 frame preparation

## 状态

已接受（Phase 3 Track C，Rust backend/ignored AppKit probe；不是生产窗口切换）。

## 背景

上一阶段的 `yu_storage_session_macos_visual_render_plan` 已证明 CoreText glyph、CPU atlas、
scene、RenderPlan 和 owned FFI scalar 的契约，但每次诊断调用都会创建临时 atlas 和
`RenderPlanBuilder`。`yu-render-macos` 已有持久 `MetalAtlas`、retained target、damage culling
和 revision-aware host，却没有一个入口把真实 CoreText 文档持续发布给这条 backend 链路。

## 决策

- 新增 macOS-only `CoreTextViewportFrameBuilder`。它持有 `CoreTextShaper`、CPU `GlyphAtlas`、
  `RenderPlanBuilder` 和 `ViewportFramePublisher`，但不持有 `EditorDocument`、窗口、surface、
  `MTLTexture` 或其他 native pointer。
- `publish` 先按当前 viewport 对可见 shaped layout 做按需 glyph rasterization，再调用共享
  `ViewportFramePublisher`；atlas page 与 fingerprint 可以跨 Revision 重用，publisher 的 staged
  plan state 继续保证失败不污染缓存。
- `publish_and_submit` 只组合已有顺序：CoreText preparation → host publication/revision gate →
  `MetalAtlas::sync_plan` → `MetalFrameRenderer::render_plan` → consumer commit。host/session 仍是
  revision、surface generation 和 submission scalar 的唯一所有者。
- ignored AppKit lifecycle probe 改用真实 `CoreTextShaper` 和 `CoreTextViewportFrameBuilder`，不再
  手工注册测试字体/伪造 glyph bitmap；它仍只创建临时 probe window，不属于产品 UI。

## 结果

真实 macOS 字体 shaping 和 rasterization 现在可以沿 Rust-owned publication 进入已有 Metal
surface/retained target，重复发布同一 Revision 不重复上传 atlas page；新增 glyph 会改变 page
fingerprint 并触发一次增量上传。canonical source、layout cache、CPU atlas 和 GPU atlas 仍保持
分层所有权，生产 TextKit/source mirror 不受影响。

下一步才是把这个 builder 接入真正的产品 view lifecycle（编辑、resize、scroll、IME 和
Accessibility 回退），并实现 visual tree 的完整 primitive/命令语义；本 ADR 不宣称已经完成这些
产品集成。

## 验证

```bash
cargo test -p yu-render-macos core_text_builder_reuses_atlas_and_render_upload_state
cargo test -p yu-render-macos macos_device_surface_and_atlas_upload_are_live -- --ignored
```

完整 AppKit attachment/resize lifecycle probe 仍需要可响应的 AppKit 主线程环境；在无交互的自动化
session 中可能保持等待，因此不作为普通回归门槛。
