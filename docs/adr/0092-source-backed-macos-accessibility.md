# ADR 0092：source-backed macOS Accessibility 快照

## 状态

已接受（2026-08-14）

## 背景

`DocumentTextView` 是 AppKit 的可丢弃 mirror。它的 TextKit 字符串可以包含 IME preedit，也可能在
未来变成 Markdown visual projection，因此不能让系统 Accessibility 默认把这个字符串当作 canonical
document。Rust 已有 `AccessibilityTextSnapshot`，需要把它作为 macOS 壳的唯一文本语义来源。

## 决策

`yu-storage-ffi` 增加三个 Revision-bound 查询：

- `yu_storage_session_accessibility_snapshot`：返回 Revision、UTF-16 字符数、选区、行数和 affinity；
- `yu_storage_session_accessibility_line_range`：返回 LF 逻辑行的 UTF-16 range；
- `yu_storage_session_accessibility_line_for_position`：将 UTF-16 位置解析为逻辑行。

文本内容仍通过已有的 `copy_source_range(expected_revision, ...)` 查询，不把 Rust buffer 指针暴露给
Swift。旧 Revision 的 line/range/position 请求统一返回 `YU_STORAGE_STALE_REVISION`。

macOS `DocumentTextView` 显式覆盖以下 AX 查询并回到上述 ABI：

```text
AX value / character count / selected range / line range
                         │
                         ▼
YuStorageAccessibilitySnapshot(revision)
                         │
                         ▼
copy_source_range(expected_revision)
```

TextKit 仍负责绘制、可见范围和鼠标 hit-test；它不拥有 Accessibility 文本、Revision 或逻辑行模型。
composition preedit 是 transient overlay，AX canonical value 仍读取 Rust source；VoiceOver 实际朗读
质量必须通过 macOS 人工验收记录确认。

## 取舍

本阶段不实现 Markdown visual projection 的语义树、AX geometry 的 Rust layout ABI 或跨平台
Accessibility。line query 直接复用 `AccessibilityTextSnapshot` 的 UTF-8/UTF-16 边界检查，避免 Swift
自行处理 surrogate、CRLF 和 grapheme 边界。

## 验证

- `cargo test -p yu-storage-ffi`
- `swift build --package-path experiments/macos-document-host`
- `docs/experiments/macos-document-host-accessibility-2026-08-14.md` 人工验收清单
