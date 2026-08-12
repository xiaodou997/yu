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
6. `InsertNewline`、空 list item 的 `DeleteBackward`、`IndentList` 和 `OutdentList` 必须通过普通
   Transaction 修改 source；task continuation 必须重置为 `[ ]`，ordered marker 只能在安全范围内
   递增，不能创建富文本第二真源。
7. `EditorHistory` 只能保存有界 inverse Transaction；Undo/Redo 回放不得再次写入 history，
   entry 的 source edits 必须在回放时重绑定到当前 Revision。新的永久 edit 必须清空 redo，光标/
   selection/composition 边界必须断开当前 group。

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
9. Unicode grapheme command 查询不得为了单次移动或删除调用完整 Snapshot 物化；跨 chunk 的
   边界必须与连续 UTF-8 文本的 extended grapheme 结果一致。
10. 原生 selection mutation 必须携带 expected Revision 和合法 CaretAffinity；Revision 过期、
    UTF-16 越界、surrogate 中间位置或未知 affinity 必须拒绝，并保持 EditorDocument selection
    不变。
11. Projection 只能引用同一 Revision 的 source range；Visible 与 LineBreak run 必须保持
    source/visual 长度一致，HiddenSyntax run 的 visual width 必须为零。
12. 原生 key route 必须先解析共享 `EditorKey`/`KeyModifiers`；普通字符或未拥有的 shortcut
    必须返回 unhandled 且不得修改 source，已拥有的 command 才能进入 `EditorDocument::execute`。
    活动 composition 时不得通过 shortcut 直接修改 canonical source。
13. `YuEditorCommandResult` 的 Revision、UTF-16 selection、CaretAffinity 和 `changed` 必须来自
    同一次 command 结果；ABI 的空指针、未知 command、未知 key 和无效 affinity 必须返回明确
    status，不得写入半成品 output。
14. 原生 TextKit/AppKit mirror 在 command 成功后必须从 Rust canonical source 和 result selection
    同步；mirror 不是第二个 source/history，command route 不得把平台文本副本作为正确性边界。
15. 每个 `CommandResult` 必须显式声明 `SourceSync::None`、`Range` 或 `Full`；发生本地 source edit
    时 Range 的旧区间绑定输入 Revision、新区间绑定结果 Revision，平台只能用结果 Revision 查询
    新区间。`changed=false` 不得携带遗留 range。
16. 成组 Undo/Redo 在不能表示为单个安全 replacement 时必须请求 Full 同步；FFI 和平台不得根据
    command 名称重新猜测同步范围。Tab/Shift-Tab 在非列表上下文必须返回 unhandled。
17. macOS `doCommand(by:)` 只能将明确 allowlist 的 Selector 映射到共享 `EditorCommand`；只读
    availability 查询不得推进 Revision 或改变 selection/history，活动 composition 时永久
    command 必须不可用。未知 Selector 必须回退平台默认路径，不能直接改 TextKit mirror。
18. `MoveWordLeft/Right` 必须使用 Unicode word-boundary segment，并保持 UTF-8 source boundary；
    空白可被跨越，标点/符号/emoji 不得静默并入相邻字母词。word movement 只能改变 selection，
    不得推进 Revision 或写入 history；非连续 Snapshot 不得因单次移动被 `as_str()` 全量物化。
19. `MoveUp/Down` 必须通过当前或相邻 block 的 `LayoutSnapshot` 做 visual-line caret/hit-test
    映射；preferred-X 必须在连续上下移动中保持，且横向/word movement、edit、显式 selection、
    composition/reset 时清除。平台层不得在 Rust command 之外自行修改跨 block source selection；
    caret reveal 必须通过 revision-bound `CaretScrollRequest` 查询，真实 scroll container 仍属于
    后续 GUI 契约。
20. `MoveUpExtend/MoveDownExtend` 必须保留 `EditorSelection::anchor()`，只更新 focus，并与普通
    Up/Down 共用 layout/preferred-X/source affinity；回到 anchor 时必须产生 collapsed selection。
    Shift 垂直移动不得推进 Revision、history 或 SourceSync，marked text 期间不得执行。
21. `CaretScrollRequest` 必须绑定当前 source Revision，并由 Rust 根据 focus block 的 layout、
    高度索引和显式 block estimate 计算 document-space caret 与绝对 target scroll。caret 已在
    viewport 内时必须返回 no-op；target 必须限制在合法 content scroll 范围。FFI 查询必须携带
    expected Revision，过期请求不得被平台应用。
22. macOS native viewport consumer 只能消费仍匹配 current Revision 的 `CaretScrollRequest`；
    stale 请求不得触碰 `NSClipView`，absolute target 只能在平台边界按 content/clip height 做最后
    clamp。`NSScrollView`、`NSClipView` 和 document view 不得穿过 Rust FFI。
23. `yu_composition_session_set_viewport_config` 必须绑定 expected Revision；非法
    `max_width`/`line_height`/`default_advance`/estimate/overscan 必须拒绝且保留旧配置，成功
    配置不得推进 source、selection 或 history。metrics-only layout 的 `default_advance` 必须
    进入 `LayoutConfig`/`LayoutCache` identity，shaped backend 可独立替换它。
24. shaped layout 的每个 visual line 必须拥有有序、非重叠且 source-backed 的 source range；
    触发 wrap 的 glyph 不能扩展上一行。CoreText shaped-line diagnostic FFI 返回的 UTF-16
    range/width 必须是 owned 值，count/fill 容量不足必须返回明确 status，且诊断查询不得
    修改 canonical source、selection、history 或 Revision。
