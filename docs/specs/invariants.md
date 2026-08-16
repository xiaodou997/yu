# 核心不变量

以下约束优先于具体数据结构和第三方库选择。

## Source

1. Markdown 源码是唯一持久化真源。
2. 投影视图不拥有源码副本，只引用 Source Range 或明确声明临时替代物。
3. 未被编辑的源码不得因解析、布局或投影而被重新序列化。
4. `yu-storage::DocumentSession` 只接受有效 UTF-8；UTF-8 BOM 属于文件元数据，必须在加载/保存时
   保留，但不得进入 canonical Markdown source 坐标或 parser ranges。

## Editing

1. 所有永久修改必须经过 Transaction。
2. Transaction 中的所有 range 都属于同一个 base Revision。
3. Transaction 必须原子提交；任一 edit 非法时文档保持不变。
4. 成功提交产生严格递增的新 Revision 和可应用的 inverse Transaction。
5. 跨编辑长期存在的位置使用 Anchor，不使用裸 ByteOffset。
6. `InsertNewline`、空 list item 的 `DeleteBackward`、`IndentList` 和 `OutdentList` 必须通过普通
   Transaction 修改 source；task continuation 必须重置为 `[ ]`，ordered marker 只能在安全范围内
   递增，不能创建富文本第二真源。
7. `EditorHistory` 只能保存有界 inverse Transaction；Undo/Redo 回放不得再次写入 history，
   entry 的 source edits 必须在回放时重绑定到当前 Revision。新的永久 edit 必须清空 redo，光标/
   selection/composition 边界必须断开当前 group。
8. 文件 session 的 dirty 是当前 `EditorDocument` Revision 与 `saved_revision` 的比较；Undo 回到
   相同字节也不能绕过显式保存边界。外部文件指纹变化或目标消失时，storage save 必须拒绝覆盖，
   reload 只能在 clean session 执行。

## Parsing

1. Lossless 结构中的源码范围必须有序、有效且能覆盖预期源码。
2. `incremental_parse(edit(old))` 必须与 `full_parse(new)` 等价。
3. parser 不得复制普通正文；节点通过源码范围引用 Snapshot。
4. malformed Markdown 必须保留为可编辑源码，不能导致文档内容丢失。
5. inline parser 产生的 Link/Image/Reference/Autolink destination/reference、soft/hard LineBreak
   和 delimiter span 必须只引用当前 Snapshot 的合法 source range；flanking 失败、未闭合结构、
   未命中同 revision definition index 的 shortcut、HTML-like angle text 和 escaped punctuation
   必须保持可见/可编辑，不得凭空制造 semantic node。
6. `ReferenceDefinition` 的整行、label 与 destination range 必须 source-backed；definition index
   lookup 只能消费同一 Snapshot revision，且 incremental/full parse 产生的定义顺序和 fingerprint
   必须一致。
7. `TaskListItem` 只能由列表首行中保守的 `[ ]`、`[x]` 或 `[X]` marker 产生；marker 必须有合法
   source range，右侧非空白的 attached marker 不得被识别为 task marker。`TaskState` 的改变必须
   在 full/incremental parse 中与 block kind 一致。

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
9. Native composition update/commit/cancel 除 expected Revision 外还必须携带当前 composition
   generation；generation 失配必须返回 stale status，不能触碰 canonical source 或替换新的
   marked text。
10. Unicode grapheme command 查询不得为了单次移动或删除调用完整 Snapshot 物化；跨 chunk 的
   边界必须与连续 UTF-8 文本的 extended grapheme 结果一致。
11. 原生 selection mutation 必须携带 expected Revision 和合法 CaretAffinity；Revision 过期、
    UTF-16 越界、surrogate 中间位置或未知 affinity 必须拒绝，并保持 EditorDocument selection
    不变。
12. Projection 只能引用同一 Revision 的 source range；Visible 与 LineBreak run 必须保持
    source/visual 长度一致，HiddenSyntax run 的 visual width 必须为零。
13. 原生 key route 必须先解析共享 `EditorKey`/`KeyModifiers`；普通字符或未拥有的 shortcut
    必须返回 unhandled 且不得修改 source，已拥有的 command 才能进入 `EditorDocument::execute`。
    活动 composition 时不得通过 shortcut 直接修改 canonical source。
14. `YuEditorCommandResult` 的 Revision、UTF-16 selection、CaretAffinity 和 `changed` 必须来自
    同一次 command 结果；ABI 的空指针、未知 command、未知 key 和无效 affinity 必须返回明确
    status，不得写入半成品 output。
15. 原生 TextKit/AppKit mirror 在 command 成功后必须从 Rust canonical source 和 result selection
    同步；mirror 不是第二个 source/history，command route 不得把平台文本副本作为正确性边界。
16. 每个 `CommandResult` 必须显式声明 `SourceSync::None`、`Range` 或 `Full`；发生本地 source edit
    时 Range 的旧区间绑定输入 Revision、新区间绑定结果 Revision，平台只能用结果 Revision 查询
    新区间。`changed=false` 不得携带遗留 range。
