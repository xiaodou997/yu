# ADR 0116：Visual viewport scroll coordinate contract

## 状态

已接受（Phase 3 Track B，visual renderer 前的 native 诊断边界）。

## 背景

已有 shaped viewport FFI 会返回 document-space block `y/height`，但 header 没有携带 native 请求的
scroll 输入，也没有 storage-level shaped caret reveal 请求。若 Swift 自己维护 scroll origin 或
从 block 数组重算 content height，visual mirror、NSScrollView 和未来 scene 会出现第二套坐标。

## 决策

- `YuStorageShapedViewportSnapshot` 在原有 ABI 尾部追加 `scroll_y`、`viewport_height` 和
  `max_scroll_y`。block `y/height` 始终是 Rust `ViewportLayout` 的 document-space 值；Swift 只按
  `viewport_y = document_y - content_origin_y - effective_scroll_y` 转换，effective scroll 对
  native 越界输入做显式 clamp。
- storage FFI 新增 `yu_storage_session_macos_shaped_caret_scroll_request`，复用当前
  `DocumentEditorSession` 的 CoreText `ViewportLayout`，返回 Revision-bound document-space caret、
  current/target scroll、margin 和 `needs_scroll`。它不改变 source、selection、composition、history
  或 Revision。
- Swift 的 `NativeVisualViewport` 和 `NativeCaretScrollRequest` 只保存 owned scalar。opt-in
  self-check 将 transform 接到 disposable visual TextKit mirror；生产窗口仍由 source mirror 和
  NSScrollView 控制，直到 visual scene、scroll origin 和 IME 坐标统一。

## 结果

- document-space block/caret geometry、viewport offset 和 scroll reveal 有一个可测试的 Revision
  boundary，native 不需要复制 HeightIndex。
- stale Revision 会在 viewport 与 caret scroll 查询处被拒绝；visual mirror 发现旧 Revision 后
  不再提供 viewport hit-test 坐标。
- 这一步仍不承诺完整 visual renderer、真实 NSScrollView attachment 或 GPU scene；下一步可以
  在该坐标契约上接入 retained scene 和实际 viewport host。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_shaped_caret_scroll_request_is_revision_bound_and_document_space
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-viewport-self-check experiments/macos-document-host/Fixtures/block-projection.md
```
