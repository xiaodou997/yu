# Yu 架构总览

## 产品模型

Markdown 源码是唯一持久化真源。投影视图、完整源码视图和局部语法揭示共享同一个
`TextBuffer`，不通过富文本模型往返序列化。

```text
File bytes
    │
    ▼
Text Snapshot ──► Lossless Markdown ──► Projection ──► Layout ──► Scene
    ▲                                                                  │
    │                                                                  ▼
Transaction ◄── EditorCommand ◄── EditorSelection ◄── Hit Test ◄──── GPU
```

## 两条验证线

Phase 1 同时推进两个方向：

```text
Core contracts                  macOS risk spike
──────────────                  ────────────────
Revision/Snapshot               NSView
Anchor/Affinity                 NSTextInputClient
Transaction/Inverse             marked text
Lossless source ranges          candidate rect
Parser equivalence              native event loop
           │                           │
           └──────── first vertical slice ────────┘
```

平台实验允许被重写，其任务是尽早发现核心接口无法满足原生同步查询、IME 或可访问性的地方。

## 依赖方向

第一阶段只创建已经有明确契约和验证价值的 crate：

```text
yu-core
   ▲
   │
yu-text ◄── yu-markdown ◄── yu-projection ◄── yu-layout ◄── yu-editor ◄── yu-editor-ffi
   ▲             ▲                                 ▲              ▲              ▲
   │             │                                 │              │              │
yu-core      yu-inspect                    block layout     ProjectionCache   macOS/Swift shell
                                                   │
                                                   ▼
                                               yu-scene
                                                   │
                                                   ▼
                                               yu-render
                                                    ▲
                                                    │
                                           yu-workspace
                                           ▲     ▲     ▲
                                           │     │     │
                                      yu-editor yu-scene yu-font
```

后续预计增加：

```text
yu-markdown-edit
yu-platform
yu-storage
yu-export
yu-extension-host
```

只有当边界已通过真实调用证明稳定时才拆成 crate。通用 `yu-syntax` 暂不创建；Green/Red Tree
首先服务 Markdown，出现第二个真实消费者后再抽取。

## 并发模型

平台 UI 线程拥有窗口和输入状态。后台解析、搜索、布局或扩展任务只能读取不可变 Snapshot，
并在发布结果前比较 Revision：

```text
Snapshot(41) ──► background task ──► Result(41)
                                           │
Current revision == 41? ── yes ── publish  │
                       └── no  ── discard ◄┘
```

任何后台任务都不能持有可变文档引用。

`yu-editor::EditorDocument` 是编辑阶段的状态边界：它同时保存 canonical `TextBuffer`、当前
Revision 对齐的 `MarkdownDocument`、revision-bound `EditorSelection`、transient composition
和 `ProjectionCache`。平台 view 可以保留 AppKit 的渲染/输入投影，但永久命令必须回到该边界
并通过 Transaction 提交。

composition 需要参与视觉布局时，`Projection::with_composition` 在 canonical projection 上建立
一个不入缓存的临时视图：替换范围外仍使用 parser-owned runs，preedit 是 plain
`VisualRunKind::Composition`。layout/shaping 通过 projection 读取临时文本和零基 shaping range，
再把 glyph/cluster 映射回 canonical replacement range；因此 preedit 可以改变换行、宽度和 caret
visual range，但不会改变 source bytes 或 Markdown 语义。`EditorDocument` 只提供
`block_layout_with_composition*` 这类 transient 查询，commit/cancel 后普通 cache 仍是唯一可复用
的 canonical layout。见 ADR 0076。

`EditorHistory` 只保存有界 inverse Transaction，不保存完整 Snapshot。连续输入、删除和列表命令
按 group 聚合；Undo 逆序回放、Redo 正序回放，并将每个 entry 的 base Revision 重绑定到当前
Revision。光标移动、显式 selection、composition 边界和 reset 会断开 group；新的永久 edit 会清空
redo。回放绕过 history recording，但仍完整经过 Markdown 增量解析、selection 映射和 projection/
layout/viewport cache 失效。

左右移动和删除通过 `yu-text::TextSnapshot` 的 chunk cursor 与 Unicode grapheme cursor 查询
相邻边界；Accessibility 使用 `AccessibilityTextSnapshot::from_document` 一次性绑定 source、
selection 和 Revision，不从平台 view 复制一份 canonical selection。

