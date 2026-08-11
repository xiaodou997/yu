# Phase 1：Contracts & Risk Spikes

## 目标

第一阶段的目标不是完成编辑器，而是证明 Yu 最关键的数据契约和 macOS 输入路径能够成立，
并为后续 Piece Tree、增量 Markdown 与 Projection 提供不会频繁变化的边界。

## Track A：Core contracts

- [x] Rust Workspace 与固定工具链
- [x] `ByteOffset`、`TextRange`、`Revision`、`TextAnchor`、`Affinity`
- [x] 不可变 `TextSnapshot`
- [x] 原子多 edit Transaction
- [x] ChangeSet、Anchor 映射和 inverse Transaction
- [x] 参考 UTF-8 文本后端
- [x] lossless Markdown block scanner
- [x] blockquote/list item container-aware block CST v1（源码范围、深度和 marker metadata）
- [x] task-list marker 的 source-backed block state、projection 隐藏与 toggle transaction
- [x] list Enter continuation、空项退出、Indent/Outdent source command
- [x] bounded EditorHistory、inverse Transaction、Undo/Redo group replay
- [x] `yu-inspect` CLI
- [x] 可重复运行的 parse/edit 参考 benchmark harness
- [x] 持久化 Piece Tree 与 Persistent Rope 初代候选及共同 workload benchmark
- [x] 确定性随机 Transaction model test（2,000 次 Unicode edit/inverse）
- [x] Piece/leaf 局部合并与 insert/inverse 结构稳定性测试
- [x] 多版本 Snapshot 共享分配测量并选择 Piece Tree 主后端
- [x] Piece Tree 行数与 UTF-16 长度摘要、chunk cursor
- [x] Chunk-aware 完整解析与保守增量解析 differential harness
- [x] 带 start/end state、hash 与 suffix reuse 的持久化 block sequence
- [x] 长期增量 session、block retention 统计与 idle compaction 策略

## Track B：macOS risk spike

