# Markdown Parser

## Full Parse

Phase 1 parser 不要求连续源码：

```text
TextSnapshot
    ↓ ChunkCursor
single-pass line scan
    ↓
LineAnalysis + container marker + composable source hash
    ↓
persistent lossless BlockSequence / flat block CST v1
```

行扫描跨 Piece/rope leaf 保留 CRLF 原始 byte range。`LineAnalysis` 只缓存 block 分类需要的有限
状态，不复制普通正文。每行在同一次扫描中计算可组合 hash，block 不需要再次查找源码。
parser 前后 retained Snapshot 的 materialized buffer 数必须不变。

当前 block CST v1 的节点仍是 source-backed root-level `Block`，不会复制源码或生成 HTML：

```text
BlankLine | Paragraph | AtxHeading | FencedCodeBlock
BlockQuote { depth }
ListItem { ordered, depth, marker, start }
TaskListItem { ordered, depth, marker, start, state }
ReferenceDefinition
```

blockquote 和 list item 的源码范围包含其连续/lazy continuation 行；嵌套 list 先以更大的
`depth` 记录为独立 source range。这样 block sequence、projection cache 和 layout cache 可以
先消费稳定的 `(range, kind)`，真正的 child arena 和稳定 node identity 留到有第二个消费者后再
抽取为通用 syntax crate。

`TaskListItem` 是 `ListItem` 的 source-backed 语义细分。根级 block parser 只在列表首行的内容
以保守的 `[ ]`、`[x]` 或 `[X]` 开始，且右方紧跟空白或行尾时识别它；marker 的三字节
`TextRange` 由 `yu_markdown::task_marker` 暴露，`TaskState` 只保存 Todo/Done。`[x]attached`
不会被识别为 task marker，仍然是普通 `ListItem`。该判断复用列表 marker 的最多三格缩进规则，
并在完整解析与增量解析中通过 block kind/hash/source range 一起收敛。

`yu-editor` 的列表命令不重新解析或序列化整个文档：它读取 caret 所在的 source line，并先确认
该行属于 `ListItem`/`TaskListItem` block。非空项的 Enter 只插入同风格 line ending 和下一项
prefix；task prefix 重置为 `[ ]`，ordered number 在安全范围内递增。空项 Enter/Backspace 删除
整段 prefix 以退出列表，Indent/Outdent 则通过两个 ASCII 空格的普通 Transaction 修改行首。
这些行为都经过 `MarkdownDocument` 增量重建，因而 block kind、projection 和 selection mapping
仍然绑定同一个 Revision。

`BlockSequence` 由不可变 `Arc<[BlockRecord]>` 分段组成。增量结果通常包含：

```text
old prefix allocation (delta = 0)
new middle allocation  (delta = 0)
old suffix allocation  (delta = edit byte delta)
```

suffix 的绝对 source range 通过 segment delta 延迟映射，因此文本长度变化不会迫使复制所有旧
block。相邻且来自同一 allocation 的连续 segment 自动合并。

## Block Lifetime

`retained_markdown_stats()` 对多个 `MarkdownDocument` 的 text Snapshot、segment table 和 block
allocation 按指针去重，统计 allocation 的完整长度，而不是仅统计被 segment 引用的 slice。这样
可以识别“大 allocation 被小 suffix 钉住”的保留放大。

同步增量 parser 不执行 block compaction。`BlockCompactionPolicy` 只给 idle task 提供建议：

```text
segments > 4096
    OR
reclaimable records >= 8192 AND retained records > active blocks * 4
```

`compact_blocks()` 将当前活动 record 复制到一个新 allocation，成本为 O(blocks)。调用方应在
输入空闲且可以释放旧 checkpoint 时执行；Undo/history 仍保留旧文档时频繁压实会同时保留新旧
完整 allocation，反而显著增加内存。

## Incremental Parse

输入契约：

```text
previous MarkdownDocument (revision N)
ChangeSet (N -> N+1)
new TextSnapshot (revision N+1)
```

算法使用分段内二分找到最早 old range 所在 block，并回退一个 block 处理相邻 paragraph 合并。
边界通过 `ChangeSet::map_anchor(..., Affinity::Before)` 映射到新 Snapshot。新的流式 parser 逐个
产生 block，并与最后一个 edit 之后的旧 block 比较。

