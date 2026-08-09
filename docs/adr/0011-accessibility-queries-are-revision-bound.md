# ADR 0011：Accessibility 文本查询绑定 Revision

- 状态：Accepted
- 日期：2026-08-09

## 决策

平台可访问性适配层必须从一个不可变的编辑状态构造同步查询快照。每个原生 UTF-16 position
和 range 都携带该快照的 `Revision`；使用其他 Revision 的 range 必须失败，不能静默映射到当前
文本。

Rust core 提供：

```text
TextSnapshot + source selection
              │
              ▼
AccessibilityTextSnapshot
├── number_of_characters
├── selected_range
├── text_for_range
├── line_for_position
└── range_for_line
```

`NSRange` 只存在于 macOS 适配边界，其单位是 UTF-16。UTF-16 range 转为 source byte range 时
必须验证边界，拒绝落在 surrogate pair 中间的位置。查询正文时可以跨 Piece Tree chunk 收集所需
子串，但不得为每次查询物化整个 Snapshot。

字符范围与屏幕几何是两类职责：Rust 文本查询负责 source/UTF-16/line 映射；
`AXBoundsForRange`、visible range 和 point hit testing 由拥有当前 Layout 的平台适配层回答。

## Composition

当前协议覆盖 canonical source 快照。正式垂直切片必须在平台发布一次一致的编辑状态：存在
`CompositionOverlay` 时，Accessibility 与 `NSTextInputClient` 都查询同一份带 preedit 的投影视图，
不能让文本、selection 和 caret rect 分别来自不同 Revision。

## 结果

- 同步 AX 查询不会读到后台更新中的半成品状态；
- 原生 UTF-16 range 不能逃逸成无 Revision 的长期位置；
- Piece Tree 保持查询局部性，Accessibility 不强迫文本存储退化为连续 `String`；
- 几何查询等待 Layout/Projection 垂直切片，不在文本核心中伪造坐标。