17. 成组 Undo/Redo 在不能表示为单个安全 replacement 时必须请求 Full 同步；FFI 和平台不得根据
    command 名称重新猜测同步范围。Tab/Shift-Tab 在非列表上下文必须返回 unhandled。
18. macOS `doCommand(by:)` 只能将明确 allowlist 的 Selector 映射到共享 `EditorCommand`；只读
    availability 查询不得推进 Revision 或改变 selection/history，活动 composition 时永久
    command 必须不可用。未知 Selector 必须回退平台默认路径，不能直接改 TextKit mirror。
19. macOS 基础纯文本 copy 必须从 expected Revision 的 Rust selection 读取，paste/cut 必须分别
    通过 `InsertText`/删除 `EditorCommand` 修改 canonical source；selectAll 必须通过
    revision-bound selection mutation 完成。`NSTextView` 的本地 undo/source 不得成为剪贴板正确性边界。
20. `MoveWordLeft/Right` 必须使用 Unicode word-boundary segment，并保持 UTF-8 source boundary；
    空白可被跨越，标点/符号/emoji 不得静默并入相邻字母词。word movement 只能改变 selection，
    不得推进 Revision 或写入 history；非连续 Snapshot 不得因单次移动被 `as_str()` 全量物化。
21. `MoveUp/Down` 必须通过当前或相邻 block 的 `LayoutSnapshot` 做 visual-line caret/hit-test
    映射；preferred-X 必须在连续上下移动中保持，且横向/word movement、edit、显式 selection、
    composition/reset 时清除。平台层不得在 Rust command 之外自行修改跨 block source selection；
    caret reveal 必须通过 revision-bound `CaretScrollRequest` 查询，真实 scroll container 仍属于
    后续 GUI 契约。
22. `MoveUpExtend/MoveDownExtend` 必须保留 `EditorSelection::anchor()`，只更新 focus，并与普通
    Up/Down 共用 layout/preferred-X/source affinity；回到 anchor 时必须产生 collapsed selection。
    Shift 垂直移动不得推进 Revision、history 或 SourceSync，marked text 期间不得执行。
23. `CaretScrollRequest` 必须绑定当前 source Revision，并由 Rust 根据 focus block 的 layout、
    高度索引和显式 block estimate 计算 document-space caret 与绝对 target scroll。caret 已在
    viewport 内时必须返回 no-op；target 必须限制在合法 content scroll 范围。FFI 查询必须携带
    expected Revision，过期请求不得被平台应用。
24. macOS native viewport consumer 只能消费仍匹配 current Revision 的 `CaretScrollRequest`；
    stale 请求不得触碰 `NSClipView`，absolute target 只能在平台边界按 content/clip height 做最后
    clamp。`NSScrollView`、`NSClipView` 和 document view 不得穿过 Rust FFI。
25. `yu_composition_session_set_viewport_config` 必须绑定 expected Revision；非法
    `max_width`/`line_height`/`default_advance`/estimate/overscan 必须拒绝且保留旧配置，成功
    配置不得推进 source、selection 或 history。metrics-only layout 的 `default_advance` 必须
    进入 `LayoutConfig`/`LayoutCache` identity，shaped backend 可独立替换它。
26. shaped layout 的每个 visual line 必须拥有有序、非重叠且 source-backed 的 source range；
    触发 wrap 的 glyph 不能扩展上一行。CoreText shaped-line diagnostic FFI 返回的 UTF-16
    range/width 必须是 owned 值，count/fill 容量不足必须返回明确 status，且诊断查询不得
    修改 canonical source、selection、history 或 Revision。
27. projection-aware shaped diagnostic 返回的 projected UTF-8 必须由 Rust parser/projection
    唯一生成；每条 line 同时携带合法且有序的 source/visual UTF-16 range，hidden syntax 可以
    让 visual range 变短但不能改写 source。Swift 临时 TextKit mirror 只能消费返回文本，
    zero-width trailing caret line 必须保持零宽并从 source-consuming line comparison 中排除。
28. `yu_composition_session_projection_caret` 必须携带 expected Revision，并在 Projection
    `Before/After` bias 下返回合法的 source/visual UTF-16 boundary 与 round-trip source；stale
    Revision、surrogate split、未知 affinity 或 projection range 错误必须拒绝且不写入半成品
    output。查询不得修改 source、selection、composition、history 或 Revision，平台只能消费
    owned scalar，不得取得 Projection/TextSnapshot 指针。
29. `yu_composition_session_block_projection_caret` 必须通过 `EditorDocument` 的
    `block_index_for_source` 和 `block_projection` 选择当前 Revision 的 parser-owned block；返回的
    visual UTF-16 必须是该 block-local projection 的坐标并携带 block index。stale Revision、无
    matching block、surrogate split、未知 affinity 或映射错误必须清空 output 并拒绝，查询不得
    物化整份文档 projection，也不得修改 source、selection、composition、history 或 Revision。
