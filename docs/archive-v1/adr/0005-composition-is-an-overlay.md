# ADR 0005：IME composition 是临时 Overlay

- 状态：Accepted
- 日期：2026-08-09

## 决策

IME marked/preedit text 不进入 canonical `TextBuffer`。一次 composition 固定引用开始时的
`Revision` 和待替换 `TextRange`，后续 `setMarkedText` 只更新 `CompositionOverlay`：

```text
CompositionOverlay
├── base_revision
├── replacement_range       source UTF-8 bytes
├── preedit text
├── selection_utf16         native bridge
└── selection_bytes         projection/layout
```

`insertText`/commit 将 overlay 转换为一个 Transaction；cancel 直接丢弃 overlay，不产生
Transaction，也不进入 Undo。

## 并发与冲突

如果 composition 存在期间 canonical document 已进入新 Revision，原 composition commit 会因
stale base revision 被拒绝。第一版协议由平台层取消并重新开始 composition；在明确需要之前，
不自动把 composition range 映射穿过并发编辑。

## 结果

- Markdown parser 不会解释未完成拼音或假名；
- Escape cancel 不需要从 Undo 中恢复正文；
- 每次输入法提交最多产生一次永久 Transaction；
- 平台适配层负责验证 UTF-16 selection，不能把 offset 放在 surrogate pair 中间；
- Projection/Layout 必须能合成 source replacement 与临时 preedit。

