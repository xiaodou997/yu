# ADR 0146：intrinsic 图片 reflow 与 Metal atlas 淘汰

## 状态

Accepted（Phase 3 Track C）

## 背景

0145 已把 ready 图片限制在当前 viewport/overscan，并在 scene placement 上使用真实宽高比，
但 block 高度仍只来自文本 line count；图片变高不会推动后续 block、content height 或 max scroll。
同时，持久 `MetalImageAtlas` 会保留所有曾经 ready 的 texture，长文档滚动会令 GPU 资源随访问历史
增长。

## 决策

1. `LayoutSnapshot::block_height()` 返回文本 line-height 总和与所有 image placement bottom 的最大值。
   editor viewport 使用 image intrinsic resolver 只重测当前选择的 block，并把结果写入现有
   `ViewportLayout::HeightIndex`；随后 snapshot 的 block y、content height 和 max scroll 都来自同一
   index。source、selection、composition canonical state 和 layout cache 不携带 decoded bytes。
2. workspace scene 组装使用与 image primitive 相同的 source-backed resolver，因此 HeightIndex、
   layout bounds、hit-test 和 Scene/RenderPlan 的 image bounds 使用同一份 publication 尺寸。
3. `MetalImageAtlas::retain_publications` 在每次 native submit 前接收当前 Revision 的 publication
   集合，只保留这些 fingerprint 对应的 texture/identity；离开集合的资源立即释放，空 atlas 释放
   device association。eviction 发生在 command conversion 前，当前 RenderPlan 只能引用 retained
   resource 或走 fallback。
4. surface snapshot 新增 `image_atlas_eviction_count`，与 CPU `ImageCache` 的 `image_eviction_count`
   分开，便于区分 decoded publication 淘汰和 GPU texture 淘汰。

## 结果

- ready 图片变高会推动后续 block 的 document-space y，并同步更新 content height/max scroll。
- 滚动离开图片后，GPU atlas 不再无限保留历史 texture；回到图片时可从 CPU cache 重新 publication
  并上传，source-backed image command 不变。
- 未 ready 或解码失败仍使用 placeholder/fallback；尺寸 metadata 持久化、退避重试和窗口调度
  由 ADR 0147 继续定义。

## 验证

```text
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
swift run --package-path experiments/macos-document-host \
  YuMacDocumentHost --macos-render-host-surface-self-check \
  experiments/macos-document-host/Fixtures/render-images-plan.md
swift run --package-path experiments/macos-document-host \
  YuMacDocumentHost --visual-render-plan-self-check \
  experiments/macos-document-host/Fixtures/render-images-plan.md
```

surface self-check 验证 ImageIO ready texture、intrinsic content-height publication、滚动离屏后
`image_resource_count == 0` 和 atlas eviction counter 增长；临时 PNG 不进入仓库。
