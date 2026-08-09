# macOS Caret Round-trip Spike：2026-08-09

## 目标

验证自定义 AppKit `NSView` 可以在多行 shaping 下完成如下闭环：

```text
UTF-16 caret + affinity → glyph point → mouse/text-input hit → UTF-16 caret + affinity
```

同时确认 `NSTextInputClient.characterIndex(for:)` 的 screen-space 坐标要求，以及软换行处是否必须
保留独立 affinity。

## 实现

- TextKit 容器限制为 360 pt，确保测试文本产生真实软换行；
- hit test 使用 `characterIndex(for:in:fractionOfDistanceBetweenInsertionPoints:)`，支持 glyph
  前后半区和 ligature 内部 insertion point；
- mouse event 先转换成 view-local point，`NSTextInputClient`/Accessibility 参数则按协议从
  screen point 转换；
- caret 保存 `NSSelectionAffinity.upstream/downstream`；
- 自检遍历 TextKit 可导航的 canonical grapheme boundary，并验证 local 与 screen 两条路径。

## 实测结果

```text
Layout self-check lines=4 boundaries=45 affinitySplits=2 softWrapSplits=1
AX self-check characters=47 selection={47, 0} firstLine={0, 19}
AX runtime probe trusted=true role=AXTextArea
```

45 个 canonical caret boundary 均完成 point round-trip。2 个位置存在上下游矩形分叉，其中至少
1 个是软换行，不是只有显式 LF 才需要 affinity。

## 发现

第一版遍历所有 grapheme boundary 时，offset 18（首行 LF 前）点击后返回 offset 19 + upstream。
这不是简单的 off-by-one：TextKit 将相同的行末视觉位置规范化到 LF 后的 upstream caret。修正后
测试排除 LF 前的非独立 visual stop，并保留规范化后的 offset 与 affinity。

旧实现还把 `characterIndex(for:)` 当成 view-local API 使用；Apple 协议规定其参数是 screen
coordinate。mouse hit 与原生文本协议现已分成两条入口，并在 screen-space round-trip 中验证。

## 边界

本实验使用 TextKit 作为系统行为 oracle，不代表 Yu 将采用 TextKit 作为最终 Layout。当前闭环是
identity projection；BiDi、Markdown delimiter hide/reveal、composition overlay 和虚拟化 block
需要后续在同一契约上增加测试。
