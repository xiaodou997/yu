# ADR 0122：macOS real surface submit self-check

## 状态

已接受（Phase 3 Track C，macOS 诊断/opt-in 边界；生产窗口仍保留 source TextKit mirror）。

## 背景

ADR 0120 建立了 persistent CoreText/atlas/publication host，ADR 0121 又把 retained scene 的
glyph metadata 暴露给 native，但 document host 仍没有从 Swift/AppKit 入口真正走过
`CAMetalLayer nextDrawable → Metal command buffer → retained target blit → present`。已有
`yu-render-macos` ignored Rust probe 能验证 backend，但不能验证 storage FFI 与 macOS shell 的
真实调用边界。

## 决策

- 新增 `yu_storage_session_macos_render_host_surface_submit`。Swift 传入一个当前 AppKit main
  thread 的 `NSView` 指针和 surface 尺寸；Rust 在同步调用内创建 `MetalDevice`、`MetalSurface`、
  `MetalFrameRenderer`、`MetalUploader` 和 `MetalAtlas`，附着 `CAMetalLayer`，然后复用同一
  `YuStorageSession` 的 persistent CoreText publication 与 `MetalViewportHostSession::submit`。
- 提交顺序固定为 `NSView attachment → current Revision frame publication → atlas staging →
  render_plan/drawable submit → consumer commit`。失败时 snapshot 保持默认值，host 的成功
  submission 不推进；stale Revision 在 frame publication gate 处直接拒绝。
- ABI 只返回 Revision、surface generation、frame serial、uploaded pages、command/damage/atlas
  计数和 submitted 标志。native view attachment、Metal device、renderer、atlas、render target、
  command queue 和 GPU handles 不跨边界，也不写入 document/editor state。
- 本诊断桥每次创建 generation 0 的临时 surface，并在调用结束时释放 attachment 与 GPU 资源；
  它不负责持久 resize/generation。后续真实窗口 adapter 必须拥有长期 surface，并把 resize 事件
  转成 `MetalSurface::resize` 与 `MetalViewportHostSession::sync_surface_generation`。
- Swift self-check 创建并短暂显示一个临时 AppKit window 以获得真实 drawable，然后立即关闭；这
  是显式测试命令，不是产品可视化演示模式，也不替换生产 TextKit source mirror。

## 结果

现在可以从 document-host 的 Rust FFI 边界实测真实 `CAMetalLayer` attachment、atlas upload、
drawable acquisition、retained target blit、present/commit 和 stale Revision rejection。生产
窗口仍没有第二套 visual document model，也没有改变 IME、复制粘贴、Accessibility 或 source
TextKit mirror。下一步应把 surface/renderer/atlas 从同步 self-check 提升为由产品窗口拥有的
persistent native view adapter，并覆盖 resize、scroll、drawable unavailable 与 close 生命周期。

## 验证

```bash
cargo test -p yu-storage-ffi
./experiments/macos-document-host/build-rust-ffi.sh
swift build --package-path experiments/macos-document-host
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --macos-render-host-surface-self-check \
  experiments/macos-document-host/Fixtures/block-projection.md
```
