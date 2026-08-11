# ADR 0048：原生命令的 SourceSync 契约

## 状态

已接受（Phase 1）

## 背景

ADR 0047 已让 macOS 原生快捷键进入共享 Rust command route，但 spike 最初在每个已修改命令后
复制完整 canonical source。该方式能验证正确性，却会让一次 Backspace 的平台同步成本随文档
长度增长。另一方面，连续输入组成的一个 Undo group 可能回放多个 Transaction，只暴露最后一次
edit 的局部 range 会导致 TextKit mirror 与 Rust source 漂移。

同步范围也不能由 Swift 或 FFI 根据 `Undo`、`Redo` 等命令名称推断；未来菜单、Selector 和其他
平台入口必须消费同一结果语义。

## 决策

- `CommandResult` 定义 `SourceSync::None`、`Range(SourceChange)` 与 `Full`。`changed=false` 强制
  使用 None，不能携带前一命令遗留的变化范围。
- `SourceChange.old_range` 是命令输入 Revision 上的 UTF-16 replacement range，`new_range` 是
  结果 Revision 上用于查询 replacement text 的 UTF-16 range。单个 Transaction 有多个 edit 时，
  Rust 返回覆盖它们及中间未变化文本的有界 range，因此一次 replace 仍能精确恢复结果。
- 普通 insert/delete/list/task command 使用 Range。Swift 以结果 Revision 调用局部 source query，
  再用返回文本替换 TextKit mirror 的 old range，不复制完整文档。
- grouped Undo/Redo 显式返回 Full，因为一次 history command 可以回放多个分散 edit。FFI 只把
  `SourceSync` 编码为 C ABI，不检查 command variant；Swift 按结果选择局部或完整 query。
- `EditorDocument::route_key` 负责上下文消费。Tab/Shift-Tab 只有在 list indent/outdent 实际修改
  source 时返回 Executed，普通段落返回 Unhandled。

## 结果

- 常见按键命令的平台同步成本与实际变化范围相关，不再随整个 Markdown 文档长度增长。
- history group 保持正确性优先；即使组内有多个 Transaction，也不会把最后一个局部 range 错当成
  整组变化。
- 快捷键、未来菜单与 Accessibility action 可以共享同一个 `CommandResult`，平台层无需复制编辑
  语义。
- Rust model test、C ABI test 与 Swift 启动 self-check 分别覆盖同步范围、协议编码和 TextKit
  replacement。

## 限制

Full history fallback 仍会复制完整 source。后续若性能数据证明有必要，可以让 `SourceSync` 增加
有序 range set 或组合后的 ChangeSet，但必须保持输入/结果 Revision 绑定以及原子 mirror 更新。
本阶段仍不实现完整菜单/Selector registry 或产品 GUI。
