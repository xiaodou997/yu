# ADR 0145：可见图片调度、受限缓存与 intrinsic placement 测量

## 状态

Accepted（Phase 3 Track C）

## 背景

0144 把 ImageIO worker、`ImagePublication` 和 `MetalImageAtlas` 接入了 macOS 持久 surface，
但 host 仍会从整份文档生成 image request。这样会让不可见图片占用 worker、CPU cache 和 GPU
资源，也没有区分“本帧未请求”“解码失败”和“已 ready”。此外，image placement 仍使用固定
placeholder 尺寸，ready 后即使知道真实像素尺寸也不会反映到视觉 bounds。

## 决策

1. CoreText viewport builder 暴露当前 viewport/overscan 的 block index；macOS storage FFI 只从
   这些 block 生成 image request。非可见 block 的 image 仍保留 source-backed projection，但不排队
   ImageIO。
2. `yu-assets::ImageCache` 使用显式正容量和 LRU publication eviction。失败以
   `Revision + ImageKey` 记录 `ImageFailureKind` 与 attempts；同一 Revision 不重复排队，新的
   Revision 才重新获得尝试机会。
3. `yu-layout` 接受当前 Revision 的 ready publication intrinsic width/height，在可见 placement
   上按内容宽度约束和原始宽高比更新 `ImagePlacement.bounds`。source ranges、visual mapping、
   image hit-test 与 canonical Markdown 不变。
4. intrinsic bounds 在下一次 frame 的 layout/scene 组装中生效，未 ready 或失败仍绘制 opaque
   fingerprint 对应的 placeholder fallback。当前阶段不把图片高度变化写回完整 block height
   index，也不宣称已经完成 scroll extent reflow。
5. surface snapshot 增加可见 request、failure 和 eviction 计数，供 self-check、日志和未来
   telemetry 诊断；计数不把 pending 或 stale publication 伪装为 ready texture。

## 结果

- 长文档滚动时只有当前 viewport/overscan 的图片进入 ImageIO worker，降低后台解码和内存压力。
- image cache 不会无限增长，失败可以按 Revision 定位并在新内容版本中重试。
- ready 图片的 scene bounds 会反映真实宽高比，placeholder 与 ready texture 的 hit-test/source
  映射保持一致。
- 图片高度改变导致的完整 block reflow、scroll-height 更新和 GPU atlas texture eviction 仍是
  后续独立阶段，避免把局部 placement measurement 误当成完整图片布局。

## 验证

```text
cargo test -p yu-assets -p yu-layout -p yu-workspace -p yu-storage-ffi
cargo clippy --workspace --all-targets -- -D warnings
swift run --package-path experiments/macos-document-host \
  YuMacDocumentHost --macos-render-host-surface-self-check \
  experiments/macos-document-host/Fixtures/render-images-plan.md
swift run --package-path experiments/macos-document-host \
  YuMacDocumentHost --visual-render-plan-self-check \
  experiments/macos-document-host/Fixtures/render-images-plan.md
```

surface self-check 临时写入 1×1 PNG，验证可见 request、ImageIO publication、ready Metal texture、
intrinsic bounds 和失败/计数边界；临时文件不会作为仓库资源提交。