30. `yu_macos_composition_session_block_shaped_caret` 必须在同一 Revision 的 block-local
    `LayoutSnapshot` 上使用真实 CoreText shaper，返回有序 source/visual UTF-16、line index、
    有限的 block-local x/y 和正的 line height；hidden delimiter 的 Before/After affinity 可以
    改变 round-trip source，但不得改变 visual point。stale Revision、surrogate split、未知
    affinity、非法 size/max width、CoreText/layout 失败必须清空 output；非 macOS 必须返回明确的
    `YU_FFI_CORE_TEXT_UNAVAILABLE`，查询不得修改 source、selection、composition、history 或
    Revision，也不得把平台句柄暴露到 ABI。
31. `yu_macos_composition_session_shaped_caret_scroll_request` 必须使用当前 Revision 的
    CoreText-backed `ViewportLayout`/HeightIndex，返回与 `CaretScrollRequest` 相同语义的绝对
    document-space caret/target；host 必须先发布匹配的 width、line height 和 default advance，
    不匹配时拒绝而不能隐式重置 viewport measurements。stale Revision、非法尺寸/viewport、
    unavailable backend 或布局失败必须清空 output，查询不得修改 source、selection、composition、
    history 或 Revision，AppKit 对象只能留在 native adapter。
32. `yu_macos_composition_session_shaped_viewport_blocks` 必须从同一 Revision 的 shaped
    `ViewportSnapshot` 返回有序 block index/source UTF-16 range、有限且单调的 document-space
    `y`、正 height、measured 标志和稳定 kind tag；header 与 block values 必须共享 Revision。
    `capacity == 0 && blocks == NULL` 只能执行 count，不足容量不得写入部分 block 数组；stale
    Revision、非法参数、layout/unavailable 失败必须清空 header/count，查询不得修改 source、
    selection、composition、history 或把 Rust layout/Markdown/AppKit 对象暴露到 ABI。

33. `ViewportSceneInput` 进入 `yu-scene` 前必须验证同一 Revision、连续 block index、source
    range 顺序、单调 document-space origin、正 height 和 content-height 上界；
    `SceneBuilder::append_layout_at_block` 必须再验证 layout Revision/source range，并在解析
    全部 atlas entry 前保持 scene 原子。scene 只能平移 block-local layout 到已验证 origin，不能
    根据 Markdown kind、source text 或自己的 HeightIndex 重新计算 block 几何。
34. `SceneBuilder::append_viewport` 必须按 `ViewportSceneInput` 顺序预检全部 block layout；所有
    Revision/source/atlas/geometry/budget 检查成功前不得追加任何 primitive。批量提交必须同时更新
    primitive 与 damage，失败不得发布 viewport 前缀；layout 只能按 geometry origin 平移。
35. `yu-workspace::assemble_viewport_scene` 必须从同一次 `EditorDocument::visible_blocks_with_shaper`
    结果建立 `ViewportSceneInput`，再按相同 block index/config 取得 shaped layout；它不得复制或
    修改 HeightIndex、source、selection、composition 或 history。返回的 `ViewportSceneFrame`、
    `Scene` 与 `RenderPlan` 必须共享该结果的 Revision，任何 layout/atlas 失败都不得发布部分 scene。
36. `ViewportRenderFrame` 的 scene 与 render plan 必须拥有同一 Revision；`ViewportFrameCache` 只
    能发布等于调用方当前 Revision 的 frame，必须拒绝 stale frame 和较旧 Revision 回退，并在
    `invalidate_stale`/替换时保持 scene+plan 原子。cache 不得持有 source、EditorDocument、
    HeightIndex、native object 或 GPU handle。
37. `MetalFrameConsumer` 只能接受等于 macOS host current Revision 且不早于其最后接受 Revision 的
    `ViewportRenderFrame`；检查必须发生在 native command conversion 之前，只有 `render_plan` 成功
    后才能推进 consumer Revision。stale、回退或 backend 失败不得改变已接受 Revision，consumer
    不得持有 source、layout、native object 或 GPU handle。
38. `yu_storage_session_projected_source` 与 `yu_storage_session_projection_caret` 必须携带
    expected Revision，并只返回由 Rust parser/projection 生成的 owned visual UTF-8/UTF-16 值；
    stale Revision、surrogate split、未知 affinity 或 projection 错误必须拒绝。查询不得修改
    canonical source、selection、composition、history 或 Revision，Swift 不得从 delimiter 自行
    推导 source range。
39. `yu_storage_session_projection_block_count` 与 `yu_storage_session_projected_block` 必须携带
    expected Revision；block index、source UTF-16 range、parser kind、projection kind 和 visual
    UTF-8/UTF-16 长度必须来自同一 Rust parser/projection revision。stale Revision、无效 index、
    UTF-8/UTF-16 转换失败或 projection 错误必须拒绝，且不得写入半成品 metadata；count/fill
    查询只能返回 owned bytes/scalars，不得让 Swift 解析 Markdown、取得 Block/Projection 指针，
    或修改 source、selection、composition、history、Revision。