macOS mouse hit-test 和 Accessibility selection 的反向同步路径是：

```text
AppKit NSRange
    │
    ▼
cancel active composition
    │
    ▼
yu_composition_session_set_selection(expected Revision, UTF-16 range, CaretAffinity)
    │
    ▼
EditorDocument::set_selection
```

该路径只更新 selection，不生成 Transaction；Rust FFI 会拒绝 stale Revision 和无效
UTF-16 boundary/affinity。这样 AppKit 的原生 selection 与 caret affinity 仍是
`EditorDocument` 状态的投影，而不是第二个 canonical selection。

macOS keyDown 的命令路由也以 Rust 为唯一解释器：

```text
NSEvent
   │
   ▼
无 marked text？ ── no ──► NSTextInputClient / inputContext
   │ yes
   ▼
yu_composition_session_route_key
   │
   ├── YU_FFI_KEY_UNHANDLED ──► native text-input/default command
   │
   └── YuEditorCommandResult
           │
           ├── SourceSync(None / Range / Full)
           ├── revision-bound canonical source query
           ├── UTF-16 selection + CaretAffinity
           └── TextKit mirror / Accessibility notification
```

`EditorKey`/`KeyModifiers` 与 `command_for_key` 位于 `yu-editor`，因此 Swift 只负责把 AppKit
keyCode、charactersIgnoringModifiers 和 modifierFlags 转换成 ABI 值。Cmd-Z、Cmd-Shift-Z、
Enter、Tab/Shift-Tab、删除和左右移动通过同一 `EditorCommand` 执行；普通字符不经过该映射，
继续由 `NSTextInputClient` 处理。command result 绑定当前 Revision，活动 composition 则必须
先 commit/cancel，不能被永久命令绕过。

本地 source command 在结果中携带输入 Revision 的旧 UTF-16 range 与结果 Revision 的新 range；
AppKit 只查询新 range 并替换 mirror 的旧 range。成组 Undo/Redo 可能回放多个不连续 Transaction，
因此显式请求 Full fallback。同步范围由 `CommandResult` 决定，FFI、快捷键和未来菜单入口不按
command 名称重复推断。没有 source 变化的移动命令使用 None；非列表 Tab/Shift-Tab 返回
unhandled。

AppKit 的 `doCommand(by:)` 也走同一边界：实验只允许删除、前后/word/上下移动和换行 Selector 映射到
`yu_composition_session_execute_command`，先通过只读 `yu_composition_session_command_available`
查询上下文，再按同一个 `YuEditorCommandResult` 更新 mirror。取消 composition 是唯一不经过
永久 command 的 Selector；未知 Selector 交还 `super`，活动 marked text 时不执行永久命令。

左右 word movement 使用同一 `EditorKey` 加 Option/Control modifier 映射到
`MoveWordLeft/MoveWordRight`。Rust 只读取 caret 所在行和必要的相邻行，通过 Unicode word-boundary
segment 跳过空白、保留标点/符号/emoji 的独立边界；该命令只改变 revision-bound selection，不
生成 Transaction，也不调用 `TextSnapshot::as_str()` 物化整份非连续 source。

上下移动同样不把 AppKit selection 当成第二个真源：`MoveUp/MoveDown` 先由
`LayoutSnapshot::caret_for_source` 把当前 source caret 定位到 block-local visual line，使用私有
`PreferredCaretX` 保留第一次命中的 X，再以目标行 y 调用 `LayoutSnapshot::hit_test` 反向得到 source
boundary。横向/word movement、edit、显式 selection 和 composition/reset 会清除 preferred-X；
非空 selection 的 Up/Down 先折叠到 ordered start/end。当前可在相邻 Markdown block 的首/末视觉行
间穿越，但不把前一 block 的合成 trailing empty caret line 重复暴露。`EditorDocument` 另提供
revision-bound `CaretScrollRequest`：它消费当前 viewport、focus block 的 layout 和高度索引，
返回 document-space caret geometry 与绝对 target scroll；平台只应用仍属于当前 Revision 的目标，
macOS `YuNativeViewportAdapter` 再把 target 转换为 `NSClipView.bounds.origin.y`。当前 spike 已将
`TextInputView` 作为真实 `NSScrollView.documentView`，从 TextKit used rect 同步 native content
height，并在 Rust command、selection 写回和 IME commit 后消费 reveal。它通过 revision-bound
viewport metrics FFI 发布 native container width、CoreText system UI line height、混合 grapheme
sample 的 shaped advance、estimated block height 和 overscan，Rust 随后直接以 native point
计算 request，不再在 bridge 中乘除临时 scale。`.SFNS-*` 这类私有系统 UI alias 通过
`CTFontCreateUIFontForLanguage` 创建，不能走普通 family lookup；FFI 只返回 owned scalar。
fallback advance 只属于 metrics-only backend，正式产品必须由共享 shaped-layout metrics 替换。

