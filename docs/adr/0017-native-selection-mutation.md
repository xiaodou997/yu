# ADR 0017：原生选区写回使用 revision-bound UTF-16 FFI

- 状态：Accepted
- 日期：2026-08-10

## 背景

macOS 的命中测试和 Accessibility API 会在 AppKit 侧产生新的 `NSRange`。如果这些范围只
停留在 `NSView`，`EditorDocument` 的 canonical selection 就会再次与平台状态分叉；如果
直接接受裸 UTF-16 range，又可能把旧 Revision 或 surrogate pair 中间位置写入 Rust。

## 决策

`yu-editor-ffi` 增加：

```c
int32_t yu_composition_session_set_selection(
    YuCompositionSession *session,
    uint64_t expected_revision,
    uint64_t start_utf16,
    uint64_t end_utf16);
```

Rust 侧按以下顺序处理：

```text
expected Revision
        │
        ├── stale → reject, selection unchanged
        │
        ▼
UTF-16 range
        │
        ├── out of bounds / surrogate split → reject
        │
        ▼
source byte range + CaretAffinity::Downstream
        │
        ▼
EditorDocument::set_selection
```

该函数只改变 `EditorDocument` 中的 selection，不推进 source Revision，也不创建
Transaction。成功返回后，后续 `yu_composition_session_selection` 会返回新的
revision-bound UTF-16 range。

macOS bridge 在 mouse hit-test 或 Accessibility selection 改变前先取消活动 composition
overlay，然后以当前 Rust Revision 调用该函数；键盘左右移动和 self-check 也走同一入口。Rust
仍是 canonical selection 的唯一拥有者，AppKit `selection` 只是当前 View 的原生坐标投影。

## 结果

- mouse、Accessibility、键盘移动与 IME commit 共享同一个 Rust selection；
- stale native range 不会静默套用到新文本；
- UTF-16 surrogate pair 中间位置会返回明确错误，且保留旧 selection；
- 该 ABI 不暴露 `EditorSelection`、`TextSnapshot` 或 Rust-owned memory；
- 选区写回和 source 查询都遵守同一套 Revision/UTF-16 边界协议。

## 限制

当前 macOS spike 在主线程内执行“读取 Revision → 写回 selection”；ABI 尚未承诺跨线程并发
调用。正式 editor 需要把平台事件、布局发布和 selection mutation 放入统一的 UI state
发布协议，并继续覆盖 VoiceOver 与真实输入源。
