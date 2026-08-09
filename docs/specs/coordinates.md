# 坐标与位置

Yu 不使用同一个 `usize` 表示所有位置。

| 类型 | 用途 | 是否可跨 Revision 保存 |
| --- | --- | --- |
| `ByteOffset` | UTF-8 存储、parser、source range | 否 |
| `Utf16Offset` | AppKit/TSF 等原生桥接 | 否 |
| `LineIndex` | 零基源码逻辑行，按 LF 分隔 | 否 |
| `GraphemeOffset` | 用户感知的字符移动与删除 | 否 |
| `TextAnchor` | selection、异步结果、批注 | 是，需要映射 |
| `SourceCaretPosition` | source caret 与视觉行 affinity | 否 |
| `NativeCaretPosition` | AppKit UTF-16 caret 与视觉行 affinity | 否 |
| `VisualPosition` | 投影后的逻辑位置 | 否 |
| `Point` | 布局坐标 | 否 |

## Anchor affinity

Anchor 位于插入点时必须说明黏附方向：

```text
文本: ab|cd
插入: XY

Before affinity -> ab|XYcd
After affinity  -> abXY|cd
```

Replacement 内部的 Anchor 会折叠到 replacement 的左边或右边，具体由 affinity 决定。

## Caret affinity

`TextAnchor::Affinity` 决定 source edit 后 Anchor 跟随哪一侧；`CaretAffinity` 决定换行边界的
caret 显示在前一视觉行还是后一视觉行。这两个概念禁止复用：

```text
CaretAffinity::Upstream    → preceding visual line end
CaretAffinity::Downstream  → following visual line start
```

macOS TextKit 会把硬行末规范化为 LF 后 offset + upstream affinity。因此 point hit test 返回的是
canonical caret position，不保证恢复一个具有相同几何但非独立可导航的 LF 前 offset。

## Snapshot boundary

所有裸 offset 都隐含所属 Revision。跨线程或跨异步边界传递 offset 时，必须同时携带 Revision，
或者先转换为可映射 Anchor。

## Line boundary

源码逻辑行只按 LF byte 分隔。CRLF 计为一个行界且两个 byte 都保留；孤立 CR 和 Unicode
LS/PS 暂不增加 `LineIndex`。空源码仍有第 0 行，以 LF 结尾的源码包含一个末尾空行。

UTF-16 与 byte 的反向查询必须拒绝 surrogate pair/UTF-8 scalar 中间位置，不能静默取整。

## Native accessibility boundary

AppKit Accessibility 的字符数和 `NSRange` 使用 UTF-16。进入 Rust 后必须转换为携带
`Revision` 的 `AccessibilityTextPosition` 或 `AccessibilityTextRange`；来自旧 Revision 的查询
直接拒绝，不自动映射。

Accessibility range 与屏幕坐标不能混为一种位置：

```text
AX UTF-16 range ──► revision-bound text query ──► source byte range
AX screen point  ──► platform coordinate map   ──► layout point
```

`AXBoundsForRange`、candidate rect 和 caret rect 均为屏幕几何查询，必须使用与所查询文本相同的
Projection/Layout 状态。

`NSTextInputClient.characterIndex(for:)` 和 Accessibility point query 接收 screen coordinate；
mouse event 则先进入 view-local coordinate。平台适配层必须显式转换，不能让一个 `Point` 同时
隐含两种坐标空间。
