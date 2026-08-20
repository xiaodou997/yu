# ADR 0089：统一 Rust 文档编辑会话

## 状态

已接受（Phase 2，storage/editor session 合并）

## 背景

最小 AppKit host 已经可以通过 `yu-storage-ffi` 打开文件、显示 source snapshot 和处理关闭
提示，但早期 ABI 只提供 storage 操作。若未来再单独持有一个 `yu-editor-ffi` handle，文件
生命周期、编辑 Revision、dirty、IME composition 和 close prompt 就可能被拆成两份状态。
这会违反 Markdown source 单一真源原则。

## 决策

在 `yu-storage` 中新增 `DocumentEditorSession`：

```text
DocumentEditorSession
├── DocumentSession
│   └── EditorDocument（唯一 canonical source）
└── CloseStateMachine
```

它统一提供：

- source snapshot、Revision、saved Revision、BOM、dirty、磁盘状态；
- `EditorCommand`、native key route、command availability 和 selection；
- begin/update/commit/cancel composition；
- save/reload 与 close request/save/discard/cancel。

`yu-storage-ffi` 的 `YuStorageSession` 现在只持有这个统一对象。原有 snapshot/state/save/close
ABI 继续工作，同时新增 command、selection、key route 和 composition 入口。所有 native 输入
操作都通过同一 handle 回到同一个 `EditorDocument`；FFI 仍只传递 owned scalars/buffers，绝不
暴露 Rust buffer、parser 或 overlay 指针。

## 结果

编辑命令和 IME commit 现在共享同一个 source Revision 与 dirty boundary：preedit 不推进
Revision，commit 只产生一个永久 transaction，外部文件冲突会让统一 close session 留在 conflict
prompt，而不会半关闭。未来 Swift `NSTextInputClient` 只需要保存一个 session handle，不必在
storage/editor 两个 ABI 之间同步 source。

当前 AppKit host 仍然只读，这是刻意的。新的可写 ABI 已通过 Rust 行为测试，但在接入 Swift
之前还需要把 `NSTextInputClient` 的 marked range、native mirror source sync 和 selection 写回
绑定到这些 revision-bound 结果；不能直接把 `NSTextView` 改为独立可编辑文本框。

## 验证

```bash
cargo test -p yu-storage --test document_session
cargo test -p yu-storage-ffi
cargo clippy -p yu-storage -p yu-storage-ffi --all-targets -- -D warnings
```