40. `yu_storage_session_composition_projection`、`yu_storage_session_copy_composition_projection`
    与 `yu_storage_session_composition_caret` 必须同时遵守 canonical Revision 与 transient
    composition generation：metadata 先清空再写入，copy/caret 的 stale generation、无 overlay、
    UTF-16 surrogate split 或投影转换失败必须拒绝。projected UTF-8、preedit/visual selection、
    active marked caret 和 round-trip source 必须由 Rust `Projection::with_composition` 生成；
    begin/update/cancel 期间 source、Markdown CST、selection/history 和 Revision 不得改变，Swift
    只能消费 owned bytes/scalars，不能自行拼接 preedit 或推导 hidden delimiter range。
41. `yu_storage_session_projection_selection` 与 `yu_storage_session_projection_hit_test` 必须
    同时携带 expected Revision；selection 的 visual range、source round-trip 和 hit-test 的
    source/visual/line/point/affinity 必须来自同一 Rust projection/layout snapshot。stale Revision、
    surrogate split、未知 affinity、无效 layout config 或非有限 point 必须拒绝且不得写入半成品
    output；point 结果是 layout-local owned scalar，平台必须自行完成 screen/view 坐标转换，不能
    在 Swift 复制 Markdown projection、delimiter 语义或 layout。
42. `yu_storage_session_block_layout`、`yu_storage_session_macos_block_layout` 与
    `yu_storage_session_macos_block_caret` 必须以 expected Revision 和 parser-owned block index
    绑定同一 `DocumentEditorSession`。metrics/CoreText layout 只能返回 owned source range、
    block-local visual length、line/size/caret scalar；stale Revision、越界 block、surrogate split、
    非法尺寸或 CoreText/layout 失败必须清空 output 并拒绝，不能让 Swift 保存 LayoutSnapshot、
    Projection、CoreText 句柄或第二份 Markdown source。
43. `yu_storage_session_set_viewport_config` 与
    `yu_storage_session_macos_shaped_viewport_blocks` 必须共享同一 expected Revision 和
    CoreText width/line-height/default-advance contract；shaped viewport header 与每个 block 必须
    共享 Revision，block index/source UTF-16 range 必须有序，document-space `y` 必须单调、height
    必须为正且 content height 有限。count/fill 的容量不足不得写入部分 block，stale、非法 viewport、
    metrics 不匹配、layout/CoreText 失败和非 macOS unavailable 必须清空 header/count 并拒绝；Swift
    只能消费 owned scalar，不得复制 HeightIndex、Markdown block 或 layout 对象。
44. `yu_storage_session_projection_source_caret` 与
    `yu_storage_session_projection_source_selection` 必须从同一 expected Revision 的 Rust
    projection 将 visual UTF-16 边界映射回 source UTF-16，并返回 visual round-trip 边界；非折叠
    visual selection 必须使用 Before/After 外缘保留 hidden Markdown syntax。stale Revision、未知
    affinity、visual UTF-16 surrogate split、逆序/越界 range 或 projection 错误必须先清空 output
    再拒绝；TextKit 只可持有 owned projected text/selection，不能解析 Markdown、缓存 source range
    或在 Rust source 变更后继续使用旧 Revision。
45. `DocumentTextView` 的 visual pointer adapter 只能在拥有同一 Revision 的 disposable visual
    TextKit mirror 时处理点击/拖选；Rust reverse mapping 成功后才能提交 source selection 并更新
    native source mirror，任何 stale/范围/映射失败都必须回退 AppKit source selection，不能修改
    source、composition 或 history。默认产品窗口必须关闭该 adapter，直到 visual renderer、scroll
    origin 和 IME 坐标共享同一 visual layout。

46. visual IME overlay 只能使用同一 expected Revision + composition generation 返回的 Rust
    projected UTF-8、visual replacement range 和 marked selection；`DocumentTextView` 在 metadata、
    mirror 或 `markedRange` 任一版本不匹配时不得发布 visual callback，应回退 canonical source
    mirror。visual preedit、marked range 和 attributed substring 都不得改变 source、selection、
    history 或 Revision；cancel/commit 仍必须经由 generation-bound Rust composition API。
47. `yu_storage_session_macos_shaped_viewport_blocks` 的 header 与 block 必须共享同一 Revision；
    `scroll_y`、`viewport_height`、`max_scroll_y`、block document-space `y/height` 和
    `yu_storage_session_macos_shaped_caret_scroll_request` 的 caret/target scroll 必须来自同一
    CoreText metrics 与 Rust `ViewportLayout`。Swift 只能做显式 document↔viewport 平移并在 Revision
    不匹配时丢弃 snapshot；不得复制 HeightIndex、重算 block origin，或把 stale target 应用到新文本。

## Accessibility

1. 原生 Accessibility 文本 range 使用 UTF-16，并绑定一个明确的 Revision。
2. 过期 range 与 surrogate pair 中间位置必须拒绝，不能静默取整或套用到新文本。
3. 文本、selection、visible range 与 range bounds 必须来自同一次发布的编辑/布局状态。
4. 查询局部文本不得要求物化整个 Piece Tree Snapshot。
5. `AccessibilityTextSnapshot::from_document` 产生的 source、selected range 与 Revision 必须
   来自同一个 `EditorDocument` 状态。
