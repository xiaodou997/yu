# 核心不变量

以下约束优先于具体数据结构和第三方库选择。

## Source

1. Markdown 源码是唯一持久化真源。
2. 投影视图不拥有源码副本，只引用 Source Range 或明确声明临时替代物。
3. 未被编辑的源码不得因解析、布局或投影而被重新序列化。
4. 第一阶段只接受有效 UTF-8；编码与 BOM 策略由 `yu-storage` 阶段补充。

## Editing

1. 所有永久修改必须经过 Transaction。
2. Transaction 中的所有 range 都属于同一个 base Revision。
3. Transaction 必须原子提交；任一 edit 非法时文档保持不变。
4. 成功提交产生严格递增的新 Revision 和可应用的 inverse Transaction。
5. 跨编辑长期存在的位置使用 Anchor，不使用裸 ByteOffset。

## Parsing

1. Lossless 结构中的源码范围必须有序、有效且能覆盖预期源码。
2. `incremental_parse(edit(old))` 必须与 `full_parse(new)` 等价。
3. parser 不得复制普通正文；节点通过源码范围引用 Snapshot。
4. malformed Markdown 必须保留为可编辑源码，不能导致文档内容丢失。

## Async work

1. 后台任务只读取不可变 Snapshot。
2. 每个结果携带输入 Revision。
3. 过期结果不得发布到当前编辑状态。
4. 取消只是优化；Revision 检查才是正确性边界。

## Input

1. IME marked/preedit text 是临时 Overlay，不自动进入 Undo 历史。
2. 只有 commit 才生成永久 Transaction。
3. OS 查询的 selection、surrounding text 和 caret rect 必须来自一致的编辑状态。
4. Source Anchor affinity 与 visual caret affinity 是两个独立语义。
5. 软换行处的 hit test 必须保留 upstream/downstream，不能只返回裸文本 offset。
6. 原生平台与 Rust composition core 的 FFI 只传递 pointer+length 的 UTF-8 buffer 和 UTF-16
   range；Rust-owned buffer、overlay 或 Snapshot 指针不得逃逸到平台层。
7. FFI 函数必须返回明确 status code，不能让 panic、未对齐的 UTF-8 或 surrogate 中间位置
   穿过 ABI；commit 成功后最多推进一次 Revision，cancel 不推进 Revision。
8. FFI source query 必须携带 expected Revision；局部查询只能复制请求范围，不能因平台查询
   而物化完整 Snapshot。
9. Unicode grapheme command 查询不得为了单次移动或删除调用完整 Snapshot 物化；跨 chunk 的
   边界必须与连续 UTF-8 文本的 extended grapheme 结果一致。
10. 原生 selection mutation 必须携带 expected Revision 和合法 CaretAffinity；Revision 过期、
    UTF-16 越界、surrogate 中间位置或未知 affinity 必须拒绝，并保持 EditorDocument selection
    不变。
11. Projection 只能引用同一 Revision 的 source range；Visible run 必须保持 source/visual
    长度一致，HiddenSyntax run 的 visual width 必须为零。

## Accessibility

1. 原生 Accessibility 文本 range 使用 UTF-16，并绑定一个明确的 Revision。
2. 过期 range 与 surrogate pair 中间位置必须拒绝，不能静默取整或套用到新文本。
3. 文本、selection、visible range 与 range bounds 必须来自同一次发布的编辑/布局状态。
4. 查询局部文本不得要求物化整个 Piece Tree Snapshot。
5. `AccessibilityTextSnapshot::from_document` 产生的 source、selected range 与 Revision 必须
   来自同一个 `EditorDocument` 状态。
6. AppKit 命中测试与 Accessibility selection 写回必须先结束活动 composition，再通过
   revision-bound FFI 更新 `EditorDocument`；平台 selection 只能作为该状态的投影。

## Degradation

1. 每个昂贵功能都必须定义预算、取消和 fallback。
2. 大文件可以关闭投影、图片、嵌入渲染和全文索引，但基本源码编辑必须保持可用。

## Projection

1. Projection 不拥有可编辑的第二份 Markdown 文本。
2. source/visual 映射必须拒绝 projection range 外的 source offset 和 visual offset。
3. hidden syntax 两侧的 caret 必须通过显式 Before/After bias 解析，不能依赖遍历顺序的
   隐式取整。
4. Inline Projection 的 hidden ranges 和 visible style 必须来自 parser-owned `InlineSpan`；
   projection 不得重新配对同一 source revision 的 delimiter。fenced code 必须走独立
   `CodeProjection`，其 body 不得经过 inline parser。
5. `ProjectionCache` entry 必须绑定当前 source Revision；严格位于 entry range 外的 edit 可以
   通过 ChangeSet 映射，触及 range 或边界的 edit 必须失效，source reset 必须清空 cache。
6. `EditorDocument::markdown().revision()` 必须等于 canonical TextBuffer Revision；block-keyed
   projection 必须同时匹配当前 block 的 source range 和 BlockKind，fenced code 必须返回
   `BlockProjection::FencedCode`。

## Layout

1. `LayoutSnapshot::revision()` 必须等于其 Projection Revision；layout 不得持有另一份可编辑
   source。
2. `VisualCluster` 的 source range 必须位于一个 visible run 内，并且只能在 Unicode grapheme
   boundary 拆分；hidden syntax 不产生可见宽度。
3. Layout hit-test 返回的 source offset 必须通过 Projection 的 source/visual mapping，不能自行
   计算第二套 delimiter 或 Unicode offset。
4. `ClusterMetrics` 只能提供 advance，不得改变 source/visual ranges；无效或非有限 advance
   必须拒绝构建 layout。