- [x] 可编译的 AppKit 实验程序
- [x] `NSTextInputClient` 的 marked text/commit/candidate rect 最小链路
- [x] 人工验证中文拼音、emoji 与 Escape cancel
- [ ] 人工验证日文、dead key 与组合重音
- [x] 日文、组合重音与 cancel 的 NSTextInputClient 协议回放
- [x] 将实验事件转换为 Rust `CompositionOverlay` 协议
- [x] 通过 C ABI static library 完成 Swift ↔ Rust `CompositionOverlay` smoke test
- [x] `EditorDocument` 统一拥有 canonical source、Revision 与 composition overlay
- [x] FFI revision-bound 局部 UTF-8 source query（不物化完整 Snapshot）
- [x] `EditorSelection`、caret affinity 与基础 Unicode command 模型
- [x] chunk-aware Unicode grapheme command 查询（不物化完整 Snapshot）
- [x] `AccessibilityTextSnapshot::from_document` 绑定 canonical selection/Revision
- [x] FFI selection revision/UTF-16 查询并接入 macOS composition commit 自检
- [x] revision-bound FFI selection mutation，并同步 macOS hit-test/Accessibility selection
- [x] FFI selection mutation/query 保留 upstream/downstream CaretAffinity
- [x] 平台无关 key command map、FFI command result 与 macOS Cmd-Z/Cmd-Shift-Z/native keyDown 路由
- [x] command result 的 None/Range/Full source sync 与 macOS TextKit 局部 mirror 更新
- [x] macOS `doCommand(by:)` Selector allowlist、Rust execute bridge 与 command availability 查询
- [x] Unicode `MoveWordLeft/Right`、macOS Option-←/→ key route 与 Selector bridge
- [x] source-backed identity/inline projection 与 hidden delimiter 双向 mapping
- [x] `yu-markdown` lossless inline token CST 被 `yu-projection` 消费
- [x] inline link/image destination ranges、soft/hard line-break tokens
- [x] delimiter flanking 校验与 intraword underscore 拒绝规则
- [x] Projection/Layout 消费显式 soft/hard LineBreak run，并隐藏 hard-break marker
- [x] explicit/collapsed reference link 与 URL/email autolink source ranges
- [x] block-level reference definition index、shortcut reference 与非局部 cache 失效
- [x] `yu-editor` revision-bound `ProjectionCache` 的命中、映射与失效规则
- [x] `yu-editor` revision-bound `LayoutCache` 的 config key、映射与 block 结构失效规则
- [x] `EditorDocument` 持有增量 `MarkdownDocument` 并提供 block-scoped projection
- [x] parser-owned semantic inline spans 与 styled visual runs
- [x] fenced code 独立 source-backed projection 协议与缓存映射
- [x] block-local `yu-layout` snapshot、grapheme cluster wrapping 与 hit-test contract
- [x] `EditorDocument::block_layout` revision-bound integration
- [x] 纯 Rust `ViewportLayout` 的 block 高度估计、可见窗口测量与增量失效
- [x] `yu-font` 字体 coverage/fallback、GlyphRun 与可替换 TextShaper 契约
- [x] shaped glyph advance 接入 `yu-layout` 换行，并保持 source cluster hit-test 映射
- [x] `yu-editor` LayoutCache/ViewportLayout 区分 metrics 与 shaped backend
- [x] macOS-only CoreText family catalog 与 fallback resolver（只返回安全字体元数据）
- [x] macOS CoreText CTLine/CTRun shaping、UTF-16→UTF-8 cluster mapping 与 `yu-layout` smoke test
- [x] macOS CoreText glyph metrics/alpha rasterization、owned CPU glyph atlas 与 metrics cache
- [x] revision-bound `yu-scene` retained primitives、viewport 与 damage coalescing
- [x] backend-neutral `yu-render` render plan、atlas page fingerprint upload 与 stale-entry 检查
- [x] shaped `GlyphPlacement` 保留 source/visual cluster 与 baseline 坐标
- [x] `LayoutSnapshot → SceneBuilder → RenderPlan` revision-bound vertical slice 与 fake uploader
- [x] macOS Metal device、未附着 `CAMetalLayer` 与 surface generation contract
- [x] macOS `MetalUploader` 的 owned alpha page → `R8Unorm MTLTexture` bridge
- [x] macOS command queue 与 clear-only drawable/present/commit frame lifecycle
- [x] macOS solid rectangle + alpha glyph Metal pipeline、UV atlas sampling 与 retained plan ABI
- [x] macOS `NSView` ↔ `CAMetalLayer` scoped attachment 与 backing layer 恢复契约
- [x] Metal surface generation-aware full clear、damage clipping 与 scissor redraw
- [x] Metal backend-owned retained color target 与 drawable blit 生命周期
- [x] probe-only AppKit main-thread host lifecycle test harness（真实 Metal session 按条件运行）
- [x] 系统 Accessibility text range 与 screen bounds 查询实验
- [x] Yu View AX text entry tree 运行时查询
- [ ] VoiceOver 实际朗读质量验证
- [x] 多行 shaping、点击和 caret round-trip

## Phase 1 退出条件

进入完整编辑器垂直切片前，必须满足：

1. 随机编辑下 Transaction + inverse 保持内容正确。
2. 新文本存储后端通过同一套行为测试。
3. Markdown 增量结果与完整解析结果可自动比较。
4. macOS 拼音 composition 不把 preedit 写入 Undo。
5. `SourceCaret → NativeCaret → Point → NativeCaret → SourceCaret` 有 identity projection
   下的最小可验证闭环。
6. 形成第一份真实性能基线，而不是仅有目标数字。
7. selection、composition commit 和永久 Transaction 使用同一个结果 Revision。
8. macOS 原生 hit-test/Accessibility selection 写回 Rust 时，stale Revision 与无效 UTF-16
   range 都能被拒绝，且不会改变旧 selection。
9. identity projection 下，hidden Markdown delimiter 不占据 visual width，且 source/visual
   caret mapping 在 Before/After bias 下可重复。
10. projection 使用 parser-owned inline source ranges；projection 内不再维护第二套 delimiter
    scanner，且 inline token coverage 与 Piece Tree chunk 解析均有测试。
11. `EditorDocument` 的 projection cache 对同一 Revision/range 命中；strictly-outside edit 可
    映射并复用，intersecting/boundary edit 会失效，reset 会清空。
