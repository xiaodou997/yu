# ADR 0047：平台无关 KeyCommand 与 macOS 原生快捷键路由

## 状态

已接受（Phase 1）

## 背景

`EditorHistory` 已经能在 `EditorDocument` 内完成分组 Undo/Redo，但 macOS 实验此前仍把
删除、移动和换行交给 AppKit 的默认 `doCommand` 路径。若快捷键在平台层各自解释，就会产生
两个命令语义；若 Swift 直接维护一份 source，又会让 Rust canonical source 与 TextKit mirror
发生漂移。IME marked text 还不能在 overlay 活跃时被永久命令修改。

## 决策

- `yu-editor` 定义平台无关的 `EditorKey`、`KeyModifiers` 和 `KeyEvent`。`Command` 表示平台的
  主应用快捷键修饰键；macOS 映射为 Command，未来 Windows/Linux 可以映射为 Control。
- `command_for_key` 只解析必须先于文本输入处理的命令：`Cmd-Z`、`Cmd-Shift-Z`、Enter、Tab、
  Shift-Tab、Backspace、Forward Delete、左右移动。普通字符和未拥有的快捷键返回 `None`，留给
  原生 text-input/default command 路径，因此不会绕过 IME。
- `yu-editor-ffi` 增加 `yu_composition_session_execute_command` 与
  `yu_composition_session_route_key`。处理结果通过 `YuEditorCommandResult` 一次返回当前
  Revision、UTF-16 selection、CaretAffinity 和 `changed`；无映射 key 返回
  `YU_FFI_KEY_UNHANDLED`，活动 composition 拒绝永久命令。
- macOS `TextInputView.keyDown` 在没有 marked text 时先调用 Rust route；处理成功后用 Rust
  canonical source 和 command result 更新 TextKit mirror。局部范围与完整同步的后续 ABI
  契约见 ADR 0048。
- 原生 `NSTextInputClient` 仍是普通字符、中文/日文 preedit、emoji 和组合字符的入口；快捷键
  路由不是第二个文本编辑器，也不创建 GUI 产品层。

## 结果

- Undo/Redo 和基础结构编辑在 macOS 实验中拥有与 Rust editor model 相同的语义，`Cmd-Z` 与
  `Cmd-Shift-Z` 已通过 Rust FFI 测试和 Swift 编译验证。
- Swift 不再需要为这些命令复制 history 或猜测 source；每个已处理命令都绑定一个结果 Revision。
- 未处理的 printable key 仍会回到 `inputContext`，所以原生 IME composition 不会被快捷键映射
  抢走。

## 限制

本阶段没有实现完整菜单/Selector registry、Option/Control 文本导航、上下移动或 macOS
Accessibility action routing。Tab/Shift-Tab 仅在 list command
实际修改 source 时由 Rust 消费；普通段落返回 unhandled，继续交给平台 focus/text-input 策略。
