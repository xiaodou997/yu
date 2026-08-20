# ADR 0052：Shift 垂直移动与 Selection 扩展

## 状态

已接受（Phase 1）

## 背景

普通 Up/Down 只移动 caret；macOS 的 `moveUpAndModifySelection:`/
`moveDownAndModifySelection:` 以及 Shift+上下箭头必须保留 selection anchor，同时把新的
visual caret 写回 Rust。若由 AppKit 自己扩展 selection，跨 Markdown block、preferred-X 和
source affinity 会再次出现两套状态。

## 决策

- `EditorCommand` 增加 `MoveUpExtend` 与 `MoveDownExtend`。`EditorKey::Up/Down` 携带单独
  `SHIFT` modifier 时映射到这些命令；macOS modify-selection Selector 与 FFI wire id 使用同一
  命令。
- 扩展命令以当前 `EditorSelection::anchor()` 为固定端点，只移动 `focus`。focus 可以跨相邻
  Markdown block，目标行仍由 `LayoutSnapshot` hit-test 得到；当 focus 回到 anchor 时自然形成
  collapsed selection。
- 扩展与普通垂直移动共享 `PreferredCaretX`。首次扩展从 focus 的 layout X 初始化，后续
  Shift+上下继续使用该 X；横向/word movement、永久 edit、显式 selection、composition/reset
  清除它。
- 扩展命令只更新 selection，不生成 Transaction、Revision、history 或 source sync；活动
  composition 时仍不可用。

## 结果

- Rust model、FFI key route、macOS `doCommand(by:)` 和 TextKit mirror 都使用同一 selection
  result；长行/短行跨行扩展不会丢失 anchor。
- 普通 Up/Down 仍会在非扩展模式下折叠非空 selection；Shift+上下只改变 focus，允许用户连续
  扩展或收缩同一 selection。
- 当前只覆盖垂直 Shift selection；左右/word/page 的 modify-selection、真实 scroll container
  接入和完整菜单验证留给后续阶段。caret reveal 的 Rust/FFI 查询协议见 ADR 0053。