该 host attachment 不改变 Rust source/selection/layout 的所有权：adapter 只保存 NSScrollView
viewport、Revision 和 content height，stale request 仍被丢弃。见 ADR 0054、ADR 0055、ADR 0056
与 ADR 0057。

Shift+上下使用独立的 `MoveUpExtend`/`MoveDownExtend` command：`EditorSelection::anchor()` 保持不动，
只把 hit-test 得到的 visual caret 写入 focus。macOS `moveUpAndModifySelection:`/
`moveDownAndModifySelection:` 与 `EditorKey` 的 `SHIFT` modifier 共用该命令；focus 回到 anchor
时 selection 自然折叠，命令仍不推进 Revision 或 source sync。

Caret reveal 不从 `CommandResult` 猜测：macOS FFI 通过 expected Revision、viewport scroll/height
和 margin 查询 `YuEditorCaretScrollRequest`。Rust 负责 focus block 的 layout、前缀高度和 target
clamp；caret 已可见时返回 no-op。查询不会推进 source Revision，过期请求必须被平台丢弃。

`yu-projection::Projection` 现在提供一个 source-backed inline 试验层：它只保存
`TextSnapshot`、source range、visible/line-break/hidden runs 和双向 mapping，不生成第二份可编辑文本。
它通过 `yu-markdown::parse_inline` 或 definition-aware 的
`parse_inline_with_definitions` 获取 parser-owned `InlineDocument` 和 matched
`InlineSpan`，不再在 projection 内维护 delimiter pairing；visible run 同时携带 Plain、Emphasis、
Strong 或 Code style，Link/Image/ReferenceLink/ReferenceImage/Autolink 的 syntax range 由
parser-owned span 隐藏但目前仍使用 Plain label/alt/text style；`MarkdownDocument` 提供的同一
revision `ReferenceDefinitionIndex` 还可解析 shortcut reference；definition block 自身使用
zero-width source-backed projection。LineBreak run 携带 soft/hard 标记，hard marker bytes
作为 hidden syntax，供 layout 直接建立 visual line。当前 span 仍是保守的 Phase 1 语义层，不
宣称完整 CommonMark inline AST。

`yu-editor::EditorDocument` 拥有 revision-bound `ProjectionCache`：同一 Revision/range 查询命中
缓存，永久 edit 会映射严格位于 changed range 外的 projection，并保守地使相交或边界 projection
失效。definition index fingerprint 变化时，编辑器还会清空 projection、layout 与 viewport
cache，避免远处 shortcut reference 继续使用旧语义。`block_projection(index)` 以当前
`MarkdownDocument` 的 `(range, kind)` 为 key，并在增量 block sequence 更新后再次验证 entry；
普通 block 返回 inline projection，reference definition 返回零宽 source-backed projection，
fenced code 返回独立的 `CodeProjection`，只隐藏 fence 行并把 body 当作字面量 code run，不会把
body 中的 Markdown delimiter 当作 emphasis。
task-list block 返回 `BlockProjection::TaskList`，只隐藏 parser-owned `TaskMarker` 的 `[ ]`/
`[x]`/`[X]` source range，列表 bullet 和任务文本仍可通过 source/visual mapping 定位。当前
`EditorCommand::toggle_task` 将状态字节替换为普通 Transaction；因此 source Revision、Undo 和
projection/layout cache 失效与其他永久编辑一致，checkbox 的原生绘制和鼠标 overlay 留到 GUI
阶段。
composition overlay 不推进 source Revision，因此不会触发 projection cache 失效。

列表编辑命令也保持 source-backed：`InsertNewline` 只读取当前行，非空 list item 复制其缩进和
marker（task 新项重置为 `[ ]`，ordered marker 在可表示范围内递增）；空 list/task item 的
Enter 或 `DeleteBackward` 删除 prefix 以退出列表。`IndentList`/`OutdentList` 只对 parser 识别的
list block 插入/删除两个 ASCII 空格，selection 仍由同一 ChangeSet 映射。当前行扫描只构造小的
line-local 字符串，不物化完整 Snapshot。

