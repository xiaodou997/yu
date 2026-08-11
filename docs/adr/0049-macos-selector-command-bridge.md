# ADR 0049：macOS Selector 到共享 EditorCommand 的桥接

## 状态

已接受（Phase 1）

## 背景

`keyDown(with:)` 能处理物理按键，但 AppKit 的 `NSTextInputClient`、菜单和默认 responder
链路还会通过 `doCommand(by:)` 发送 Selector。原 spike 曾在这个方法里直接调用 Swift 的
`deleteBackward`、`moveLeft`、`moveRight` 和 `insertText("\\n")`，这会绕过 Rust 的 grapheme、
列表续行、history 和 source sync 契约。

同时，菜单验证需要知道命令在当前 selection、history、列表和 composition 上下文是否可用，
但这个查询不能通过执行命令或复制 TextKit mirror 来完成。

## 决策

- Swift 维护一个很小的 Selector allowlist：`deleteBackward:`、`deleteForward:`、`moveLeft:`、
  `moveRight:`、`moveWordLeft:`、`moveWordRight:`、`moveUp:`、`moveDown:`、
  `moveUpAndModifySelection:`、`moveDownAndModifySelection:` 和 `insertNewline:` 映射到已有的
  `YU_EDITOR_COMMAND_*` wire id。取消与
  composition 状态相关的 `cancel:`/`cancelOperation:` 仍由 overlay cancel 路径处理。
- allowlist Selector 在执行前调用 `yu_composition_session_command_available`。Rust 查询只读
  当前 `EditorDocument`：检查 composition、selection 边界、history 深度、list prefix、task
  block 和 source 边界；不推进 Revision，也不改变 selection、history 或 mirror。
- 可用 Selector 通过 `yu_composition_session_execute_command` 执行，Swift 只消费同一个
  `YuEditorCommandResult`，包括 `SourceSync`、Revision、UTF-16 selection 和 CaretAffinity。
- marked text 或 Rust composition overlay 活跃时，永久 Selector 不执行；未进入 allowlist 的
  Selector 调用 `super.doCommand(by:)`，不由 Swift 自己编辑 TextKit。
- command availability 使用 FFI byte `YU_COMMAND_UNAVAILABLE/AVAILABLE`，未知 command 和
  空指针仍返回明确 status，避免菜单验证把错误当作可用。

## 结果

- AppKit 默认 command、未来菜单入口和物理 key route 共享同一 Rust command 语义。
- 换行不再绕过 Markdown list continuation，删除和移动不再绕过 Unicode grapheme/word/visual-line selection
  mapping，所有 source mutation 都有统一的局部或完整同步范围。
- availability 可以被未来菜单/Selector registry 重用，而不会引入一份可变的 UI 文档状态。
- Rust document test、C ABI test 和 Swift 编译/self-check 覆盖边界、列表、history 和 Selector
  bridge。

## 限制

当前 allowlist 仍不包含 page movement 或完整 `validateMenuItem`/菜单 registry；Option/Control
word navigation 见 ADR 0050，visual-line/preferred-X 见 ADR 0051，垂直 modify-selection 见
ADR 0052。其他命令必须先在 Rust editor model 中定义，再扩展同一桥接。
未知 Selector 仍依赖 AppKit 默认路径，产品窗口和菜单 UI 不属于本阶段。
