# ADR 0123：persistent macOS native surface adapter

## 状态

已接受（Phase 3 Track C，macOS 诊断/opt-in 边界；生产窗口仍保留 source TextKit mirror）。

## 背景

ADR 0122 用同步调用验证了 storage FFI 到真实 `CAMetalLayer` drawable 的首帧提交，但每次调用
都会重新创建 `MetalSurface`、renderer 和 GPU atlas。那条路径无法验证重复提交的 atlas 复用，也
无法验证 resize 后 surface generation 与 `MetalViewportHostSession` 的同步。

## 决策

- `MacosRenderHostState` 增加可选的 `MacosPersistentSurfaceState`，由 Rust storage session 持有
  `MetalSurface`、`MetalFrameRenderer`、`MetalUploader`、`MetalAtlas`、当前 AppKit view identity
  和一个显式 owned view attachment。
- 首次 `yu_storage_session_macos_render_host_surface_submit` 创建 generation 0 的 surface 并
  attach 到调用方 view；相同 view 的后续调用复用 renderer、retained target 和 GPU atlas。surface
  logical size 或 scale 变化时调用 `MetalSurface::resize`，随后用该 generation 调用
  `MetalViewportHostSession::sync_surface_generation`，让 renderer 在下一次提交执行 full clear。
- view identity 变化必须先调用 `yu_storage_session_macos_render_host_surface_detach`；当前 submit
  不隐式覆盖另一个 view 的 layer。detach 在 AppKit main thread 显式释放 attachment、surface、
  renderer、atlas 和 target，并且是幂等的。
- 原有 `MetalViewAttachment<'surface>` 保留编译期生命周期保护；backend adapter 使用独立的
  `MetalViewAttachmentOwned`，但 `MacosPersistentSurfaceState` 的 `Drop` 明确先 detach 再释放
  `MetalSurface`，不把 native pointer 或 attachment 露给 shared editor/document state。
- surface submit 仍只返回 owned scalar。重复同一 Revision 应复用 atlas page fingerprint；resize
  只改变 surface generation/target 生命周期，不复制 Markdown source、layout 或 scene。

## 结果

document-host self-check 现在覆盖：首次真实 drawable submit、同 Revision 重复提交、atlas upload
复用、surface resize/generation、编辑后的 stale Revision、新 Revision 提交以及显式 detach。
生产窗口仍未接入该 adapter；下一步可以在产品 NSView 的生命周期中拥有同一 state，并把窗口
resize、scroll、close 事件映射到该协议，而不改变 TextKit source mirror、IME、Accessibility 或
canonical source ownership。

## 验证

```bash
cargo test -p yu-render-macos -p yu-storage-ffi
cargo clippy --workspace --all-targets -- -D warnings
./experiments/macos-document-host/build-rust-ffi.sh
swift build --package-path experiments/macos-document-host
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --macos-render-host-surface-self-check \
  experiments/macos-document-host/Fixtures/block-projection.md
```