`yu-layout::LayoutSnapshot` 是 block-local、revision-bound 的纯 Rust 布局契约：它消费
`Projection` 的 visible 与显式 LineBreak runs，按 grapheme cluster 生成 `VisualLine`/`VisualCluster`，并提供
`LayoutCaret` 与 `LayoutHit` 的 source/visual 双向查询。当前默认只使用确定性的
`MonospaceMetrics`；`yu-font::FontMetrics` 已提供同一接口的 fallback-aware adapter，真实字体
shaping 可通过 `LayoutSnapshot::from_projection_with_shaper` 注入；该入口消费
`ShapedText/GlyphRun` 的 glyph advance 和 source cluster range，布局层仍不依赖窗口或 GPU。
`ClusterMetrics` 保留为不需要 glyph 级数据的兼容入口。
`EditorDocument` 现在拥有独立的 `LayoutCache`：entry 以 block 的 `(range, kind)` 和
`LayoutConfig` 和 `LayoutBackend` 为 key，同一 revision/config/backend 查询命中同一个
cache-owned snapshot。`EditorDocument::block_layout_with_shaper` 可以在不把 provider 存进
canonical document 的情况下构建 shaped entry；metrics 与 shaped entry 不会互相命中。永久
edit 会通过 layout/projection mapping 保留严格位于 changed range 外的布局，并在 block range
或 kind 改变时删除 entry。`LayoutSnapshot::height_index` 暴露 Fenwick prefix index，支持
后续 viewport virtualization 的 O(log n) 高度查询与点更新；当前仍未连接窗口、GPU 或真实
字体。
`yu-editor::ViewportLayout` 在此之上维护每个 Markdown block 的估计/实测高度：viewport 查询
只测量窗口及 overscan 内的 block，更新 `HeightIndex` 后重新计算可见范围，并返回带 source
range、kind、y/height 的 `ViewportSnapshot`。前缀 edit 会映射未触碰 block 的估计和实测状态，
block 结构变化则保守失效；切换 metrics/shaped backend 会把已测量高度退回 estimate，避免
使用错误的换行结果；它仍是纯 Rust 的测量/索引层，不是渲染器。

`yu-font` 定义了平台无关的 `FontDatabase`、coverage/fallback、`FontRequest`、方向/script
提示、`TextShaper` 和可分 fallback face 的 `ShapedText/GlyphRun`。`MockShaper` 只用于契约测试，
不会冒充 CoreText/DirectWrite 的真实 shaping；`FontShaper` 将它桥接为 layout 的
`ShapingProvider`，glyph advance 参与 line breaking，但不改变 source/visual 坐标。未来原生
backend 可以替换它而不改变 layout 的 source/visual 坐标。

在正式 GUI 之前，macOS spike 还提供一个只读的 shaped-line comparison probe：
`yu-editor-ffi::yu_macos_core_text_shaped_lines` 使用同一份 UTF-8 source、System UI
CoreText shaper 和 native point width，返回 owned 的 UTF-16 source line ranges 与宽度。
Swift 侧把这些范围与 TextKit line fragments 逐行比较；count/fill ABI 不携带任何
CoreText 对象，也不推进 `EditorDocument` Revision。比较时会过滤共享 editor layout 保留的
zero-width trailing caret line；这些行必须仍然有序、宽度为零，但不属于 TextKit 的 source-
consuming line fragments。该 probe 同时约束 `yu-layout` 的 source range 不重叠规则，但目前
只覆盖 plain source，Markdown hidden syntax、复杂 fallback 和最终 shaped viewport 仍需后续
projection/layout 契约。

projection-aware probe 在此之上调用 `Projection::inline`，通过同一个 FFI count/fill 返回
projected UTF-8 和 source/visual UTF-16 line ranges。Swift 只把 projected 文本装入临时
TextKit storage，再比较 visual ranges；因此 `**strong**`、link destination 等 hidden
syntax 的宽度由 Rust projection 决定，平台层不会复制 Markdown parser。当前 self-check
用宽容器和显式换行隔离 source/visual 映射；shared grapheme wrapping 与 TextKit 自然语言
word-break 的差异仍是后续独立议题。

