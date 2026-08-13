# ADR-0080：macOS candidate rect 与结构化 IME 事件日志

## 状态

已接受（2026-08-13）

## 决策

`NSTextInputClient.firstRect(forCharacterRange:actualRange:)` 返回请求范围的首个视觉
fragment 的 screen-space 矩形。对跨多行的 marked preedit，不使用整个 glyph range 的 union，
避免 candidate window 被后续行的高度和位置拉走。Accessibility 的
`accessibilityFrame(for:)` 保留完整 range geometry，两种接口语义不混用。

Spike 同时输出一行一个 JSON 的 `IME_EVENT` 记录，包含：

```text
sequence / context / kind
UTF-16 replacement、selection、marked range
canonical Revision / composition generation
candidate screen rect（如果存在）
```

启动自检使用 `startup-self-check`，窗口获得焦点并进入真实人工输入后切换到 `interactive`。
日志是诊断输出，不写入 Markdown source、Undo 或 Rust document state。

## 后果

- 多行 candidate panel 的真实输入法跟随仍需要在 macOS 上人工验收；自动自检只验证几何协议。
- 结构化日志可以审计日文输入源、dead key、组合重音的真实事件序列，而不会把协议回放当成真实
  输入源验证。
- `firstRect` 和 AX frame 的差异必须在后续原生平台 bridge 设计中保留下来。
