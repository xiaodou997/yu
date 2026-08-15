# ADR 0120：macOS document host 的 persistent render lifecycle

## 状态

已接受（Phase 3 Track C，Rust/FFI 诊断桥；生产窗口仍保留 source TextKit mirror）。

## 背景

`CoreTextViewportFrameBuilder` 已经能够跨调用保留 CoreText shaper、CPU glyph atlas、
RenderPlan fingerprint 和 workspace publication，但 macOS document host 仍只能通过一次性
`yu_storage_session_macos_visual_render_plan` 诊断调用得到临时数组。这样无法验证编辑、滚动和
resize 事件是否共享同一个 Rust-owned frame lifecycle，也无法把 surface generation 与 Revision
一起交给未来的 native GPU view。

## 决策

- `YuStorageSession` 在 macOS 上懒创建一个 `MacosRenderHostState`，内部持有
  `CoreTextViewportFrameBuilder` 和 `MetalViewportHostSession`；它不持有 `EditorDocument`，文档仍
  只归 `DocumentEditorSession` 所有。
- 新的 `yu_storage_session_macos_render_host_frame` 接受 expected Revision、字体/viewport 参数和
  native surface generation。Rust 先验证 Revision 与 viewport config，再同步 host generation，
  通过 builder 发布当前 document，最后接受 publication 并返回一个 owned scalar snapshot。
- snapshot 只包含 frame Revision、host frame serial、surface generation、viewport/content geometry、
  command/upload/damage 数量和 atlas 统计。command 数组、atlas pixels、scene、Metal handles 不跨
  ABI；Swift 不能自行重建 layout 或 page identity。
- 同一字体大小的 scroll/resize 只更新 builder config，保留 atlas 与 publication serial；字体大小
  改变时重建 builder，且 surface generation 仍禁止回退。编辑会先清理 host 的旧 frame，再发布新
  Revision；旧 Revision 的调用在入口直接返回 stale。
- 这个入口目前只用于无窗口 Swift self-check 和后续 view lifecycle 接线，不替换生产 TextKit
  source mirror，也不创建可视化演示模式。

## 结果

现在可以在 Swift↔Rust 边界验证完整的 Rust-owned lifecycle：首次 frame 产生 atlas upload，重复
同 viewport 不重复上传同一 page，scroll/resize 推进 frame serial 与 surface generation，编辑后旧
Revision 被拒绝，新 Revision 可重新发布。后续接入真实 native view 时，只需把这个 snapshot 与已有
Metal surface attach/submit 组合起来，不必再引入第二份文档或 atlas state。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_render_host_reuses_state_across_viewport_events
swift build --package-path experiments/macos-document-host
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --macos-render-host-self-check experiments/macos-document-host/README.md
```

生产 visual view、真实 Metal surface submit、完整 visual primitive 仍属于后续阶段。
