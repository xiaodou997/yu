# ADR 0138：macOS visual pointer selection 保留 anchor/focus

## 状态

已接受（Phase 3 Track B；为完整 visual renderer 迁移提供稳定 selection 语义）。

## 背景

Rust `EditorSelection` 同时保存 anchor、focus 和 caret affinity。此前 macOS visual pointer
adapter 把 Rust reverse mapping 得到的 ordered source range 重新交给普通 `set_selection`，因此
向后拖选在视觉上正确，但下一次 Shift-click 或继续拖动时会丢失原始 anchor，导致选择方向和
扩展行为不稳定。

## 决策

- 新增 `YuStorageSelectionEndpoints` 与
  `yu_storage_session_selection_endpoints`，只返回当前 revision 的 anchor/focus UTF-16 endpoints
  和 focus affinity；旧的 ordered `YuStorageSelection` ABI 保持不变。
- 新增 `yu_storage_session_set_selection_endpoints`，验证 expected Revision、UTF-16 scalar
  boundary 和 affinity 后构造同一个 Rust `EditorSelection`。
- visual pointer adapter 继续使用 Rust/CoreText shaped hit-test 和 reverse projection；拖选时
  ordered visual range 只用于求 source endpoints，anchor/focus 方向由 visual anchor 与当前命中
  boundary 的顺序决定，再通过 endpoint ABI 写回 Rust。
- AppKit `NSTextView` 仍只收到 ordered range 以维持输入、IME 和 Accessibility；方向真源保留在
  Rust，下一次 visual drag/Shift-click 从 endpoint ABI 读取 anchor。

## 结果

普通点击、正向拖选、反向拖选和继续扩展选择共享同一 Rust selection 语义，不再因为 native
ordered range 回写而丢失方向。旧 clipboard、Accessibility、TextKit fallback 和 source range
调用者无需迁移。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_selection_endpoints_preserve_visual_drag_direction
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --shaped-projection-hit-test-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
