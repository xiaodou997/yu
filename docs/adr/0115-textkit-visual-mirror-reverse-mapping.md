# ADR 0115：TextKit visual mirror reverse mapping

## 状态

已接受（Phase 3 Track B，过渡镜像诊断边界）。

## 背景

Rust 已能把 canonical source selection/caret 投影为 visual UTF-16，但 TextKit 过渡镜像如果
只会 source→visual，用户在 visual 文本上拖选或点击时仍会迫使 Swift 猜测 hidden Markdown delimiter
和 Unicode 边界。该猜测会在 `**strong**`、链接尾部、emoji surrogate pair 和 Revision 变化时漂移。

## 决策

- storage FFI 新增 `yu_storage_session_projection_source_caret` 与
  `yu_storage_session_projection_source_selection`；输入是 expected Revision 绑定的 visual
  UTF-16 边界，输出是 source UTF-16 和 visual round-trip 边界。
- 非折叠 visual selection 使用 `ProjectionBias::Before`/`After` 的外缘映射，使隐藏 delimiter
  保留在 canonical source selection；collapsed caret 遵循 caller affinity。
- FFI output 在 Revision、affinity、UTF-16 boundary 和 range 校验前清空；不返回 Projection、
  LayoutSnapshot、TextKit 或 AppKit 对象。
- Swift 只在 self-check 或显式 opt-in pointer adapter 中创建临时 `NSTextStorage`/`NSLayoutManager`
  验证/命中 visual UTF-8；成功的 reverse mapping 才会把 source selection 同步到现有 source
  mirror。生产 `DocumentTextView` 默认仍保持 source mirror，visual editing 尚未切换。
- composition metadata 追加 projected visual replacement UTF-16 range。opt-in visual mirror 在
  marked text active 时只接受与 mirror 相同 Revision + generation 的 projected preedit，并从该
  range 实现 `markedRange`/`attributedSubstring`；source mirror 仍是过期或失败时的回退。

## 结果

- visual/source 双向坐标协议已经可以独立测试，native 后续可以把鼠标/拖选结果提交给 Rust
  selection，而不在 Swift 维护第二份 Markdown 语义。
- Rust source Revision 变化后旧 visual mirror 查询会立即失败，避免 late callback 覆盖新 source。
- `DocumentTextView` 已有 opt-in 点击/拖选 adapter、visual composition marked-range adapter 和安全
  source fallback；下一步仍需在真实 visual view 中接入上下移动、滚动 origin 和完整 IME 坐标，再决定
  是否替换 source TextKit mirror。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_projection_visual_mirror_maps_caret_and_selection_back_to_source
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-mirror-self-check experiments/macos-document-host/Fixtures/projection.md
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-ime-self-check experiments/macos-document-host/Fixtures/projection.md
```