在该 probe 之后，`yu_composition_session_projection_caret` 提供更窄的 revision-bound source
caret 查询：Rust 以当前 `EditorDocument` 的 parser-owned projection 将 source UTF-16 boundary
映射为 visual UTF-16，并按 `ProjectionBias::Before/After` 返回同一 visual boundary 的
round-trip source。Swift 只传 `NSSelectionAffinity`、expected Revision 和 scalar 结果；stale
Revision、surrogate split 或未知 affinity 都在 FFI 边界拒绝。该查询不改变 source、selection、
composition、history 或 Revision，且暂不携带 line/point/CoreText 对象。

在产品路径上，`yu_composition_session_block_projection_caret` 进一步把同一契约限制在当前
Markdown block：`EditorDocument::block_index_for_source` 负责 boundary 选择，随后通过
`block_projection` 命中 parser-owned projection cache。返回的 visual UTF-16 是 block-local，
并携带 block index；因此 native layout/reveal 可以只准备当前 block，不需要为一次 caret 查询
物化整份文档 projection。它与 ADR 0060 的全局诊断 ABI 并存，仍不携带 line/point 或平台句柄。

在该局部映射之上，`yu_macos_composition_session_block_shaped_caret` 使用同一
`EditorDocument` 的 `block_layout_with_shaper` 和 CoreText System UI shaper，返回 block-local
visual UTF-16、round-trip source、line index、x/y 和 line height。结果是 revision-bound 的
owned scalar；Swift 只负责将 block origin、viewport 和 AppKit caret view 组合起来。隐藏
delimiter 的 upstream/downstream 仍共享 shaped point，但 round-trip source 保留 affinity。
非 macOS 保留 ABI 并返回 unavailable；这一步仍是 geometry/IME 风险验证，不是完整 GUI。

macOS host 在该 geometry 之后可以调用 `yu_macos_composition_session_shaped_caret_scroll_request`。
它复用当前 `ViewportConfig` 的 estimate/overscan 和 `ViewportLayout` HeightIndex，但将当前
block 的真实 line count/height 交给 CoreText-backed `caret_scroll_request_with_shaper`；返回值
仍是 revision-bound absolute document-space caret/scroll scalar。host 必须先发布相同的
CoreText width/line metrics，Rust 不会在一次 scroll query 中静默重置既有 viewport measurements。
`YuNativeViewportAdapter` 的职责仍限于 stale 检查与最后的 AppKit clip clamp。

随后 `yu_macos_composition_session_shaped_viewport_blocks` 以 count/fill ABI 暴露同一 shaped
`ViewportLayout` 的局部 metadata：Revision、可见 block range、content height、source UTF-16
range、document-space block origin/height、measured 和稳定 kind tag。它只复制 owned scalar，
不暴露 `ViewportSnapshot`、Markdown block 或 layout 对象；因此 scene/document view 可以直接
消费 block geometry，同时保持 Rust 为唯一的 block height/source 边界。

`yu-scene::ViewportSceneInput` 是 metadata 进入 retained scene 的下一层边界。它再次验证 block
顺序、source range、document-space y/height、content height 和 Revision；`SceneBuilder` 的
`append_layout_at_block` 只把已经验证的 block-local shaped layout 平移到 geometry 的 document
origin，绝不根据 kind 或 source 重新布局。这样 FFI/native host、editor layout cache 和 scene
都共享同一 block origin，而不会各自维护 HeightIndex 的副本。

完整可见窗口使用 `SceneBuilder::append_viewport` 批量组装：它按 `ViewportSceneInput` 的 block
顺序预检所有 layout、source range、Revision、atlas entry、glyph bounds 和 primitive budget，
然后一次性发布 glyph primitives 与 damage。任何一个 block 失败都不会留下 viewport 前缀；这
使 stale frame 可以整体丢弃并在新的 Revision 重试，而不会让 renderer 接收到部分窗口。

`yu-workspace::assemble_viewport_scene` 是 editor 到 retained scene 的组合边界。它只消费
`EditorDocument::visible_blocks_with_shaper` 返回的当前 Revision metadata，并按同一 block index
取得 shaped `LayoutSnapshot`；随后交给 `ViewportSceneInput` 和 `SceneBuilder::append_viewport`。
因此 macOS host 不需要复制 Markdown block traversal、HeightIndex 或 layout cache，后续窗口/Metal
层只消费 `ViewportSceneFrame`/`RenderPlan`。