25. projection-aware shaped diagnostic 返回的 projected UTF-8 必须由 Rust parser/projection
    唯一生成；每条 line 同时携带合法且有序的 source/visual UTF-16 range，hidden syntax 可以
    让 visual range 变短但不能改写 source。Swift 临时 TextKit mirror 只能消费返回文本，
    zero-width trailing caret line 必须保持零宽并从 source-consuming line comparison 中排除。
26. `yu_composition_session_projection_caret` 必须携带 expected Revision，并在 Projection
    `Before/After` bias 下返回合法的 source/visual UTF-16 boundary 与 round-trip source；stale
    Revision、surrogate split、未知 affinity 或 projection range 错误必须拒绝且不写入半成品
    output。查询不得修改 source、selection、composition、history 或 Revision，平台只能消费
    owned scalar，不得取得 Projection/TextSnapshot 指针。
27. `yu_composition_session_block_projection_caret` 必须通过 `EditorDocument` 的
    `block_index_for_source` 和 `block_projection` 选择当前 Revision 的 parser-owned block；返回的
    visual UTF-16 必须是该 block-local projection 的坐标并携带 block index。stale Revision、无
    matching block、surrogate split、未知 affinity 或映射错误必须清空 output 并拒绝，查询不得
    物化整份文档 projection，也不得修改 source、selection、composition、history 或 Revision。
28. `yu_macos_composition_session_block_shaped_caret` 必须在同一 Revision 的 block-local
    `LayoutSnapshot` 上使用真实 CoreText shaper，返回有序 source/visual UTF-16、line index、
    有限的 block-local x/y 和正的 line height；hidden delimiter 的 Before/After affinity 可以
    改变 round-trip source，但不得改变 visual point。stale Revision、surrogate split、未知
    affinity、非法 size/max width、CoreText/layout 失败必须清空 output；非 macOS 必须返回明确的
    `YU_FFI_CORE_TEXT_UNAVAILABLE`，查询不得修改 source、selection、composition、history 或
    Revision，也不得把平台句柄暴露到 ABI。
29. `yu_macos_composition_session_shaped_caret_scroll_request` 必须使用当前 Revision 的
    CoreText-backed `ViewportLayout`/HeightIndex，返回与 `CaretScrollRequest` 相同语义的绝对
    document-space caret/target；host 必须先发布匹配的 width、line height 和 default advance，
    不匹配时拒绝而不能隐式重置 viewport measurements。stale Revision、非法尺寸/viewport、
    unavailable backend 或布局失败必须清空 output，查询不得修改 source、selection、composition、
    history 或 Revision，AppKit 对象只能留在 native adapter。
30. `yu_macos_composition_session_shaped_viewport_blocks` 必须从同一 Revision 的 shaped
    `ViewportSnapshot` 返回有序 block index/source UTF-16 range、有限且单调的 document-space
    `y`、正 height、measured 标志和稳定 kind tag；header 与 block values 必须共享 Revision。
    `capacity == 0 && blocks == NULL` 只能执行 count，不足容量不得写入部分 block 数组；stale
    Revision、非法参数、layout/unavailable 失败必须清空 header/count，查询不得修改 source、
    selection、composition、history 或把 Rust layout/Markdown/AppKit 对象暴露到 ABI。

31. `ViewportSceneInput` 进入 `yu-scene` 前必须验证同一 Revision、连续 block index、source
    range 顺序、单调 document-space origin、正 height 和 content-height 上界；
    `SceneBuilder::append_layout_at_block` 必须再验证 layout Revision/source range，并在解析
    全部 atlas entry 前保持 scene 原子。scene 只能平移 block-local layout 到已验证 origin，不能
    根据 Markdown kind、source text 或自己的 HeightIndex 重新计算 block 几何。
32. `SceneBuilder::append_viewport` 必须按 `ViewportSceneInput` 顺序预检全部 block layout；所有
    Revision/source/atlas/geometry/budget 检查成功前不得追加任何 primitive。批量提交必须同时更新
    primitive 与 damage，失败不得发布 viewport 前缀；layout 只能按 geometry origin 平移。
33. `yu-workspace::assemble_viewport_scene` 必须从同一次 `EditorDocument::visible_blocks_with_shaper`
    结果建立 `ViewportSceneInput`，再按相同 block index/config 取得 shaped layout；它不得复制或
    修改 HeightIndex、source、selection、composition 或 history。返回的 `ViewportSceneFrame`、
    `Scene` 与 `RenderPlan` 必须共享该结果的 Revision，任何 layout/atlas 失败都不得发布部分 scene。
34. `ViewportRenderFrame` 的 scene 与 render plan 必须拥有同一 Revision；`ViewportFrameCache` 只
    能发布等于调用方当前 Revision 的 frame，必须拒绝 stale frame 和较旧 Revision 回退，并在
    `invalidate_stale`/替换时保持 scene+plan 原子。cache 不得持有 source、EditorDocument、
    HeightIndex、native object 或 GPU handle。
35. `MetalFrameConsumer` 只能接受等于 macOS host current Revision 且不早于其最后接受 Revision 的
    `ViewportRenderFrame`；检查必须发生在 native command conversion 之前，只有 `render_plan` 成功
    后才能推进 consumer Revision。stale、回退或 backend 失败不得改变已接受 Revision，consumer
    不得持有 source、layout、native object 或 GPU handle。

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
