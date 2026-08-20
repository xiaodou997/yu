# ADR 0045：列表编辑命令与 source-backed continuation

## 状态

已接受（Phase 1）

## 背景

任务列表已经能够解析、投影和通过 `toggle_task` 修改，但仅有 block 语义还不足以支撑原生编辑
器的基本 Enter、Backspace 和缩进体验。列表行为必须继续写回 Markdown source，不能把视觉
checkbox 或格式化后的富文本变成第二份文档状态。

## 决策

- `EditorCommand::InsertNewline` 只读取当前 source line 和 parser 已确认的 list block。普通文本
  插入当前行的 CRLF/LF（无终止符的末行使用 LF）；非空 list item 在换行后复制缩进与 marker，
  ordered marker 在 `u64` 范围内递增，task marker 新项固定为 `[ ]`。
- 在空 list/task item 的 content 末尾按 Enter 时，删除该行的 list prefix、保留原有 CRLF/LF，
  以退出列表。多选 Enter 不猜测列表结构，只替换为当前行风格的 line ending。
- `DeleteBackward` 在空 list/task item 的 prefix 之后按下时，同样删除整段 prefix；其他情况继续
  使用 Unicode grapheme boundary 删除一个 cluster。该特殊行为只适用于 parser 识别为 list block
  的行，不会在 fenced code 或普通方括号文本中触发。
- `EditorCommand::IndentList` 和 `OutdentList` 只对当前 parser list block 产生 source
  Transaction：前者插入两个 ASCII 空格，后者最多删除两个已有前导空格。selection 通过同一
  ChangeSet 映射，命令不直接改写 caret。
- 编辑器只物化当前 source line 供 prefix 判断，不调用 `TextSnapshot::as_str()` 复制整个文档；
  解析、增量更新、ProjectionCache 和 LayoutCache 继续走既有 revision-bound 路径。

## 结果

- `- [x] item` 在行尾 Enter 会生成 `- [ ] `，ordered list 会生成下一个编号，且所有修改都能
  被 inverse Transaction 记录。
- 空任务项可以用 Enter 或 Backspace 退出，避免用户逐字删除 marker；普通列表也共享该契约。
- 缩进和反缩进不会绕过 Markdown parser 或 selection mapping，列表 block kind 的变化会自然触发
  projection/layout/viewport 的结构失效。

## 限制

本阶段不实现完整 CommonMark/GFM list continuation、跨多行 selection 的结构化换行、自动合并或
重排整个 ordered list、Tab 键绑定、Space 快捷 toggle、列表格式化和 GUI checkbox hit-test。
后续行为必须继续以 source range 和 Transaction 为边界扩展。