`ViewportRenderFrame` 将 scene 与 render plan 绑定到同一个 Revision，`ViewportFrameCache` 是当前
文档的单项发布门：`publish_if_current` 拒绝 stale frame，也不允许旧 Revision 回退覆盖新 frame；
编辑后可以先 `invalidate_stale`，再发布新结果。这个 cache 不拥有 source、EditorDocument、
HeightIndex、native object 或 GPU handle，只保存最近一次可提交的 owned frame。

macOS backend 在这个 cache 之后再设一层 `MetalFrameConsumer`：`MetalFrameRenderer::render_viewport_frame`
只有在 workspace frame 与 host current Revision 相同、且不早于已接受 Revision 时才进入
`render_plan` 的 command conversion 和 native Metal path。成功返回后才记录 Revision；缺页、target、
drawable 或 native encoder 失败都不会推进 consumer。consumer 不持有 editor/source/layout 或 GPU
对象，并可通过 revision-only 单元测试覆盖 stale 与回退，而无需创建窗口。

macOS host 的推荐提交入口是 `MetalFrameRenderer::submit_viewport_frame`。它把顺序固定为
`Revision gate → MetalAtlas::sync_plan → render_plan → consumer commit`，并返回只含 Revision 与
上传页数的 `MetalFrameSubmission`。`MetalAtlas` 对同一 device 的相同 page fingerprint 去重，
staging 失败不会替换已有 texture；device mismatch 在 native 调用前拒绝。现有 AppKit ignored
probe 已使用真实 workspace frame 覆盖 stale、匹配提交、resize 后再次提交和 atlas 复用，但仍不
拥有产品窗口。

`MetalViewportHostSession` 是这一 backend 边界上的 host 状态机。它只保存 current Revision、
surface generation、frame cache、host-local frame serial 和最后一次成功 submission 的 owned
scalar；`advance_revision`/`sync_surface_generation` 都拒绝回退，`publish_frame` 只接受当前
Revision，`submit` 只消费匹配 generation 的 cached frame。这样真实 AppKit view 后续只需把编辑、
resize 和 frame publish 事件翻译成 session 调用，不需要在 Swift/ObjC 中复制 stale 或 viewport
generation 规则。

`yu-workspace::ViewportFramePublisher` 是平台 host 之前的共享发布边界。它读取
`EditorDocument` 当前 Revision，组装 `ViewportRenderFrame`，并返回同时拥有 frame、Revision
和 monotonic serial 的 `ViewportFramePublication`；发布器自身不拥有 source、native object
或 GPU handle。macOS `MetalViewportHostSession::accept_publication` 会再次验证 publication
Revision、frame 内部 Revision 与 serial 顺序，再把 frame 放入 host cache。这样平台层只负责
surface lifecycle 和 Metal submission，不再自己组装 viewport 或猜测 frame 更新状态。

`ViewportFrameCache`、`ViewportFramePublication` 和 macOS host cache 现在通过不可变
`Arc<ViewportRenderFrame>` 共享同一个 frame allocation。借用查询仍返回普通 frame reference，
只有跨边界 handoff 才 clone handle；因此发布/接收不会复制 scene 或 render plan。Arc 只覆盖
backend-neutral render preparation，不改变 `EditorDocument`、GPU texture 或 surface 的所有权。

发布器对 `RenderPlanBuilder` 使用 staged clone：组装过程中产生的 atlas page fingerprint 先留在
临时 builder，只有 frame Revision、serial 和 cache handoff 全部成功后才 move 回调用方。这样
serial overflow、stale cache 或组装失败不会推进上传去重状态，失败后可用同一个 builder 重新发布；
staged state 只复制轻量 fingerprint map，不复制 scene、source、layout 或 GPU 资源。见 ADR 0075。

