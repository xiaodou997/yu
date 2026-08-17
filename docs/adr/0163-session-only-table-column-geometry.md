# ADR 0163：GFM table 的会话级列宽覆盖

## 状态

已接受（2026-08-17）

## 背景

标准 GFM Markdown 没有表达列宽或行高的字段。上一阶段已经把 pointer drag
建模为 Revision-bound、source-neutral 的 `TableResizeGesture` 和
`TableResizeCommit`，但 commit 不能直接变成 Markdown transaction。当前
`TableLayoutSnapshot`、line layout、viewport height index 和 scene border 也都
以统一行高为基础。

## 决策

1. 第一版视觉覆盖只支持相邻列的临时宽度调整。`LayoutSnapshot::apply_table_resize`
   接受同一 Revision 的 column commit，保持表格总宽度不变，并把两列分别限制在
   可用的最小宽度之上；cell source/visual range、Markdown 文本、selection、history
   和 canonical layout cache 都不改变。
2. `EditorDocument::block_layout_with_table_resize`（以及 shaped 版本）返回一个不进入
   cache 的 transient layout。覆盖由 native/session caller 持有，source edit 或 Revision
   变化后必须丢弃旧 commit。
3. row resize 本阶段仍只提供命中和 gesture 协议。由于当前 row bounds、baseline、
   `hit_test`、block height、Fenwick height index 和 scene border 依赖 uniform row height，
   不在没有完整 variable-row 设计与回归测试前伪造行高覆盖。
4. 暂不把列宽写回 GFM，也不引入未经设计的 HTML/CSS 或隐藏注释。未来若需要跨会话
   保存，另行选择 Yu 扩展或 sidecar 格式，并单独定义序列化、版本、Undo 和冲突策略。

## 结果

- native drag 可以在当前帧看到真实的列宽变化，同时不会制造 Markdown 的第二份真源。
- 过小/过大的拖动会在相邻两列之间安全 clamp，总表宽度和行几何保持稳定。
- session-only override 不会污染下一次普通 layout 查询，因此可以继续使用现有 cache key。
- 行高持久化被明确后置，避免只改 cell rectangle 却遗漏 viewport、caret 和 scene 的不一致。

## 验证

- `yu-layout` 覆盖列宽移动、总宽度、cell x/content origin、source range、最小宽度 clamp
  和 row commit 拒绝。
- `yu-editor` 覆盖 transient layout 不进入 cache、canonical layout 不变，以及 source
  Revision 变化后旧 commit 被拒绝。