6. AppKit 命中测试与 Accessibility selection 写回必须先结束活动 composition，再通过
   revision-bound FFI 更新 `EditorDocument`；平台 selection 只能作为该状态的投影。
7. macOS document host 的 `NSTextView` source mirror 只能消费统一
   `DocumentEditorSession` 的 Rust-owned snapshot；它可以接收 native 输入，但不得形成第二份
   可变 source、dirty 或 history。
8. 可写 native host 必须只持有一个 `DocumentEditorSession` handle；command、selection、key
   route、composition、save 和 close 的 Revision/dirty 结果必须来自同一 `EditorDocument`，不得
   通过并列 storage/editor handles 复制 source 或猜测 state。

## Degradation

1. 每个昂贵功能都必须定义预算、取消和 fallback。
2. 大文件可以关闭投影、图片、嵌入渲染和全文索引，但基本源码编辑必须保持可用。

## Projection

1. Projection 不拥有可编辑的第二份 Markdown 文本。
2. source/visual 映射必须拒绝 projection range 外的 source offset 和 visual offset。
3. hidden syntax 两侧的 caret 必须通过显式 Before/After bias 解析，不能依赖遍历顺序的
   隐式取整。
4. Inline Projection 的 hidden ranges、LineBreak runs 和 visible style 必须来自 parser-owned
   `InlineSpan`/`InlineNode`；projection 不得重新配对同一 source revision 的 delimiter。fenced
   code 必须走独立 `CodeProjection`，其 body 不得经过 inline parser。
5. `ProjectionCache` entry 必须绑定当前 source Revision；严格位于 entry range 外的 edit 可以
   通过 ChangeSet 映射，触及 range 或边界的 edit 必须失效，source reset 必须清空 cache。
6. `EditorDocument::markdown().revision()` 必须等于 canonical TextBuffer Revision；block-keyed
   projection 必须同时匹配当前 block 的 source range 和 BlockKind，fenced code 必须返回
   `BlockProjection::FencedCode`，reference definition 必须返回零宽 projection。definition index
   fingerprint 变化时，projection/layout/viewport cache 必须整体失效；否则 strictly-outside edit
   才允许映射复用。
7. task-list block 必须返回 `BlockProjection::TaskList`；projection 只能隐藏 parser-owned
   `TaskMarker` source range，不能删除 bullet 或任务文本。`EditorCommand::toggle_task` 必须只用
   普通 Transaction 替换 marker 状态字节，非 task block 必须拒绝且不改变文档。
8. 列表编辑命令只能对当前 parser 识别的 `ListItem`/`TaskListItem` 行生效；空项退出必须保留
   原有 line ending，Indent/Outdent 最多改动两个 ASCII 空格，selection 必须通过 ChangeSet 映射。

## Layout

1. `LayoutSnapshot::revision()` 必须等于其 Projection Revision；layout 不得持有另一份可编辑
   source。
2. 普通 `VisualCluster` 的 source range 必须位于一个 visible run 内，并且只能在 Unicode
   grapheme boundary 拆分；LineBreak cluster 只能来自显式 LineBreak run；hidden syntax 不产生
   可见宽度。
3. Layout hit-test 返回的 source offset 必须通过 Projection 的 source/visual mapping，不能自行
   计算第二套 delimiter 或 Unicode offset。
4. `ClusterMetrics` 只能提供 advance，不得改变 source/visual ranges；无效或非有限 advance
   必须拒绝构建 layout。
5. `LayoutCache` entry 必须绑定当前 source Revision，并同时匹配 block 的 source range、
   `BlockKind`、`LayoutConfig` 和 `LayoutBackend`；strictly-outside edit 可映射，交集或 block
   结构变化必须失效，metrics 与 shaped entry 不能互相命中。
6. `HeightIndex` 只索引已经产生的视觉行高，不得隐式触发全文 layout；prefix、point update 和
   viewport line lookup 必须保持与原始 height values 一致。
7. `ViewportLayout` 的未测量 block 必须使用显式 estimate；一次 viewport 查询不得为了定位
   可见窗口而隐式 layout 全部 block，返回的 block y/height 和 source range 必须来自同一
   Revision。
8. `LayoutSnapshot::from_projection_with_shaper` 必须验证 shaped output 的 source range 属于
   请求的 visible run；glyph cluster range 必须有序且可映射回 Projection，advance 和 offset
   必须有限。合法 glyph advance 决定 wrapping，但不得改写 source buffer。
9. `ViewportLayout` 的 measured height 必须绑定当前 `LayoutBackend`；切换 metrics/shaped
   backend 必须先将旧 measured entry 恢复为 estimate，不能用旧 backend 的高度定位 viewport。
10. shaped layout 的 `GlyphPlacement` 必须保留 face/glyph identity、source/visual cluster range
    和 painter-order 的 x/baseline 坐标；metrics-only layout 必须返回空 placement 列表，不能
    从 cluster metrics 推导伪造 glyph identity。

## Font and shaping

1. Font selection、fallback 和 shaping 都是 source/layout 的输入，不得生成第二份可编辑文本。
2. 每个 `Glyph` 或 glyph cluster 必须保留 source `TextRange`；fallback 切换只能拆分
   `GlyphRun`，不能改变 source/visual mapping 的语义。
