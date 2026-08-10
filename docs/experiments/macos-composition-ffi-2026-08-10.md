# macOS CompositionOverlay FFI：2026-08-10

## 目标

把 AppKit `NSTextInputClient` 的 marked/preedit、UTF-16 selection、commit 和 cancel 事件
接入 Rust `CompositionOverlay`，确认不需要把 Rust 文本存储暴露给 Swift，也不会因为每次
`setMarkedText` 而推进 canonical Revision。

## 实现

```text
TextInputView (Swift/AppKit)
        │ NSRange + String
        ▼
RustCompositionBridge (Swift)
        │ C ABI: pointer + length + UTF-16 start/end
        ▼
YuCompositionSession (Rust)
        └ EditorDocument
           ├ TextBuffer
           └ Option<CompositionOverlay>
```

Swift 侧只保存 `OpaquePointer`，不读取 Rust struct 字段。Rust 侧负责：

- 校验 UTF-8 pointer/length；
- 将当前 source 的 UTF-16 replacement range 映射到 UTF-8 byte range；
- 校验 preedit selection 不落在无效边界；
- commit 时通过 `EditorDocument` 应用一个 Transaction；
- cancel 时清除 overlay 并保持 Revision；
- source 校验/读取携带 expected Revision；局部查询逐 chunk 复制 UTF-8 bytes。

## 验证命令

```bash
cargo test -p yu-editor-ffi
experiments/macos-text-input/build-rust-ffi.sh
swift build --package-path experiments/macos-text-input
NSUnbufferedIO=YES \
  experiments/macos-text-input/.build/arm64-apple-macosx/debug/YuMacTextInputSpike
```

最后一个命令会打开实验窗口；启动自检完成后用 Ctrl-C 退出。`build-app.sh` 会在生成临时
`.app` 前自动构建同一 static library。

## 实测输出

```text
Layout self-check lines=4 boundaries=45 affinitySplits=2 softWrapSplits=1
setMarkedText preedit="にほんご" selection={4, 0} replace={47, 0}
setMarkedText preedit="にほんご" selection={4, 0} replace={47, 4}
insertText commit="日本語" replace={47, 4}
setMarkedText preedit="\u{0301}" selection={1, 0} replace={50, 0}
setMarkedText preedit="é" selection={2, 0} replace={50, 1}
insertText commit="é" replace={50, 2}
setMarkedText preedit="にほん" selection={3, 0} replace={51, 0}
cancelComposition range={51, 3}
Unicode composition self-check japanese=日本語 combining=é cancel=restored
AX self-check characters=47 selection={47, 0} firstLine={0, 19} ...
AX runtime probe trusted=true role=Optional(AXTextArea) ...
```

Rust 单元测试另外确认：

```text
ffi_session_maps_utf16_ranges_and_commits_once ... ok
ffi_cancel_does_not_advance_revision ... ok
ffi_local_source_query_requires_revision_and_preserves_utf8_boundaries ... ok
ffi_commit_exposes_stale_revision_and_keeps_overlay ... ok
```

真实 AppKit binary 启动后保持窗口可交互，所有 Swift `precondition`（包括 overlay 文本、
selection、source 和 revision 断言）均通过；AX runtime probe 能读取同一个 focused text area。

## 结论与限制

- 最小 Swift ↔ Rust composition 链路成立；
- FFI session 已包装 `EditorDocument`，不再维护独立的 TextBuffer/overlay shadow state；
- Swift self-check 的 source 断言已改用 expected Revision 的局部 UTF-16 query；
- 日文 preedit、组合重音、commit 和 cancel 都经过 Rust overlay，不是 Swift-only shadow state；
- 当前 ABI 是单线程 spike 协议，尚未承诺跨线程调用或正式插件稳定性；
- `copy_source` 只为兼容诊断保留，正式大文档路径使用 `copy_source_range`；
- 当前没有切换真实日文输入源、dead key 或 VoiceOver 朗读质量的额外声明，已有状态见
  [macOS IME 实验](macos-ime-2026-08-09.md)。
