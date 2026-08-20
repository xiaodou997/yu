# ADR 0050：Unicode Word Movement

## 状态

已接受（Phase 1）

## 背景

macOS Option-←/→、Windows/Linux 常见 Control-←/→ 需要跨词移动，但简单按空格或 ASCII 字符
切分会在中文、组合字符、emoji、标点和混合脚本中产生错误 caret。把这类逻辑留在 AppKit
`moveWordLeft:`/`moveWordRight:` 也会绕过 Rust selection revision 契约。

## 决策

- `EditorCommand` 增加 `MoveWordLeft` 与 `MoveWordRight`；`keymap` 将 Option/Control-←/→
  映射到同一命令，macOS Selector `moveWordLeft:`/`moveWordRight:` 也使用同一 FFI command id。
- 边界使用 `unicode-segmentation` 的 UAX word-boundary segments。向左跳过尾随 whitespace，
  返回前一个非 whitespace segment 的起点；向右跳过前导 whitespace，返回下一个 segment 的终点。
  标点、符号和 emoji 是可导航的独立 segment，不会被并入相邻字母词。
- 命令只更新 `EditorSelection`，不创建 Transaction、Revision 或 history entry。非空 selection
  时左移到 ordered start，右移到 ordered end，与 grapheme movement 保持一致。
- 为保持 Piece Tree/Rope 的低物化成本，算法只读取当前 source line 和必要的相邻 line；不调用
  `TextSnapshot::as_str()`。跨行时最多读取目标相邻行，下一次命令继续推进。
- command availability 只根据 selection/source 边界判断，composition active 时返回 unavailable。

## 结果

- macOS Option-←/→ 的 keyDown、`doCommand(by:)` 和未来菜单入口共享同一个 Rust command。
- 中文、emoji、标点、空白和换行均有可重复的 source caret 行为；移动不会污染 Undo 或 source
  synchronization。
- Rust model、FFI key route 和 Swift self-check 覆盖 Unicode segment、跨行和无 Revision 变化。

## 限制

当前实现按 UAX word-boundary segment 工作，不实现系统级语言词典或编辑器自定义 camelCase/subword
规则；Option/Control page movement 和完整菜单 registry 留给后续阶段。上下移动与 preferred-X
的 block-local 契约见 [ADR 0051](0051-vertical-caret-preferred-x.md)。
