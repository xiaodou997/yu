# ADR 0108：macOS native Undo/Redo 显式路由到 Rust history

## 状态

已接受（Phase 2，macOS host）。`⌘Z`、`⇧⌘Z`、编辑菜单与无窗口 self-check 均使用同一条
Rust history command 路径；TextKit 不拥有第二套 undo 状态。

## 背景

`yu-editor` 已经实现了按输入分组的 Undo/Redo、逆 Transaction 和 redo 清理规则，但
`DocumentTextView` 将 `allowsUndo` 设为 `false`，避免 TextKit mirror 建立第二份 history。
仅依赖 `doCommand(by:)` 接收 `undo:`/`redo:` selector 时，AppKit 的 Command-Z key equivalent
不保证会进入该回调，导致用户输入成功而 `⌘Z` 无效。

## 决策

- Rust `EditorDocument`/`DocumentEditorSession` 是唯一 Undo/Redo owner；不重新启用 TextKit
  undo manager。
- `DocumentTextView.performKeyEquivalent(with:)` 显式识别无 Option/Control 的 Command-Z：
  无 Shift 路由 `Undo`，带 Shift 路由 `Redo`。
- 编辑菜单显式提供“撤销”和“重做”，其 action 与快捷键都调用 `DocumentTextView` 的
  `performUndo`/`performRedo`，而不是 `NSTextView.undoManager`。
- 菜单 validation 从 Revision-bound Rust `commandAvailable` 查询 history 是否有对应 entry；
  composition active 时两者均不可用。
- command 成功后沿用已有 `NativeCommandResult` 的 source/revision sync、selection projection、
  Accessibility refresh 和 dirty 状态回调。

## 验证

`--undo-self-check` 创建真实 `DocumentTextView`，通过同一 host route 执行插入、Undo、Redo，
验证 source 与 redo 状态；同时回归 selection、clipboard、Accessibility self-check、
`cargo test --workspace` 和 `cargo clippy --workspace --all-targets -- -D warnings`。

真实窗口中仍应人工确认菜单显示、`⌘Z`、`⇧⌘Z`、连续输入分组以及 IME commit 后的撤销体验。

## 后续

完整 visual projection/layout 接入前，所有平台原生 shell 都应提供等价的显式 history bridge，
但不得让平台文本控件成为第二个 canonical document 或 history。