`platform/macos/yu-font-macos` 是 macOS-only 的 CoreText 适配层。`CoreTextFontCatalog::system`
负责读取 CoreText 当前可见的非私有 family 名称，`CoreTextFontResolver::resolve` 负责根据
`FontRequest` 和文本请求 CoreText 的 family/fallback 选择；`CoreTextShaper` 再通过
`CFAttributedString → CTLine → CTRun` 取得真实 glyph id、advance、position 和 UTF-16 string
index，并转换为 `yu-font` 的 `GlyphRun`/UTF-8 source cluster。适配层只把自有的 family、
PostScript name、size、glyph 数据和 fallback 标志返回给共享层，不让 `CTFontRef` 或其他平台
句柄进入 `yu-font`、`yu-layout` 或 `EditorDocument`。当前布局契约对 RTL/non-monotonic run
仍会显式报错。`CoreTextGlyphRasterizer` 在同一平台边界内复制 font metrics、glyph bounds、
advance 和 alpha-only owned bitmap；`yu-font::GlyphAtlas` 只保存 CPU page、placement 和自有
像素，供后续 renderer 上传 GPU，绝不进入 canonical source 或 document state。当前尚未绑定
wgpu/Metal texture，也不处理彩色 glyph、subpixel LCD 或完整 BiDi。

`yu-scene` 接收 layout/投影侧准备好的 glyph placement 和几何，生成绑定 source `Revision` 的
retained scene。它只保存 primitive、viewport、颜色和 damage rectangles，不保存 source text、
bitmap 或 GPU handle。`DamageSet` 会合并相交/相邻区域，并在超出预算时收敛到总 bounds。

shaped `yu-layout::LayoutSnapshot` 保留 painter-order 的 `GlyphPlacement`：face/glyph identity、
source/visual cluster range、line index、x 和 baseline y。metrics-only layout 的 glyph list 为空，
因此不会伪造 atlas identity。`SceneBuilder::append_layout` 在追加前检查 layout/scene
Revision、font size 和 CPU `GlyphAtlas` entry，并在全部 placement 解析成功后按顺序生成 glyph
primitive；失败不会留下部分 scene。

`yu-render` 将 scene 与对应的 CPU `GlyphAtlas` 转换为 backend-neutral `RenderPlan`：命令保持
painter order，atlas page 通过尺寸/bytes fingerprint 去重 `AtlasPageUpload`，stale/missing
entry 会被拒绝。共享 `yu-render` 当前没有 `wgpu`/Metal device 或窗口依赖；`RenderUploader` 只定义未来 backend
上传 alpha page 的最小边界。`yu-render` 已用 fake uploader 覆盖 `FontShaper → LayoutSnapshot →
Scene → RenderPlan` 端到端 revision、atlas upload 去重和 command origin；实际 texture 生命周期
和 command encoding 由 macOS backend 承担，不回写 shared plan。

`platform/macos/yu-render-macos` 是共享 render boundary 之外的第一层真实 backend：Rust 侧拥有
`MetalDevice`、`CAMetalLayer`、surface generation、`MetalTexture`、`MetalAtlas`、
`MetalPipeline` 和 backend-owned retained color target，Objective-C bridge 只负责 Apple
framework 调用。它可以把
`AtlasPageUpload` 上传成 `R8Unorm` texture，并通过 `MetalCommandQueue`/`MetalFrameRenderer`
验证两条 frame 路径：`present_clear` 的 drawable → command buffer → clear → present/commit，
以及 `render_plan` 的 solid rectangle 与 alpha glyph quad。后者使用内嵌的最小 Metal shader
source，在创建 renderer 时生成 clear/solid/glyph pipeline state；首次 frame 或 surface
generation 变化时 full clear，后续 frame 在 retained color target 上使用
`RenderPlan::damage()` 局部清除并设置 scissor，再按 painter order 重绘完整 retained command
list，最后把完整 target blit 到当前 drawable。`RenderPlan` 仍只携带 owned geometry、
颜色和 page id，GPU texture 只存在 `MetalAtlas`。`MetalSurface::attach_to_view` 只托管外部
AppKit `NSView` 的 backing layer，并用 scoped attachment 恢复原 layer；它不创建窗口。无
Metal device 或有效 drawable 的会话默认跳过硬件测试，需显式运行 ignored test。当前还有一个
probe-only AppKit host ignored test：它在主线程创建临时 `NSWindow`/`NSView`，实测 attachment、resize、
drawable acquisition 和 scoped detach，然后立即销毁 host；该 host 不属于产品 UI，也不改变 backend
不创建窗口的所有权边界。

partial-damage frame 在进入 Objective-C bridge 前由 macOS backend 按 native command bounds
过滤不与 dirty region 相交的命令；full-clear frame 仍发送完整 painter-order list。过滤只影响
backend-owned 临时 ABI 数组，不能修改 shared `RenderPlan` 或 scene，因此不会引入第二套 source
geometry。见 ADR 0073。