3. shaping backend 必须显式携带方向、script、style 和 font request；缺失 face/glyph 或无效
   advance 必须报告错误或走明确 fallback，不能静默伪造平台字体状态。
4. `MockShaper` 只用于确定性测试；CoreText、DirectWrite、Fontconfig 等平台实现必须通过
   同一 `TextShaper` 边界接入，不能进入 `EditorDocument` canonical state。
5. 平台字体适配器可以在自身边界内持有 `CTFontRef` 等原生句柄，但跨边界结果必须是自有、可
   发送的元数据或 `GlyphRun`；字体 catalog/fallback 查询不得修改 source Revision，也不得把
   平台对象放入 `yu-font`、`yu-layout` 或 `EditorDocument` 的 canonical state；system UI 的
   私有 `.SFNS-*` alias 不得传给普通 family lookup，必须通过平台专用创建 API，viewport
   metrics FFI 只能返回 owned scalar。
6. CoreText 的 UTF-16 glyph string index 只能在合法 UTF-8 scalar boundary 上转换为 source
   `TextRange`；glyph cluster range 必须有序、位于请求范围内，RTL/non-monotonic 输出在布局尚未
   支持时必须返回明确错误而不是调整索引伪装成 LTR。
7. glyph rasterizer 的跨边界结果必须是自有 `FontMetricsSnapshot`、`GlyphMetrics`、
   `GlyphBitmap` 或 `RasterizedGlyph`；CoreText/CoreGraphics 句柄、context、纹理和 atlas page
   生命周期不得进入 `TextSnapshot`、projection、layout 或 `EditorDocument` canonical state。
   atlas placement 必须绑定 `GlyphRasterKey`，空 bitmap glyph 可以没有 page 但必须保留 advance。

## Scene and render

1. `yu-scene::Scene` 必须绑定一个 source `Revision`，primitive 顺序必须保持 painter order；
   scene 不得持有 source text、layout cache、native window object、bitmap ownership 或 GPU handle。
2. scene 的 `Point`/`Rect` 必须是有限值，宽高不能为负；glyph bounds 必须由 atlas metrics 和
   baseline origin 推导，不能在 renderer 中重新计算 source/layout 坐标。
3. `DamageSet` 只能包含非空、有限矩形；相交/相邻区域必须可合并，超过预算时必须显式退化为
   总 bounds，不能无限增长每帧 dirty list。
4. `RenderPlan` 必须复制 scene 的 Revision 和 viewport；`RenderPlanBuilder` 对 scene 引用的
   atlas entry 必须验证 key、page、rect 和 metrics 与当前 atlas 一致，missing/stale entry
   必须失败而不是绘制错误 glyph。
5. atlas page upload 必须是 owned alpha bytes，page fingerprint 未变化时不得重复产生 upload；
   `RenderUploader` 返回的 texture/device handle 只能由 backend 持有，不能写回 scene、layout 或
   editor canonical state。
6. `SceneBuilder::append_layout` 必须先校验 layout/scene Revision、font size 和所有 atlas
   entry，再追加 glyph primitive；任一失败不得发布部分 layout scene。glyph primitive 的 origin
   必须直接来自 placement 的 layout 坐标，不能在 renderer 中重新推导 source position。
7. macOS partial-damage frame 的 backend command culling 必须只按已验证 native command bounds
   与 damage region 做严格相交过滤，保留原 painter order、跨 region command 只保留一次；full
   clear 必须保留完整 command list，culling 不得修改 shared `RenderPlan`、scene 或 Revision。

## macOS Metal boundary

1. `platform/macos/yu-render-macos` 可以拥有 `MTLDevice`、`CAMetalLayer` 和 `MTLTexture`，但
   这些 native pointers 不得出现在 shared scene、layout、render plan 或 editor canonical state。
2. `MetalSurfaceConfig` 必须验证 logical size/scale 并显式计算 drawable pixel size；只有 native
   resize 成功后才能更新 config 和 generation。
3. `MetalViewAttachment` 必须以 scoped lifetime 持有 native attachment；drop 时只有在 view 仍
   指向 Yu layer 的情况下才能恢复之前的 backing layer，不能覆盖 AppKit 或其他组件后来安装的
   layer。传入的 `NSView` 指针只能在 AppKit main thread 使用。
4. `MetalUploader` 只能接受长度与 page width×height 一致的 owned alpha bytes；texture handle
   的释放由 macOS backend 负责，不能写回 `GlyphAtlas` 或 `RenderPlan`。
5. `MetalCommandQueue`、`MetalPipeline` 和 `MetalSurface` 必须绑定同一个 `MetalDevice`；任何
   device mismatch 都必须在 native 调用前拒绝。
6. `MetalAtlas` 可以拥有 page texture，但 `RenderPlan` 只能通过 page id 引用它们；计划中的
   glyph 必须在提交前找到对应 page，empty glyph（`page: None`）只能被跳过，不能伪造纹理。