12. `block_projection(index)` 只能使用当前 `MarkdownDocument` 的 block range/kind；增量 block
    结构改变后旧 entry 不得复用；fenced code block 必须返回独立 code projection，不能误走
    inline delimiter pairing。
13. matched inline spans 由 `yu-markdown` 产生，`yu-projection` 只消费 span ranges；可见 runs
    能区分 Plain/Emphasis/Strong/Code，且 fenced body 中的 `**` 等字面量保持可见。
14. `LayoutSnapshot` 必须绑定 Projection Revision；换行只能发生在 grapheme cluster 边界，
    source caret、visual caret 和 hit-test 在 Unicode/hidden delimiter 场景下保持可重复。
15. `LayoutCache` 对同一 revision/config 命中；strictly-outside edit 能映射 source ranges，
    block range/kind 改变会失效；`HeightIndex` 提供可测试的 O(log n) prefix/update/lookup，
    但不引入窗口或 GPU。
16. `ViewportLayout` 只能测量 viewport/overscan 选择的 block；未测量 block 使用显式估计，
    block source range/kind 变化必须失效，返回的 `ViewportSnapshot` 必须绑定当前 Revision。
17. Font fallback 和 shaping 结果必须携带 source cluster ranges；`yu-layout` 只能消费
    `ClusterMetrics`/glyph contract，不能把平台字体对象或 glyph cache 变成 canonical source；
    `from_projection_with_shaper` 的 glyph advance 必须影响 wrapping，且非法 source range、
    非有限 advance/offset 必须拒绝。
18. `yu-editor` 的 metrics/shaped layout 必须使用不同 cache backend key；Viewport 切换 backend
    时必须清除旧的 measured height，且 provider 不得进入 `EditorDocument` 的 canonical state。
19. macOS CoreText 调用必须隔离在 `platform/macos/yu-font-macos`；共享层只能接收自有的 family、
   PostScript name、size 与 fallback 元数据，系统 family catalog 与 live resolver 必须有 macOS
   实测测试；CoreText 对象与 rasterization 仍属于平台适配器边界。
20. CoreText shaper 必须通过 `CFAttributedString → CTLine → CTRun` 返回真实 glyph id/advance，
   将合法 UTF-16 string index 转换为 UTF-8 source cluster range，并让 `yu-layout` 的 shaped
   layout 消费这些 advance；RTL 或 non-monotonic 输出在当前布局契约下必须显式拒绝，不能静默
   重排 source range。
21. CoreText rasterizer 必须通过真实 font metrics、glyph bounds/advance 和 alpha bitmap 测试；
   `FontMetricsCache`/`GlyphAtlas` 的命中只能影响渲染准备，不能改变 source Revision、
   projection mapping 或 layout source ranges。CPU atlas page 可以被 renderer 上传，但本阶段
   不引入 GPU texture 生命周期。
22. `yu-scene` 必须绑定 source Revision 并只保存 owned geometry/color/atlas placement；`yu-render`
   必须复制 scene 的 Revision/viewport，校验 stale atlas entry，并按 page fingerprint 去重 owned
   alpha upload。此阶段不得创建 window、GPU device 或 texture handle。
23. shaped layout 的 glyph placement、scene append 和 render plan 必须存在无 GUI 的端到端测试：
   placement 的 origin 与 render command 一致，同页 atlas 只 upload 一次；missing atlas、stale
   revision 或预算失败不得留下部分 scene。
24. macOS backend 必须能在无窗口状态下验证 surface config/resize contract；native Metal device
   和 alpha texture upload 测试必须显式标记硬件前置条件，不能让无 Metal device 的默认 CI 失败。
25. macOS clear-only frame 必须保持 queue/device 绑定，并对 drawable、command buffer、encoder
   失败返回明确错误；完整 glyph/rect pipeline 不能隐式消费未验证的 RenderPlan commands。
26. macOS `MetalFrameRenderer::render_plan` 必须在无窗口单元测试中验证 painter order、glyph
   bounds、atlas UV 和缺页拒绝；在有 Metal device 的 session 中显式覆盖 pipeline creation、
   drawable acquisition、alpha sampling command encoding 和 present/commit。GPU 句柄只能存在
   `MetalAtlas`/backend，不能进入 shared render plan。
