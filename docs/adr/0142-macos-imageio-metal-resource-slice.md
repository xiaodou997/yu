# ADR 0142：macOS ImageIO 与 Metal 图片资源纵向切片

## 状态

Accepted（Phase 3 Track C）

## 背景

0141 建立了 source-backed `ImageRequest`、异步 pending 队列和 owned `DecodedImage`，但还没有
真实平台解码、GPU texture ownership 或 RenderPlan command。直接把这些对象塞进 editor/source
会重新引入第二份文档状态，并让输入线程等待磁盘或 GPU。

## 决策

1. `yu-assets::ImageLocation` 只解析本地路径：相对 destination 以 Markdown 文档父目录为基准；
   `https://`、`data:` 等远程/内联 scheme 明确拒绝，网络加载留给未来有策略的扩展。
2. `yu-render-macos::MacosImageDecoder` 通过 ImageIO/CoreGraphics 解码文件，并把 CGImage 转成
   owned RGBA8 bytes；Objective-C 对象和 malloc buffer 在桥内释放，不跨入共享模型。
3. `MacosImageDecodeWorker` 只传递 `ImageRequest`、文档路径和 owned result。owner 线程仍须把
   result 交给 `ImageCache::publish_decoded`，因此 Revision 检查不依赖线程取消。
4. `MetalUploader::upload_rgba_image` 与 `MetalImageAtlas` 以 publication key fingerprint 和
   generation 去重，成功上传后才替换旧 texture。GPU handle 只存在 macOS backend。
5. Scene/RenderPlan 新增 `ImagePrimitive`/`RenderCommand::Image`，只携带 opaque resource id、
   bounds 和 fallback color。native command 在 resource 尚未 ready 时降级为 FillRect；ready 时
   由独立 image texture binding 进入 `yu_image_fragment`，编辑线程不等待 ImageIO。
6. 本 ADR 只完成资源级协议和 Metal 消费路径，不自动为 Markdown image 生成 block layout、
   selection/hit-test 或产品 UI placement；这些需要下一份 ADR 明确几何和 source mapping。

## 结果

- 可以在不改变 canonical Markdown source 的情况下，验证真实 ImageIO 解码和 RGBA texture upload
  的 ownership 边界。
- RenderPlan 可以稳定表示“ready image”与“暂未 ready 的 placeholder”，设备重建只需清空
  `MetalImageAtlas`，不影响 parser/layout/editor。
- 当前产品窗口仍不会自动显示 Markdown 图片，这是刻意保留的 placement 边界，而不是遗漏。

## 验证

```text
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

macOS 上还应运行 `yu-render-macos` 的 ImageIO PNG 解码测试，以及现有 ignored Metal surface
probe；两者都不创建产品窗口或改变 native source mirror。