7. `MetalRenderTarget` 是 backend-owned 的持久颜色存储；CAMetalLayer drawable 不能被假定为
   跨帧内容真源。每次成功 render plan 必须把 retained target blit 到当前 drawable，target
   尺寸与 drawable 不一致时必须拒绝提交。
8. `MetalFrameRenderer::render_plan` 第一次提交、target 重建或 surface generation 变化后必须
   full clear；后续提交只能清除 `RenderPlan::damage()` 经过 viewport 裁剪后的区域，并在这些
   区域设置 scissor。没有 damage 的稳定 revision 不得被误当成全屏 dirty。
9. `MetalFrameRenderer::render_plan` 必须保持 `RenderPlan::commands` 的 painter order，solid
   rectangle 使用 solid pipeline，glyph 使用对应 page 的 alpha sampling pipeline；source/layout
   坐标只能在 Rust command conversion 边界按 viewport/scale 转换一次。
10. native command ABI 的 geometry、UV、颜色、damage 和 page id 必须是有限、已验证的 owned 数组；atlas
   rectangle 越界、未知 command kind、缺页或无效 viewport 必须返回错误，不得提交半成品 frame。
11. `present_clear` 与 `render_plan` 的 drawable、command buffer、blit encoder 失败都必须转换为明确
    `MetalRenderError`；没有有效 drawable 的硬件测试可以返回 `DrawableUnavailable`，但不能改变
    full-clear/generation 状态。
12. AppKit host probe 只能存在于 macOS ignored lifecycle test：它必须在 AppKit main thread 创建并
    销毁临时 `NSWindow`/`NSView`，只验证 attachment、resize、drawable 和 detach 边界，不得进入产品
    backend API 的窗口所有权，也不得把 probe state 写入 shared editor state。
13. `MetalFrameRenderer::submit_viewport_frame` 必须按 `current Revision gate → atlas staging →
    render_plan → consumer commit` 顺序消费 `ViewportRenderFrame`；stale frame 不得上传 atlas 或
    进入 native command path，atlas staging/native/backend 失败不得推进 consumer Revision。相同
    page fingerprint 只能在同一 Metal device 上复用，submission result 只能返回 owned scalar。
14. `MetalViewportHostSession` 只能持有 revision-bound `ViewportFrameCache`、current Revision、
    surface generation、host-local frame serial 和 owned scalar submission；`advance_revision` 与
    `sync_surface_generation` 必须拒绝回退，Revision/generation 变化必须清理不匹配的 frame 或
    last submission。`submit` 必须先验证 surface generation，再消费 current frame，且失败不得
    写入 last submission；session 不得持有 EditorDocument、source、layout、AppKit 或 GPU handle。
15. `yu-workspace::ViewportFramePublisher` 必须从 `EditorDocument` 当前 Revision 组装
    `ViewportRenderFrame`，返回把 frame、Revision 与 monotonic serial 绑定在一起的 owned
    `ViewportFramePublication`；发布失败或 serial overflow 不得替换已有 publication。macOS
    `MetalViewportHostSession::accept_publication` 必须拒绝旧 Revision、内部 Revision 不匹配、
    重复或回退 serial，验证完成前不得写入 host cache。
16. `ViewportFrameCache`、`ViewportFramePublication` 和 macOS host 接收路径必须通过不可变
    `Arc<ViewportRenderFrame>` 共享 frame allocation；handle clone 不得深拷贝 scene/render plan，
    `frame()` 借用与 `current_frame_handle()` owned handoff 必须保持同一 Revision，且 Arc 不得
    携带 EditorDocument、source、layout cache、surface 或 GPU handle。
17. `ViewportFramePublisher::publish` 必须使用 staged `RenderPlanBuilder`；只有 frame Revision、
    serial 和 cache handoff 全部成功后才提交 atlas page fingerprint。组装失败、stale/cache 拒绝或
    serial overflow 不得改变调用方 builder、已有 cache、last publication 或 next serial，随后用同一
    builder retry 必须重新产生与直接成功等价的 upload plan。
18. `Projection::with_composition` 只能生成 transient `VisualRunKind::Composition`，不得修改
    `TextSnapshot`、Markdown CST、source Revision 或 parser-owned delimiter runs；composition run
    的 shaping 坐标必须是临时零基 range，layout 产生的 glyph/cluster 必须映射回 canonical
    replacement range。`EditorDocument::block_layout_with_composition*` 不得写入 LayoutCache，
    composition 更新、cancel 和 stale commit 不得增加 history 或推进 Revision。
19. `yu_storage_session_macos_visual_scene` 返回的每个 primitive 必须携带同一 Revision、parser-owned
    block index 和 source UTF-16 range；Rust 必须先通过 `ViewportSceneInput`/`SceneBuilder` 验证
    顺序、有限矩形和 content bounds，再执行 count/fill。容量不足或 stale Revision 不得发布部分
    primitive，Swift 不得根据 block kind、source text 或数组位置重新推导 scene geometry。
