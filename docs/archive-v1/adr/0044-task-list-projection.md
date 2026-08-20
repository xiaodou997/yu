# ADR 0044：Task-list marker 与 source-backed projection

## 状态

已接受（Phase 1）

## 背景

Markdown task list 同时包含普通 list container、`[ ]`/`[x]` marker 和可编辑的 item 内容。
如果把 checkbox 转换成独立富文本节点，source、selection、Undo 和 projection cache 就会出现
第二份状态；如果只把整行当作普通 paragraph，又无法提供 Typora 风格的隐藏 marker 或统一的
toggle 行为。

## 决策

- `yu-markdown` 在已有 `ListItem` block 的基础上增加 `BlockKind::TaskListItem`，只接受最多三格
  缩进、列表 marker 后的 `[ ]`、`[x]` 或 `[X]`，并要求 `]` 后为空白或行尾。`TaskState` 只表达
  `Todo`/`Done`；`TaskMarker` 保存 parser-owned 三字节 `TextRange`，不复制 Snapshot 内容。
- attached marker（例如 `- [x]attached`）仍是普通 `ListItem`，避免把任意方括号文本误认为任务
  状态。ordered list 也复用同一 marker 规则，但不扩展完整 GFM 容器语义。
- `yu-projection` 生成 `BlockProjection::TaskList`，复用 block-local inline CST，仅把
  `TaskMarker` range 加入 hidden syntax。列表 bullet、任务文本、inline emphasis/link 等仍保留
  source/visual mapping；当前不在 projection 内创建 checkbox UI。
- `yu-editor` 的 `EditorCommand::toggle_task` 只替换 marker 的状态字节，并通过普通 Transaction
  提交。因此 Revision、inverse/Undo、增量解析、projection/layout cache 失效都沿用现有编辑
  协议；非 task block 直接返回错误且不改变文档。

## 结果

- Markdown source 仍是唯一真源；`[ ]` 与 `[x]` 的切换是可逆、可记录的源码编辑。
- parser、projection 和 editor 共享同一个 source range，鼠标/键盘 checkbox 行为以后可以在不改
  文档模型的前提下叠加。
- block kind 包含 task state，所以状态切换不会复用旧 projection cache；现有 cache key 和增量
  differential test 能直接发现 stale projection。

## 限制

本阶段不实现原生 checkbox 绘制、鼠标命中 overlay、Space 快捷键、Enter 自动生成下一项、列表
缩进/续行格式化、完整 GFM task-list 规范、嵌套容器语义或导出 HTML checkbox。上述行为必须继续
通过 source-backed command/extension 边界加入，不能把视觉 checkbox 变成 canonical document
state。
