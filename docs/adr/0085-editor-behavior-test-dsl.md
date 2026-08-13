# ADR-0085：用标记状态 DSL 固定编辑器行为回归

## 状态

已接受（2026-08-13）

## 背景

`EditorDocument` 的单元测试已经覆盖了不少命令，但每个测试需要手动构造 `ByteOffset`、
`EditorSelection` 和 composition 参数。这样测试意图被坐标样板淹没，也容易只验证 source 而忘记
caret、selection、Revision 或 transient overlay。

## 决策

在 `crates/yu-editor/tests/support` 增加仅供测试使用的 `EditorScenario`：

- `|` 表示 collapsed caret；
- `⟦` 和 `⟧` 表示 selection 的 anchor/focus（顺序保留）；
- `new()` 解析标记并构造 revision-bound `EditorSelection`；
- `insert/backspace/enter/word_left/shift_down/...` 调用真实 `EditorDocument::execute`；
- `begin_composition/update_composition/commit_composition/cancel_composition` 只走公开
  `EditorDocument` composition API；
- `expect_state` 同时断言 canonical source 与 selection，`expect_revision` 和
  `expect_composition` 补充 transient/history 边界。

DSL 只存在于 integration test，不进入产品 crate，也不放宽底层坐标不变量。包含单个字面 `|` 的
Markdown 源码应使用 `from_source` + `set_caret` 或 `expect_source`，避免把表格分隔符误当成 caret。

## 结果

关键行为现在可以用接近用户操作的场景表达，并且仍然由真实 command/transaction/composition 路径
执行。当前回归覆盖：Unicode grapheme 删除与 selection replacement、日文 preedit commit/cancel、
task/list Enter 与 Undo/Redo、垂直 Shift selection。未来新增编辑命令必须优先补充 DSL 场景，再考虑
平台 UI 自动化。

## 限制

该 DSL 不模拟 AppKit、IME candidate window 或 VoiceOver；平台行为仍由 macOS spike 的 native
验收和协议审计覆盖。它也不改变正式编辑器的 source/selection ownership。
