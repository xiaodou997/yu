# ADR 0173：macOS Embedded SVG 的受限栅格消费路径

状态：accepted

## 背景

`yu-workspace` 已能把 Revision-bound embedded publication 变成
`EmbeddedSvgPrimitive`、`EmbeddedSvgUpload` 和 `RenderCommand::EmbeddedSvg`。在真正的
macOS surface 提交前，SVG markup 仍需要变成 GPU 可采样的纹理；这个步骤不能把 AppKit
对象或第二套绘制管线带入 shared crates。

## 决策

macOS backend 使用 AppKit `NSImage` 解析 SVG，在 Rust 侧先验证：

- markup 非空且不超过 4 MiB；
- 宽高均不超过 4096；
- 输出 RGBA8 不超过 64 MiB。

native bridge 只返回复制后的 RGBA8 bytes。结果通过现有 `MetalUploader::upload_rgba_image`
进入 `MetalImageAtlas`，再复用既有 image quad shader。embedded texture 使用独立的
`image_kind` 命名空间，与普通 `Image` command 即使 fingerprint 相同也不会混淆。

`MetalFrameRenderer::submit_viewport_frame_with_images` 先同步 embedded uploads，再转换
native commands；纹理不存在、AppKit 解码失败或资源尚未 ready 时保留 deterministic fallback
rectangle。缓存按 `(resource, kind)` 替换 generation；由于 shared `RenderPlanBuilder` 只在
首次见到 generation 时携带 markup，当前阶段不主动淘汰 embedded texture，避免滚动离屏后
无法在没有新 upload 的 frame 中恢复纹理。后续引入带 publication rehydrate 的 LRU 后再增加
显式 eviction。

## 后果

这条路径让 macOS 首个真实 consumer 不需要引入 WebView、DOM 或新的 SVG/GPU renderer，并
保持 SVG markup 不进入 Scene。AppKit SVG 支持和栅格化成本属于平台边界；其他平台可以继续
采用自己的 consumer。当前仅保证受限 raster output，不承诺完整 SVG 动画、脚本或外部资源
加载语义。macOS storage FFI 已将受限 Math renderer 设为默认；Mermaid 在 renderer 未实现前
仍返回 unsupported。