27. AppKit attachment 必须只托管外部 `NSView` 的 backing layer，并在 scoped attachment drop 时
    有条件地恢复旧 layer；首次 frame/resize/target rebuild 后 full clear，稳定 revision 的后续
    frame 只在 backend-owned retained target 上清除并重绘裁剪后的 damage 区域，再 blit 到 drawable。
    无窗口单元测试必须验证 damage clipping 和 generation 状态转换。
28. 有图形 session 时，ignored AppKit host probe 必须在 main thread 创建临时 host，完成至少一次
    attach/render、resize/render 和 scoped detach；probe 失败不能被默认无窗口 workspace 测试误判为
    产品逻辑失败。
29. block CST v1 的 block range 必须覆盖源码且不重叠；blockquote/list item 的 marker、depth 和
    lazy continuation 必须在 full parse 与 incremental parse 中一致，attached marker 不能误判为
    list item。
30. inline CST 必须保留 punctuation、link/image label/destination 和 soft/hard line-break source
    ranges；flanking 失败的 delimiter、未闭合 link 和 escaped punctuation 不得生成错误 semantic
    span，full/incremental projection 输入必须保持同一源码覆盖。
31. Projection 的 LineBreak run 必须保留 LF/CRLF source/visual range；hard-break 的尾随空格或
    反斜杠只能作为零宽 hidden syntax，metrics 与 shaped layout 必须产生同样的 line/caret 映射。
32. ReferenceLink/ReferenceImage/Autolink 必须由 parser-owned span 提供完整 opening/content/
    closing 与 reference/destination range；Projection 只能隐藏对应 syntax，不能把 `<div>` 或
    未闭合/shortcut 未解析结构误判为 semantic link。
33. Reference definition block 必须保留整行/label/destination source ranges；shortcut 只有在同
    revision definition index 命中时才隐藏，definition fingerprint 变化必须使非局部 projection、
    layout 和 viewport cache 失效。
34. task-list marker 必须在 full/incremental block parse 中保持 `[ ]`/`[x]`/`[X]` 状态和三字节
    source range；attached marker 不得误判。`BlockProjection::TaskList` 只能隐藏 marker，
    `toggle_task` 必须通过普通 Transaction 修改状态，并让 projection cache 在 Revision 变化后
    失效。
35. list editing command 必须只读取当前 source line；非空 task Enter 生成 unchecked prefix，
    ordered marker 在安全范围内递增，空项 Enter/Backspace 删除 prefix 并保留 line ending，
    Indent/Outdent 最多改动两个 ASCII 空格，所有结果都通过普通 Transaction 与 selection mapping
    验证。
36. EditorHistory 必须只保留有界 inverse Transaction；连续输入/删除/列表操作按 group 聚合，
    Undo 逆序、Redo 正序回放且不重复记录 history；移动 selection、composition 边界和新 edit
    必须正确断组或清空 redo。
37. 原生 key route 必须在有 marked text 时让位于 NSTextInputClient；无映射 key 不得修改 source，
    已映射 command 必须通过 `EditorDocument::execute` 返回同 Revision 的 UTF-16 selection 和
    CaretAffinity，Swift mirror 只能按该结果同步。
38. 局部 command 必须返回输入/结果 Revision 各自的 UTF-16 source replacement range，Swift
    只能复制新范围并替换旧范围；成组 Undo/Redo 使用 Full fallback。无变化 command 不得要求
    source copy，普通段落的 Tab/Shift-Tab 不得被 key route 吞掉。
39. macOS 已允许的 `doCommand(by:)` Selector 必须先通过 Rust availability 查询，再消费同一
    `YuEditorCommandResult`；未知 Selector 回退平台默认路径，marked text 期间永久 Selector 不得
    修改 canonical source，availability 查询本身不得修改 Revision、selection 或 history。
40. Word movement 必须在 Unicode word-boundary segment 上保持稳定 selection 映射，Option/Control
    key route 与 Selector 必须共用 `MoveWordLeft/Right`；命令不得生成 Transaction，也不得为一次
    移动物化整个 Piece Tree/Rope Snapshot。

## 非目标

- 完整 CommonMark/GFM；
- 产品级窗口、菜单或设置页；
- 自研字体 shaping；
- 三个平台同时达到产品质量；
- 第三方插件 ABI。