```text
reused old prefix
        +
parse(new_start..convergence)
        +
reused shifted old suffix
        ↓
MarkdownDocument(N+1)
```

只有以下条件全部成立才允许收敛：

1. mapped source range 相同；
2. block kind、start state 和 end state 相同；
3. source hash 相同；
4. 旧、新 Snapshot 的对应 range 逐 byte 相同。

hash 只是快速过滤器，不是正确性依据。`MarkdownDocument` 保留其不可变 `TextSnapshot`，用于
hash 命中后的零拷贝 chunk 比较。围栏删除等状态扩散无法满足上述条件，会继续扫描到 EOF。

## Differential Validation

任何增量优化必须继续满足：

```text
incremental document == full document
lossless coverage == true
document revision == snapshot revision
```

当前 block state 覆盖 normal/fenced EOF 状态，container marker 已参与 block 边界和收敛比较；
完整 CommonMark 容器栈、inline 增量状态和稳定 syntax node identity 仍待后续阶段定义。每种新
状态都必须继续通过随机 differential test 和围栏/列表类病理 edit。

## Inline Parse

block-local inline parsing 继续以 `TextSnapshot + TextRange` 为输入，不生成 HTML 或可编辑文本
副本。lossless `InlineDocument` 现在保留：

```text
Text | Escaped | Delimiter
Punctuation(! [] ())
LineBreak { hard }
```

parser-owned `InlineSpan` 在 delimiter flanking 校验后产生 `Emphasis`、`Strong`、`CodeSpan`、
`Link`、`Image`、`ReferenceLink`、`ReferenceImage` 和 `Autolink`。链接/图片 span 的 `opening`、
`content`、`closing`、`destination`/`reference` 都是源码范围；projection 可以隐藏 `[]()`、
`![]()`、reference tail 和 autolink angle brackets 而保留 label/alt/text。显式或 collapsed 形式
`[label][id]`/`[label][]` 始终保留 reference source range；只有同一 `TextSnapshot` 的
`ReferenceDefinitionIndex` 命中时，`[label]`/`![label]` 才产生 shortcut reference span。未闭合
链接、未解析 shortcut、HTML-like angle text 和转义 punctuation 保持为普通可编辑源码。

## Reference Definition Index

根级 block scanner 将保守的 `[label]: destination` 行记录为 `ReferenceDefinition`，其 label、
destination 和整行 range 均只引用 source。`MarkdownDocument` 为每个 revision 持有一个
`ReferenceDefinitionIndex`；lookup 使用 ASCII case-fold 与空白折叠，并拒绝来自其他 revision
的 Snapshot。definition block 的 projection 是零宽 source-backed block，不会把定义行显示为
正文。

Task-list block 的 projection 继续消费同一 block range 和 inline CST，只额外隐藏 parser 返回的
`TaskMarker` range；列表 bullet、任务文本和其余 inline syntax 仍保持 source-backed。checkbox 的
绘制/鼠标 overlay 尚未进入本阶段，`EditorCommand::toggle_task` 目前通过一个普通 Transaction
只替换 marker 的状态字节，因此 Undo、Revision 和 projection cache 失效遵循统一编辑路径。

