# ADR 0008：保守的增量 Markdown Block Parse

- 状态：Superseded by ADR 0009
- 日期：2026-08-09

## 背景

Markdown block 状态可以向后传播。删除 opening/closing fence 可能改变直到文末的解释，因此
“只重解析 changed line”不满足 full/incremental 等价性。同时 parser 不应为了读取 Piece Tree
而调用 `TextSnapshot::as_str()`。

## 决策

完整 Phase 1 block scanner 直接遍历 `ChunkCursor`。每一行在单次流式扫描中产生 `LineAnalysis`：

```text
blank
0..3 spaces indentation validity
first delimiter character and run length
first character after delimiter
whether the remaining tail is whitespace
```

普通 paragraph 在前缀分类完成后使用快速 LF 扫描，不继续逐字符更新语法状态。行和 block 只
保存 source byte range，不拥有正文副本。

增量解析选择包含最早 edit 的旧 block，并额外回退一个 block，以覆盖 paragraph 跨边界合并。
此前 block 作为稳定前缀复用；从映射后的边界向 EOF 重解析。结果公开：

- `reparsed_range`；
- `reused_prefix_blocks`；
- 新 Revision 的 `MarkdownDocument`。

空 ChangeSet 复用整个文档。Previous document、ChangeSet 和新 Snapshot 的 Revision 必须严格
匹配，否则拒绝增量解析。

## 正确性门槛

三个文本后端运行相同的 1,000 次确定性随机 Markdown edit differential test：

```text
parse_incremental(previous, edit(snapshot)) == parse(new_snapshot)
```

另外单测覆盖跨 Piece delimiter/CRLF、Unicode 空白、四空格缩进、未闭合 fence，以及删除
opening/closing fence 后向 EOF 传播。

## 当前限制

1. 解析仍然传播到 EOF，没有比较 block end-state/hash 后提前停止。
2. `MarkdownDocument` 使用连续 `Vec<Block>`，创建新文档时需要复制复用前缀。
3. 当前只验证 Phase 1 block grammar，不代表完整 CommonMark/GFM。

ADR 0009 已引入带 start/end state、hash 与 suffix reuse 的持久化 block sequence。本 ADR 保留
为先建立正确性 oracle、再优化传播范围的历史依据。
