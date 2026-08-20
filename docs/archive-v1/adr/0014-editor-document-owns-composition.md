# ADR 0014：EditorDocument 统一拥有 canonical source 与 composition

- 状态：Accepted
- 日期：2026-08-10

## 背景

CompositionOverlay 的 base Revision 必须与 canonical TextBuffer 属于同一个状态，否则平台
bridge 很容易在 Swift、FFI 和 Rust core 中各自保存一份 source，导致 stale commit、cancel 或
外部编辑的语义分裂。

## 决策

`yu-editor` 新增 `EditorDocument`，统一拥有：

```text
EditorDocument
├── TextBuffer                 canonical source/revision
└── Option<CompositionOverlay> transient preedit
```

永久修改只能通过 `apply_transaction` 或成功的 `commit_composition`。preedit 更新只改变
overlay；commit 产生一个 Transaction；cancel 丢弃 overlay；如果 canonical source 在
composition 期间进入新 Revision，commit 返回 stale error，overlay 保留给平台取消/重启。

`yu-editor-ffi::YuCompositionSession` 只包装 `EditorDocument`，平台层不能直接访问 Rust
字段。source 查询使用 expected Revision；局部 UTF-16 range 通过 TextSnapshot chunk cursor
复制，避免为一次 AX/IME 查询调用 `TextSnapshot::as_str()`。

## 结果

- Rust core、macOS bridge 和未来其他平台共享同一 composition 生命周期；
- stale commit 成为 editor state 的统一错误，而不是 FFI 特殊分支；
- FFI 可以提供 revision-bound 局部查询，并保持 caller-owned output buffer；
- 后续接入 undo、parser、projection 时无需再迁移一份 FFI 私有文档模型。

## 非目标

`EditorDocument` 尚未包含 selection、layout 或窗口生命周期；这些状态将在 editor model
垂直切片中加入。当前 FFI session 仍由单个原生输入 view 在主线程拥有。
