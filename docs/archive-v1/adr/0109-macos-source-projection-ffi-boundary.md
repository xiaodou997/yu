# ADR 0109：macOS source-backed projection FFI 边界

## 状态

已接受（Phase 3 起点，macOS host diagnostic）。Rust `DocumentEditorSession` 现在可以向
macOS host 提供 revision-bound visual projection 与 source↔visual caret 映射；生产 TextKit
mirror 暂不切换到 visual 文本。

## 背景

`yu-editor`/`yu-projection` 已经能隐藏 emphasis、strong、code-span 和 link delimiter，但
macOS host 目前仍把 canonical Markdown source 全量放入 TextKit。若直接替换为 visual 文本，
AppKit 的 UTF-16 selection、IME replacement range、copy/cut 和 Accessibility range 会立即
与 Rust source 坐标分离。

## 决策

- `DocumentEditorSession` 是唯一 projection owner；`yu-storage-ffi` 新增
  `yu_storage_session_projected_source`，以 expected Revision 做 count/fill 式 visual UTF-8 查询。
- 同一 handle 新增 `yu_storage_session_projection_caret`，返回 source UTF-16、visual UTF-16、
  round-trip source UTF-16 和 affinity。隐藏 delimiter 的 Before/After 语义沿用 Rust
  `CaretAffinity`，Swift 不自行推导。
- projection 返回值是 native layout 的 disposable snapshot，不是 canonical source；source 仍
  通过已有 `copy_source` 查询，任何 stale Revision 都必须被拒绝。
- macOS `--projection-self-check` 使用真实 AppKit host/Swift FFI 验证粗体、强调、链接语法隐藏和
  两个 caret round-trip，但不改变当前生产 TextKit mirror。

## 结果

- projection 可以在不创建第二个 editor/storage handle 的前提下进入 macOS native pipeline。
- 下一步可以先替换一个受控 block/viewport 的 visual mirror，再逐步迁移 selection、IME、clipboard
  和 Accessibility；在映射未完成前不会破坏现有编辑行为。
- 当前 API 是 inline projection 起点，heading/list/fence/table 等 block projection 仍需要按
  parser-owned block ranges 合并，不能由 Swift 再解析 Markdown。

## 验证

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --projection-self-check experiments/macos-document-host/Fixtures/projection.md
cargo test -p yu-storage-ffi ffi_projection_is_source_backed_and_revision_bound
```

## 后续

实现 block-scoped projection snapshot 与 visual selection adapter；只有 source↔visual↔point 三向
映射、composition overlay 和 hit-testing 都有 Revision-bound 测试后，才把 visual mirror 接入
真实编辑窗口。
