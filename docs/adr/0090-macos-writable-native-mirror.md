# ADR 0090：macOS 可写 native mirror 接入统一文档会话

## 状态

已接受（2026-08-14）

## 背景

Phase 2 的只读文档 host 已经验证 AppKit 窗口、文件保存和关闭冲突，但 `NSTextInputClient`
仍停留在独立的 IME spike。直接把 `NSTextView` 设为可编辑会产生第二份 source、selection、dirty
和 history，违反 Markdown source 单一真源约束。

## 决策

macOS 文档 host 只持有一个 `YuStorageSession`，该 handle 内部拥有一个
`DocumentEditorSession`。Swift 的 `DocumentTextView` 只作为可丢弃的 TextKit mirror：

```text
NSTextInputClient / Selector
             │
             ▼
DocumentTextView ── Revision + generation ──► YuStorageSession
             ▲                                  │
             └── owned source / result / selection ─┘
```

ABI 约束如下：

- 普通字符使用 `insert_text(expected_revision, UTF-8)`，永久修改只由 Rust `Transaction` 提交；
- 命令结果携带 `None/Range/Full` source sync，`Range` 使用结果 Revision 查询局部 UTF-16 区间；
- `selection` mutation 携带 expected Revision 和 affinity；
- composition 的 begin 绑定 Revision，update/commit/cancel 同时绑定 Revision 和 generation；
- generation 失配返回 `YU_STORAGE_STALE_COMPOSITION`，不能覆盖新 marked text；
- `unmarkText` 只隐藏 native marked range，不取消 Rust overlay；commit/cancel 才结束 overlay；
- native mirror 不拥有 Markdown parser、dirty 状态、Undo history 或长期 source 副本。

## 取舍

当前 mirror 仍使用 TextKit 负责临时显示和 AppKit 的 `NSTextInputClient` 回调，尚未接入 Markdown
visual projection、最终 GPU renderer、剪贴板格式或 VoiceOver 的完整产品语义。局部 source ABI 已
固定，后续可以替换 TextKit 而不改变 Rust 文档模型。

## 验证

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `swift build --package-path experiments/macos-document-host`
- `experiments/macos-document-host/build-app.sh`
- `codesign --verify --deep --strict --verbose=1 .../YuMacDocumentHost.app`