heading 与 blockquote 的结构前缀不再由 projection consumer 自行识别：Markdown parser 通过
`block_syntax_hidden_ranges` 返回 ATX marker、分隔空白和每个 blockquote 行的 `>` 前缀范围，
projection 使用这些 ranges 建立 `Heading`/`BlockQuote` kind。普通 `ListItem` 返回 `List`
kind，但保留 bullet 与任务文本，避免在没有 list marker scene primitive 时丢失可编辑 source。
`BlockProjectionKind` 的稳定 FFI tag 因此能区分 inline、heading、blockquote、list、table、
task 和 fenced code；GFM table 仍在 block kind 层报告为 `Paragraph`，但
`parse_table_in_snapshot` 会为同一 source range 生成 source-backed `TableProjection`。
它暴露 header、delimiter、body row 和 cell 的绝对 source byte ranges。visual stream 只保留
header/body cell 内容的 source-backed runs；pipe、cell 周围空白、每行 line ending 和
parser-owned delimiter physical row 都是 zero-width hidden ranges。`yu-layout::TableLayoutSnapshot`
在同一 source range 上按 metrics/shaper 生成可见 cell 的列宽、行高、bounds、visual range 和
content origin；它只测量 projection 中的 visible runs，因此 cell 内的 emphasis/code 等 style
会影响列宽，但 hidden pipe、周围空白和 delimiter 不会进入宽度。随后 `LayoutSnapshot` 将
source-backed cluster/glyph 重定位到 cell 的列、行和 alignment；delimiter 只作为 source range
保留，不生成可见 cell。cell-aware caret/hit-test 在隐藏结构边界使用同一 projection bias，避免
把点击落到 pipe 或行尾空白。macOS
FFI 通过 `yu_storage_session_projected_table_cells` 暴露 parser cell ranges，并通过
`yu_storage_session_table_layout_cells` / `yu_storage_session_table_cell_hit_test` 暴露
Revision-bound UTF-16 geometry 与 source-backed hit-test，Swift 不需要自行扫描 `|` 或复制文本。

表格编辑使用同一份 source-backed cell 坐标：`TableCellAddress::row = 0` 是 header，body 从
`row = 1` 开始，parser-owned delimiter physical row 不占用 visible address。编辑器的 Tab 与
Shift-Tab 只在当前 caret 位于 visible cell 时按 row-major 顺序跳到相邻 cell 的 source 起点；
它们不创建 Transaction、不改变 source Revision，也不把 delimiter pipe 当成可编辑 cell。当前
处于表格首/尾且没有目标 cell 时命令返回 `Unhandled`；自动追加 body row 留到后续阶段。

列宽/行高交互先采用纯几何查询：`yu_storage_session_table_resize_hit_test` 接受同一 Revision、
block index、layout 参数、table-local point 和 tolerance，返回内部 column/row divider 的
kind、index 与 x/y position。outer edge、表格外点、非法 tolerance 和 stale Revision 都被拒绝；
该查询不改变 source、selection、history 或 layout cache。真正的 drag transaction 和把新宽高
持久化为 Markdown 对齐/空白策略，必须在后续编辑命令阶段定义。
Rust `yu-layout::TableResizeGesture` 为 native adapter 提供同样的按下/移动/释放边界：按下时
捕获 Revision、block index、divider target 和 pointer anchor；移动只更新临时 pointer 与
proposed divider position；释放返回 `TableResizeCommit` 几何候选。任何 update/finish/cancel
都必须匹配捕获 Revision，且非有限 pointer 被拒绝。commit 不携带 cell 文本或 source edit，
因此 Markdown 写回仍必须经过另一个明确的 editor transaction。当前会话覆盖只对 column
commit 调用 `LayoutSnapshot::apply_table_resize`：它返回保持总宽度的 transient geometry，
并按相邻列最小宽度 clamp；`EditorDocument::block_layout_with_table_resize`（含 shaped
版本）不会把覆盖插入 layout cache。row commit 仍只完成 hit/gesture 协议，直到 variable-row
layout 同时更新 baseline、hit-test、viewport height index 和 scene border。storage FFI 的
`yu_storage_session_table_layout_cells_with_resize` 复用同一规则，按一次调用返回 owned
cell rectangles；Swift 不保存覆盖，也不重建表格。

definition index 的 fingerprint 只描述定义顺序、label 与 destination 内容，不包含绝对 source
offset。因此前缀插入仍可映射普通 projection；新增、删除或修改 definition 时，编辑器会保守地
清空 projection/layout/viewport cache，因为一个 definition 的变化可能影响远处的 shortcut
reference。

硬换行的 `LineBreak` range 包含两个尾随空格或反斜杠与 CRLF/LF，软换行只包含 line ending。
`yu-projection` 将 line ending 变成显式 `VisualRunKind::LineBreak { hard }`；硬换行的 marker
前缀变成零宽 `HiddenSyntax`，因此 layout 不需要再次扫描尾随空格、反斜杠或 CRLF。line-break
run 保留 line-ending 的 source/visual range，供 metrics 与 shaped layout 统一创建下一行。
