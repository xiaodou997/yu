# 坐标与位置

Yu 不使用同一个 `usize` 表示所有位置。

## 源码坐标

全部定义在 `yu-core`（`position.rs`）。

| 类型 | 用途 | 是否可跨 Revision 保存 |
| --- | --- | --- |
| `ByteOffset` | UTF-8 存储、parser、source range | 否 |
| `Utf16Offset` | AppKit/TSF 等原生桥接 | 否 |
| `LineIndex` | 零基源码逻辑行，按 LF 分隔 | 否 |
| `TextAnchor` | selection、异步结果、批注 | 是，需要映射 |
| `SourceCaretPosition` | source caret 与视觉行 affinity | 否 |
| `NativeCaretPosition` | AppKit UTF-16 caret 与视觉行 affinity | 否 |
| `VisualOffset` | projection 后的 UTF-8 visual byte offset | 否 |

## 视觉坐标

一套实现，空间进类型，全部定义在 `yu-core::geometry`：
`Point<S>` / `Size<S>` / `Rect<S>` / `Scale<From, To>`。

| 空间 | 原点与单位 | 谁用 |
| --- | --- | --- |
| `Block` | 该 block 左上角，逻辑像素。额外约束 `x >= 0 && y >= 0 && height > 0` | `yu-layout`（别名 `LayoutPoint` / `LayoutRect`） |
| `Document` | 文档内容左上角，逻辑像素，**不含**滚动位移 | `yu-scene` / `yu-render`（别名 `Point` / `Rect`） |
| `Device` | drawable 表面左上角，物理像素 | 平台后端：scissor、栅格化目标 |

跨空间只有两条通道，都必须显式写出来：`Rect::translate_into`（平移原点）与
`Rect::scale` / `Rect::unscale`（换单位）。混用是编译错误，不是运行时检查。
反方向换算用除法而不是乘以倒数——一个 ULP 决定字形落在哪个物理像素上。

见不变量 E6 与 `tools/check-geometry.py`。

> **不属于视觉坐标的两个整数量**：`yu-font::AtlasRect` 是 atlas 页内的纹理
> 坐标，`yu-layout::ImageIntrinsicSize` 是解码后图片自身的像素尺寸。两者都不
> 落在上面任何一个空间里，也都不是 `f32`。

## 视口

`ViewportSpan(scroll_y, height)` 是视口在文档 y 轴上占的区间，不是矩形——
它没有 x 也没有宽度，视口的水平范围由布局的 `max_width` 决定。

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

## Viewport reveal

Caret reveal 查询使用 `ViewportSpan(scroll_y, height)` 作为平台输入，Rust 返回绑定当前
Revision 的 `CaretScrollRequest`。其中 caret 的 `y` 是 document-space 行顶，`target_scroll_y`
也是 document-space 的绝对滚动位置；平台不得把它当成 view-local delta，也不得在请求返回后
重新按 UTF-16 selection 推导 block 高度。平台应用前必须确认请求 Revision 仍然是当前 source
Revision；`needs_scroll=false` 表示 caret 已在 margin 内，target 与 current 相同。

macOS `YuNativeViewportAdapter` 是唯一的 native consumer：它接收已通过 Revision 校验的绝对
target，将其转换为 `NSClipView.bounds.origin.y`，并按 native content/clip height 做最后 clamp。
`NSScrollView` 的对象和坐标不能穿过 Rust FFI。

`AXBoundsForRange`、candidate rect 和 caret rect 均为屏幕几何查询，必须使用与所查询文本相同的
Projection/Layout 状态。

`NSTextInputClient.characterIndex(for:)` 和 Accessibility point query 接收 screen coordinate；
mouse event 则先进入 view-local coordinate。平台适配层必须显式转换，不能让一个 `Point` 同时
隐含两种坐标空间。

> 这一条在 Rust 侧已经由类型保证：`Point<S>` 的空间是类型参数，一个值不可能
> 同时属于两个空间。screen ↔ view-local 这一段仍在 AppKit 侧，仍需人工遵守。

原生 selection 写回使用反向路径，但仍保留 Revision 边界：

```text
mouse/AX range ──► UTF-16 + expected Revision
                         │
                         ▼
                EditorSelection(source bytes)
                         │
                         ▼
                 EditorDocument selection
```

写回不是文本编辑，不推进 source Revision；过期 Revision、越界、surrogate 中间位置或未知
affinity 直接拒绝，平台层必须重新查询当前 selection。`NSSelectionAffinity` 与
`CaretAffinity` 在 ABI 中使用显式 upstream/downstream 映射，不能依赖枚举的原始整数值。
