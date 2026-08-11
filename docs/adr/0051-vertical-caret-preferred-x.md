# ADR 0051：视觉行上下移动与 Preferred-X

## 状态

已接受（Phase 1）

## 背景

左右移动只能在 source grapheme boundary 上工作；上下移动还必须回答两个额外问题：当前
source caret 属于哪一条视觉行，以及目标行没有同样长度时是否保留用户期望的水平位置。若让
AppKit 自己决定 source offset，隐藏 Markdown delimiter、软/硬换行和后续 Rust projection
会出现两个 selection 真源。

## 决策

- `EditorCommand` 增加 `MoveUp` 与 `MoveDown`，普通 `EditorKey::Up/Down` 和 macOS
  `moveUp:`/`moveDown:` Selector 共用同一 Rust command/FFI id。
- `EditorDocument` 只保留一个 revision-bound 的私有 `PreferredCaretX`。第一次上下移动以
  `LayoutSnapshot::caret_for_source` 返回的 caret X 初始化；后续上下移动保持该 X，不使用上一
  个目标行被 hit-test 吸附后的 X。
- 目标优先使用当前 Markdown block 的 `LayoutSnapshot::lines()`；在首/末可导航视觉行跨越到
  相邻 block 的末/首行。相邻 block 的合成 trailing empty caret line 不作为前一个 block 的目标，
  避免在 block 边界重复停留。目标行通过 `LayoutSnapshot::hit_test(LayoutPoint { x: preferred_x,
  y: target_line.y })` 反向得到 source boundary，再创建新的 `EditorSelection`；命令不创建
  Transaction、Revision 或 history entry。
- 目标行较短时，命中其内容末端使用 `CaretAffinity::Upstream`，命中行首或中间使用
  `CaretAffinity::Downstream`，保证重复上下移动不会跨过换行视觉边界。
- 非空 selection 的 Up/Down 先分别折叠到 ordered start/end。横向/word 移动、永久 edit、
  显式 selection、composition 边界和 reset 都清除 preferred-X；viewport 自动滚动留给后续阶段。
- `command_available` 只做 source block 边界的保守判断；是否存在软换行或宽度换行由执行时的
  `LayoutSnapshot` 决定，执行是最终权威。

## 结果

- source、projection、layout 和 native selection 仍只有一个 Rust canonical selection 流程。
- 长行 → 短行 → 长行的连续下移会回到原始 X；相邻 Markdown block 可以连续上下穿越，合成的
  block 尾部空 caret line 不会造成重复停留。
- metrics-only layout 的垂直位置已经可由 Rust model、FFI route 和 macOS Selector self-check
  验证；真实字体 shaping、viewport scroll-to-caret 和 AppKit/TextKit 同步布局仍未在本 ADR
  中承诺。

## 限制

当前使用 `LayoutConfig::default()` 的确定性 metrics layout。完整 GUI 必须在接入真实字体后为
同一 block 提供一致的 shaping/layout backend，并另外定义 viewport 自动滚动、Shift 扩展 selection、
Page Up/Down 和 RTL 视觉行策略。
