# ADR 0141：source-backed image resource publication

## 状态

Accepted（Phase 3 Track C）

## 背景

Markdown image span 已经保留 `source`、alt label、inline destination 和 reference label
范围，但 Scene/RenderPlan 还不能携带图片资源。若在 `EditorDocument` 或 Metal backend 中直接
读取文件，会把解码、缓存和 GPU 生命周期混入 canonical source 或平台窗口状态。

## 决策

1. `yu-projection::ImageSource` 只保存 parser-owned `TextRange`，并随 Projection 的
   strictly-outside edit 一起映射；它不复制 URL、图片像素或 native handle。
2. 新增 `yu-assets::ImageCache` 作为平台解码 worker 的异步轮询边界：`ImageRequest` 绑定
   source `Revision`/range，pending request 按 destination 去重，`DecodedImage` 只接受经过
   尺寸和 RGBA8 长度校验的 owned bytes。
3. decoded entry 可以在新 Revision 重新绑定而不重新解码；旧 Revision 的 publish 在进入
   cache 前拒绝。`ImagePublication` 只携带 Revision、generation、source range、key 和
   owned CPU image，不携带 GPU texture。
4. macOS storage FFI 以 count/fill 返回 `YuStorageVisualImage`：destination/reference 是
   同一 Revision 的 UTF-16 ranges，resource fingerprint 只用于诊断和去重。Swift 不解析
   Markdown，也不从数组位置推导图片来源。
5. 本阶段不创建 ImageIO worker、Metal RGBA texture 或图片 Scene primitive。下一阶段在
   macOS backend 内把已发布的 RGBA bytes 上传为 backend-owned texture，并让 RenderPlan
   通过 opaque resource identity 引用 ready 图片；未 ready 图片必须有可测量的 placeholder/
   fallback，不得阻塞编辑线程。

## 结果

- 图片发现、引用解析、异步解码和 Revision 失效规则现在可以独立测试。
- editor/source/layout 不持有路径字符串、像素或 GPU 对象；平台可以替换解码器和缓存策略。
- `--visual-image-self-check` 验证 inline/reference image 的 UTF-16 ranges、fingerprint 和
  stale Revision rejection，但不会假装图片已经在窗口中绘制。

## 验证

```text
cargo test -p yu-assets -p yu-projection -p yu-storage-ffi
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-image-self-check experiments/macos-document-host/Fixtures/render-images.md
```
