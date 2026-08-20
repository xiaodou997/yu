# ADR 0144：macOS 持久 surface 的 image publication 消费

## 状态

Accepted（Phase 3 Track C）

## 背景

0142/0143 已经能生成 image `RenderCommand`、绘制 fallback，并在独立 ignored Metal 测试中把
`ImagePublication` 上传到 `MetalImageAtlas`。但产品持久 surface 仍调用无图片资源的提交入口，
因此真实窗口即使 ImageIO 解码完成，也只能继续显示 placeholder。

## 决策

1. macOS storage FFI 的 `MacosRenderHostState` 持有 `ImageCache`、`MacosImageDecodeWorker`、
   当前 Revision 的 publication map 和 in-flight 去重集合；编辑线程只轮询 owned result，不等待
   ImageIO。
2. 每次 host frame 从 parser-owned `Projection::images()` 生成 `ImageRequest`。相对 destination
   由 `yu-assets::ImageLocation` 以 Markdown 文档目录解析；reference image 复用同一
   `ReferenceDefinitionIndex`，native 不解析 Markdown。
3. worker result 必须经过 `ImageCache::publish_decoded` 的 Revision 校验；成功 publication 在
   surface submit 前同步到持久 `MetalImageAtlas`。纹理 generation/key 相同的重复同步为 no-op。
4. `MetalViewportHostSession::submit_with_images` 将持久 image atlas 传给
   `MetalFrameRenderer::submit_viewport_frame_with_images`。没有 ready resource 的 image command
   仍由 Metal backend 使用自身 fallback；这条路径不会改变 canonical source、projection 或
   layout geometry。
5. surface snapshot 增加 `uploaded_images` 与 `image_resource_count`，用于 self-check 和以后产品
   telemetry 区分“图片命令存在”“纹理已上传”“本次没有新上传”。

## 结果

- 产品持久 `CAMetalLayer` surface 现在会消费真实 ImageIO publication，而不只是在独立测试中验证。
- 第一次提交可以稳定显示 fallback；worker 完成后下一次同 Revision 提交即可复用同一 frame 并
  升级为 ready texture。
- 图片资源状态仍在平台 host/backend，Rust editor 的 Markdown source、CST、selection、Undo
  和 layout 不会携带 decoded bytes 或 GPU handle。
- 当前实现按文档 image source 请求资源，尚未做 viewport-only scheduling、intrinsic image
  measurement 或 atlas eviction；这些是后续阶段，不伪装成完整图片产品功能。

## 验证

```text
cargo test -p yu-render-macos -p yu-storage-ffi
cargo clippy --workspace --all-targets -- -D warnings
swift run --package-path experiments/macos-document-host \
  YuMacDocumentHost --macos-render-host-surface-self-check \
  experiments/macos-document-host/Fixtures/render-images-plan.md
```

surface self-check 在 fixture 目录临时写入 1×1 PNG，验证首次 fallback、异步 ImageIO、ready
publication、Metal texture upload、重复提交、resize generation 和 stale Revision 回退；临时
文件不会作为仓库资源提交。