20. `yu_storage_session_macos_visual_render_plan` 返回的 snapshot、glyph command、atlas page 和
    damage 必须来自同一 Revision；command 必须保持 RenderPlan painter order，source UTF-16 range、
    origin、bounds、advance、page rectangle、fingerprint 和 damage 都必须是有限且已验证的 owned
    scalar。count/fill 在 Rust 完整构造 plan 后执行；任一数组容量不足或 stale/invalid 输入不得写入
    部分输出，Swift 不得重建 glyph layout、atlas 像素或 page identity。
21. `yu_storage_session_macos_render_host_surface_submit` 只能在 AppKit main thread 接收调用方仍然
    有效的 `NSView`，并严格按 `MetalSurface attachment → current Revision frame publication →
    atlas staging → render_plan/drawable submit → consumer commit` 顺序执行；surface、renderer、
    atlas、target、attachment 和 queue 都是 Rust/backend-owned 状态，只能留在
    `YuStorageSession` 的 opt-in adapter 中，不能跨 ABI 返回。stale Revision、无 drawable、drawable
    尺寸不匹配、atlas/native 失败都必须返回错误且不写入 `submitted=1`；成功结果只能包含 owned
    scalar。首次创建 surface generation 必须为 0；同一 adapter 后续只能通过受控 resize 单调推进
    generation。
22. `MacosPersistentSurfaceState` 复用同一 view 的 `MetalSurface`、`MetalFrameRenderer`、`MetalAtlas`
    和 target；相同 Revision 的重复 submit 不得重新上传未变化 page，surface config/scale 变化必须
    通过 `MetalSurface::resize` 单调推进 generation，并在 host submit 前同步该 generation。view
    identity 变化必须先显式 detach；owned attachment 的 `Drop` 必须先恢复 view backing layer，再
    释放 surface。detach/close 不得留下 native layer、GPU handle 或 host submission。
23. `yu-font-macos::FaceTable` 的 numeric `FontFaceId` 必须在同一进程内的 CoreText shaper/rasterizer
    实例之间保持稳定；layout cache 可以把 face id 交给后续新建的 rasterizer，不能依赖清空 layout
    state 来修复 shaper identity。Face catalog 只保存 Rust-owned PostScript name metadata，不得
    把 `CTFontRef` 或 native pointer 写入 shared editor/layout state。
24. `yu_storage_session_macos_font_metrics` 返回的 metrics 必须通过 expected Revision，并且 size、
    line height、default advance 都必须是有限正数；它不依赖 parser block，空 Markdown 也必须能用
    它初始化 viewport。stale Revision 必须清空输出并拒绝后续 viewport configuration。
25. `MacosSurfaceHostView`/`MacosSurfaceHostCoordinator` 只能在 AppKit main thread 把 window、layout、
    scroll、edit Revision 和 close 事件映射到 persistent surface submit；同一 Revision/geometry/scale
    submit key 不得重复提交，view 离开 window、window close 或 controller 销毁前必须幂等 detach。
    surface host 是 source TextKit mirror 的 sibling，不得拥有 Markdown source、selection、IME、
    accessibility semantic nodes 或 native GPU handle。

## CoreText viewport preparation

1. `CoreTextViewportFrameBuilder` 可以持有 shaper、CPU `GlyphAtlas`、`RenderPlanBuilder` 和
   `ViewportFramePublisher`，但不得持有 `EditorDocument`、canonical source、selection、history、
   `MetalSurface`、`MTLTexture` 或其他 native pointer；`publish` 只能从调用方传入的当前 document
   Revision 读取并发布 frame。
2. builder 的可见 glyph rasterization 必须先确保所有 layout placement 都有同一 font size 的 atlas
   entry，再调用 staged `ViewportFramePublisher`；同一 Revision 的重复 publish 不得重复产生未变化
   page upload，新增 glyph 导致 page fingerprint 变化时才允许产生增量 upload。
3. `publish_and_submit` 必须保持 `CoreText preparation → publication/revision gate → MetalAtlas
   sync → MetalFrameRenderer::render_plan → consumer commit` 顺序；任何 preparation、stale、atlas、
   native 或 drawable 失败都不得推进 host 的 last submission 或 consumer Revision。
4. macOS document-host FFI 的 persistent render state 可以持有
   `CoreTextViewportFrameBuilder` 与 `MetalViewportHostSession`，但不得持有 `EditorDocument`、
   source、selection、history 或 AppKit/GPU handle；每次 host frame 都必须先验证 expected
   Revision，再同步 surface generation、发布当前 document frame，最后只返回 owned scalar
   metadata。surface generation 与 frame serial 不得回退，stale Revision 不得污染 snapshot。
5. `yu_storage_session_macos_visual_scene_glyphs` 的 header、glyph 数组和 persistent host publication
   必须绑定同一 Revision、frame serial 与 surface generation；count/fill 的容量不足路径不得写入
   部分 glyph。每个 glyph 只能携带 Rust-owned atlas placement、几何、颜色和所属 block 的 source
   UTF-16 range，不得跨 ABI 暴露 atlas 像素、CoreText/scene/layout 指针或 GPU handle；当前 glyph-only
   bridge 遇到非 glyph primitive 必须整体拒绝，而不能静默丢弃。
