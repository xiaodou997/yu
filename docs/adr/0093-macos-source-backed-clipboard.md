# ADR 0093：macOS source-backed Markdown 剪贴板

## 状态

已接受（2026-08-14）

## 背景

`DocumentTextView` 的字符串只是 TextKit mirror，可能包含 Markdown 投影差异或 transient IME
preedit。直接使用 `NSTextView` 默认剪贴板会把这份临时字符串当成文档内容，破坏 Rust
`DocumentEditorSession` 作为唯一 source of truth 的边界。

## 决策

macOS host 的 copy/cut 从 Rust 当前 selection 读取一次 canonical source，并向同一个
`NSPasteboard` 写入两种类型：

```text
net.daringfireball.markdown  → canonical Markdown source
public.utf8-plain-text       → 同一份 source
```

paste 按以下顺序读取：

```text
Markdown UTI → public.utf8-plain-text → no-op
```

插入仍然只走 `yu_storage_session_insert_text`，因此粘贴不会绕过 Revision、selection、dirty 或
Undo 规则。cut 只有在两个剪贴板 payload 都写入成功后才提交 Rust delete command。

本阶段不生成 `text/html`：没有完整 Markdown semantic exporter 时，把源文本包进 `<pre>` 或按
TextKit 投影拼接都属于错误的富文本语义。HTML 类型将在 Markdown exporter 定义稳定后单独加入，
并保持 Markdown 与 HTML 都由同一个 Rust source range 生成。

## 取舍

这会让 Yu 与支持 Markdown UTI 的编辑器之间保留 Markdown 结构；只支持纯文本的应用仍能通过
`public.utf8-plain-text` 互操作。跨平台 UTI、HTML exporter 和完整 rich clipboard policy 留到
后续阶段，不把平台格式常量泄漏到 `yu-text` 或 `yu-editor`。

## 验证

- `cargo test -p yu-storage-ffi`
- `swift build --package-path experiments/macos-document-host`
- `experiments/macos-document-host/build-app.sh`
- Rust FFI selection copy 测试确认 payload 来自 Revision-bound canonical source

