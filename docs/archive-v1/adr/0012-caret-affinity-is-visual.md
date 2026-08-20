# ADR 0012：Caret affinity 是独立的视觉语义

- 状态：Accepted
- 日期：2026-08-09

## 背景

软换行或硬换行处，一个逻辑文本位置可能对应两个 caret 矩形：前一视觉行末尾和下一视觉行
开头。反过来，同一个行末 point 也可能对应多个相邻 source boundary。仅保存 byte offset 无法
描述用户看到的 caret。

`TextAnchor::Affinity` 已用于决定 source edit 发生时 Anchor 黏附 replacement 的哪一侧，它不
描述视觉行，因此不能复用。

## 决策

Yu 为 caret 单独定义：

```text
CaretAffinity
├── Upstream      preceding visual line end
└── Downstream    following visual line start
```

`SourceCaretPosition` 保存 Revision、UTF-8 byte offset 与 `CaretAffinity`；平台边界通过
`CaretPositionMap` 转成带相同 Revision/affinity 的 `NativeCaretPosition`，后者使用 UTF-16 offset。
两种 position 都不能跨 Revision 直接复用。

Phase 1 尚无 Markdown projection，因此使用 identity projection 验证：

```text
SourceCaret(byte, affinity)
          ↓
NativeCaret(UTF-16, affinity)
          ↓
TextKit insertion point
          ↓
local point ↔ screen point ↔ hit test
          ↓
NativeCaret → SourceCaret
```

正式 `yu-projection` 出现后，source caret 与 native/projected caret 之间必须增加 ProjectionMap，
不能假设 UTF-16 offset 可直接映射到 canonical source。

## 换行规范化

macOS TextKit 实测会把硬行末点击规范化为“LF 后 offset + upstream affinity”，而不是把“LF 前
offset”作为另一个独立可点击 stop。因此 Layout/Platform 可以规范化多个 source boundary 到
一个 canonical visual position；hit test 必须返回 canonical position，不能只比较裸 offset。

## 结果

- Source Anchor 黏附与视觉 caret 行归属不会混为一个枚举；
- candidate rect、鼠标点击和键盘移动可以在换行处保持稳定视觉位置；
- 平台 screen coordinate 与 Layout local coordinate 的转换成为显式边界；
- BiDi、隐藏 Markdown delimiter 和 composition overlay 仍需由后续 Projection/Layout 定义。
