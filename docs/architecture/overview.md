# Yu 架构总览

## 产品模型

Markdown 源码是唯一持久化真源。投影视图、完整源码视图和局部语法揭示共享同一个
`TextBuffer`，不通过富文本模型往返序列化。

```text
File bytes
    │
    ▼
yu-storage::DocumentSession ──► Text Snapshot ──► Lossless Markdown ──► Projection ──► Layout ──► Scene
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

当前已增加：

```text
yu-storage
```

它只负责 UTF-8 Markdown 文件的 `open`/原子 `save`/clean `reload`、BOM 元数据、磁盘指纹和 Revision-bound
dirty/conflict 状态；它不拥有第二份 source，也不进入窗口/GPU 层。

`platform/macos/yu-storage-macos` 只负责把 FSEvents/DispatchSource vnode 通知转换成共享
`FileWatchDebouncer` 可消费的事件；native watcher 生命周期和 AppKit 对象留在产品壳。

`yu-storage-ffi` 是 macOS 文档壳的窄 C ABI。它把统一的 `DocumentEditorSession` 放在
Rust-owned `YuStorageSession` 中：同一个 handle 同时拥有 `DocumentSession`、唯一
`EditorDocument`、composition 和 close state，并向 Swift 暴露 owned path/source snapshot、
Revision-bound 状态、command/selection/key route、IME composition 和 save/reload/close 结果。
`experiments/macos-document-host` 使用可写的 `DocumentTextView` 作为 AppKit 输入镜像；该镜像
只消费统一 FFI 返回的 owned source、selection、command result 和 composition generation，不拥有
source、history 或 dirty，不能再并列持有 storage/editor 两个 session。

`yu-export` 位于 Markdown parser 与 native clipboard 之间。它接收一个 expected `Revision` 和
source `TextRange`，一次生成 canonical Markdown、纯文本回退和 semantic HTML fragment；HTML
只使用 parser 已识别的 source ranges，不读取 TextKit projection 或 transient composition。FFI
只提供 count/fill 查询，macOS pasteboard 仍由产品壳发布，Rust session 不拥有系统剪贴板。

Accessibility 也只从 canonical source 构建语义快照。`AccessibilitySemanticSnapshot` 为当前
Revision 生成一个 document-root、block 和已识别 inline span 的 source-backed 节点序列；节点只
携带稳定 role、父节点、列表/task flags 以及 source/label UTF-16 ranges。`yu-storage-ffi` 用
count/fill ABI 将 owned 节点交给 macOS host，Swift 将它们映射为实现
`NSAccessibilityElementProtocol` 的 child，文本按节点 Revision 回查，几何由 TextKit 当前布局提供。
`DocumentTextView` 另提供 Heading/Link custom rotor，查询仍只基于当前 child tree；刷新旧树前发布
`uiElementDestroyed`。link destination 和 task action block 也由 Rust parser/command contract 提供；
Swift 只暴露 `accessibilityURL`，以及成功时回到同一 `toggle_task` Transaction 的 checkbox press。
无窗口 self-check 会验证树的父子关系、task value/press、URL、Rotor 目标和编辑后的 stale node；
VoiceOver 实际朗读仍属于人工验收，不等同于自动化通过。

后续预计增加：

```text
yu-markdown-edit
yu-platform
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

native composition bridge 在 canonical Revision 之外维护 transient generation。`yu-editor-ffi`
通过 `YuCompositionProjection` 和 count/fill `yu_composition_session_copy_projection` 返回 Rust
生成的 projected UTF-8、replacement/preedit/visual UTF-16 ranges；所有 mirror 查询同时校验
Revision 与 generation，避免同一 source Revision 下旧的 marked text 结果回写。`YuCompositionCaret`
以 preedit selection 的 active end 作为 visual caret，macOS
`yu_macos_composition_session_block_composition_shaped_caret` 再消费未缓存 transient block
layout 返回 point/line/height。native 层只保存 owned scalar 和版本标识，不复制 Markdown parser
或 composition layout。见 ADR 0077。

`NSTextInputClient` 生命周期在 native 层再区分 canonical replacement range、当前 TextKit
 preedit range 和 marked presentation range。`unmarkText` 只改变 presentation，不会隐式取消
 Rust overlay；后续 commit/cancel 使用保存的 native range 恢复 mirror，并要求最近一次
 projection/caret snapshot 仍属于同一 Revision + generation。见 ADR 0078。

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

Phase 3 的 macOS document host 另通过同一个 `DocumentEditorSession` storage FFI 暴露
`yu_storage_session_projected_source` 与 `yu_storage_session_projection_caret`。前者是
Revision-bound visual UTF-8 count/fill，后者把 source UTF-16 caret 映射为 visual UTF-16 并返回
round-trip source。它们只提供 owned diagnostic snapshots；当前生产 TextKit mirror 仍保留 source
坐标，直到 block projection、composition 和 point hit-testing 契约全部接通。

同一边界现在还暴露 `yu_storage_session_projection_block_count` 与
`yu_storage_session_projected_block`：native host 可按 parser-owned block index 惰性取得 source
UTF-16 range、稳定 block/projection kind、visual 长度和 owned UTF-8。查询始终携带 expected
Revision，越界或过期结果不会写入 metadata；Swift 只校验快照和长度，不重建 block ranges 或
Markdown 语义。该接口是后续 block-local composition、layout 和 visual hit-testing 的输入，
当前仍不替换生产 TextKit source mirror。

composition 现在也通过同一 storage handle 暴露 generation-bound 的
`yu_storage_session_composition_projection`、`yu_storage_session_copy_composition_projection` 和
`yu_storage_session_composition_caret`。Rust 用 `Projection::with_composition` 生成临时
projected UTF-8、preedit/visual selection 和 active marked caret；Swift 必须同时保存 source
Revision 与 composition generation，旧 generation 的 copy/caret 会被拒绝。begin/update/cancel
只改变 transient overlay，canonical source、Markdown CST、history 和 Revision 保持不变；真实
TextKit mirror 仍沿用现有生命周期，visual IME renderer 与最终 Metal 输入接管尚未切换。

在同一 projection bridge 上，`yu_storage_session_projection_selection` 现在把 source UTF-16
selection 映射为 visual UTF-16 range，并返回 source round-trip range；非折叠 selection 的两端
使用 projection 外缘，hidden delimiter 不会出现在 native visual selection。另一个
`yu_storage_session_projection_hit_test` 查询显式接收 metrics layout 配置和 projection-local point，
内部构造 revision-bound `yu-layout::LayoutSnapshot`，返回 snapped source/visual caret、line、
point 和 affinity。Swift 只消费这些 owned scalar；point 仍是 projection-local，不是 screen 坐标，
当前只用于诊断/self-check，生产 TextKit mirror 尚未切换。

TextKit 过渡镜像现在还可以通过 `yu_storage_session_projection_source_caret` 与
`yu_storage_session_projection_source_selection` 将 visual UTF-16 caret/selection 反向映射到
canonical source，并返回 visual round-trip 边界。Swift 的 visual-mirror self-check 只创建临时
`NSTextStorage`/`NSLayoutManager` 验证 projected UTF-8 可被 TextKit 接收，再用同一 Revision 的
Rust reverse mapping 校验 hidden delimiter 和 Unicode；Rust source 发生编辑后旧镜像查询立即被
拒绝。`DocumentTextView` 已增加 visual pointer adapter：生产点击/拖选现在把 document-space
point 交给 `yu_storage_session_macos_projection_hit_test`，由 Rust 使用同一 Revision、CoreText
shaper 和 parser-owned block layout 直接返回 visual/source boundary；临时 TextKit visual mirror
只负责 caret/selection 矩形、输入、IME 和 Accessibility，shaped endpoint 失败时回退 AppKit
source selection。visual drag 另外通过 selection-endpoints ABI 保留 Rust anchor/focus 方向，AppKit
只接收 ordered range；source→visual caret 也通过 Rust projection caret 查询后定位到同一临时布局；
TextKit 仍是输入/IME/Accessibility owner。当前选区背景也由同一 visual range 生成 line-fragment
rectangles，source TextKit 的 selection background 在 adapter 开启时清空，避免 hidden delimiter
被直接高亮。source selection/caret 通知会异步调用同一 Rust Revision 的
`yu_storage_session_macos_shaped_caret_scroll_request`，并只在 AppKit clip boundary 内应用
absolute target。生产 `Up/Down/Shift-Up/Shift-Down` 现在先由 host 发布同一字体/宽度的 CoreText
metrics，再通过 `yu_storage_session_macos_move_vertical` 让 Rust 使用 caller-owned shaper 解析
相邻 visual line；返回的普通 `CommandResult` 继续驱动 source mirror、projected highlight 和
caret reveal。透明 surface 仍不接收输入；当前 block-local shaped visual IME preedit 已进入
同一持久 CoreText/RenderPlan/Metal publication；跨 block preedit 已通过受影响 block span 的
transient layout 发布，完整 visual renderer 迁移仍待后续。`yu_storage_session_macos_projection_hit_test` 会验证已发布的 viewport
max-width/line-height/default-advance；point 与 snapped caret 均为 document-space，Swift 不得
按字体 advance 或 Markdown delimiter 自行猜测 visual boundary。
同一 mirror 在 composition active 时还可以消费 storage FFI 返回的 visual replacement range
和 generation-bound projected preedit，并让 `markedRange`/`attributedSubstring` 读取 visual
坐标；metadata、文本和 callback 必须匹配同一 Revision + generation，过期时清空 visual mirror
并保留 source mirror 回退。该路径仍不替换生产 IME renderer；持久 surface 现在会在 composition
generation 改变时重新提交 transient block glyph。跨 block replacement 的首 block 承载完整 preedit，
后续 block 清除被替换 source，viewport working state 重新测量高度；canonical source、Revision、
LayoutCache 和 history 不变。visual preedit 继续沿用 Revision + composition generation 协议。现在同一 storage handle 还提供
`yu_storage_session_macos_composition_shaped_caret`：它对活动 preedit 构建未缓存的 CoreText
shaped block layout，返回 block-local caret point/line-height，以及 full projected UTF-16 的
selection/replacement range；Swift visual IME self-check 会同时验证 geometry 与 generation，旧
generation 不得发布。该 endpoint 仍负责 IME caret geometry handoff，而同一 composition overlay
已经由 workspace 的 transient block layout 进入 Metal glyph publication；无法建立合法 span 的
情况才安全回退到 native source mirror。
跨 block 点命中使用独立的
`yu_storage_session_macos_composition_projection_hit_test`：它额外校验 composition generation，
对命中的 transient block 使用 CoreText layout，再通过完整 transient projection 返回 source/visual
UTF-16、document-space point、visual selection/replacement 和 block index；旧 canonical
`yu_storage_session_macos_projection_hit_test` ABI 保持为普通 pointer/回退路径。

视觉 scene/glyph/render-plan 的两次 count/fill 调用也携带同一
`composition_generation`。Rust 在非零 fill capacity 前校验上一次 header；如果 marked text 在
两次调用之间更新或取消，即使 canonical Revision 没有变化，也会返回 stale composition 并清空
输出，避免 Swift 把旧容量与新 glyph 数组配对。host/surface snapshot 回传该 generation，产品提交
key 因此同时绑定 Revision、几何、surface generation 和 transient composition identity。

storage FFI 现在还提供 parser-owned block 的 `yu_storage_session_block_layout`，以及 macOS
`yu_storage_session_macos_block_layout`/`yu_storage_session_macos_block_caret`。前者使用显式
metrics 配置构造单 block `LayoutSnapshot`，后者使用 `CoreTextShaper::from_system_ui` 和同一
`yu-layout` line/caret contract；返回值只包含 Revision、source range、block-local visual
length、line count、width/height、CoreText line metrics 和 caret point。Swift 不持有 layout 或
CoreText 对象，过期 block metadata/caret 会被拒绝。随后同一 storage handle 通过
`yu_storage_session_set_viewport_config` 发布 CoreText line metrics，并由
`yu_storage_session_macos_shaped_viewport_blocks` 以 count/fill 返回可见 block range、source
UTF-16 range、document-space block origin/height、measured 和稳定 kind tag；Swift 只消费 owned
scalar，过期 viewport 与容量不足均不能污染 native 数组。
该 storage snapshot 的 header 还携带请求的 `scroll_y`、`viewport_height` 和由 Rust content height
推导的 `max_scroll_y`，明确 document-space block y 到 viewport-local point 的唯一平移入口。
同一 handle 的 `yu_storage_session_macos_shaped_caret_scroll_request` 复用 CoreText shaped
`ViewportLayout` 返回 document-space caret 和绝对 target scroll；Swift 只在 Revision 仍匹配时应用
target，不保存 Rust 高度索引。visual mirror self-check 已验证 document↔viewport round-trip、
caret reveal 和 stale Revision 拒绝，生产 source TextKit mirror 仍不切换该路径。

在此坐标契约之上，`yu_storage_session_macos_visual_scene` 做一次更窄的 retained-scene handoff：
Rust 将同一 shaped viewport 转为 `ViewportBlockGeometry`，交给 `yu-scene::ViewportSceneInput` 和
`SceneBuilder`，为每个可见 block 生成背景与文本 ink bounds 两个最小 rectangle primitive。FFI
只返回 owned Revision、block/source range、kind、矩形和 primitive count；Swift 的
`--visual-scene-self-check` 验证 painter order、同一 block 的来源范围、document-space bounds、
count/fill 容量保护和 stale Revision 丢弃。它是 Track C 的协议探针，不是生产 TextKit/Metal 路径，
也不跨边界暴露 glyph atlas、layout 或 GPU handle。见 ADR 0117。

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

完整可见窗口使用 `SceneBuilder::append_viewport_with_fills` 批量组装：它按
`ViewportSceneInput` 的 block 顺序预检所有 layout、source range、Revision、atlas entry、fill/glyph
geometry 和 primitive budget，然后一次性发布 block fills、glyph primitives 与 damage。每个
fill 先于所属 block 的 glyph，其他 block 没有 fill 时保持原有 glyph-only 结果。任何一个 block
失败都不会留下 viewport 前缀；这使 stale frame 可以整体丢弃并在新的 Revision 重试，而不会让
renderer 接收到部分窗口。

`yu-workspace::assemble_viewport_scene` 是 editor 到 retained scene 的组合边界。它只消费
`EditorDocument::visible_blocks_with_shaper` 返回的当前 Revision metadata，并按同一 block index
取得 shaped `LayoutSnapshot`；随后在 editor-to-scene 边界把 fenced code 的 `BlockKind` 映射为
可选背景颜色，再交给 `ViewportSceneInput` 和 `SceneBuilder::append_viewport_with_fills`。因此
macOS host 不需要复制 Markdown block traversal、HeightIndex 或 layout cache，后续窗口/Metal
层只消费 `ViewportSceneFrame`/`RenderPlan`。

当前 document-host 诊断桥还提供 `yu_storage_session_macos_render_host_frame`。`YuStorageSession`
在 macOS 上懒持有 `CoreTextViewportFrameBuilder` 与 `MetalViewportHostSession`，因此同一个 handle
可以跨首次绘制、重复绘制、scroll、resize 和编辑保留 CPU atlas、RenderPlan fingerprint、publication
serial、Revision 与 surface generation。Swift 只收到 frame/viewport/atlas 统计等 owned scalar，旧
Revision、回退 generation 或失败 publication 不会污染 host state；这个入口目前只由无窗口
`--macos-render-host-self-check` 使用，生产 TextKit source mirror 和窗口 renderer 尚未切换。见
ADR 0120。

在该 persistent host 之上，`yu_storage_session_macos_visual_scene_glyphs` 提供一个更窄的
Revision-bound retained scene handoff。Rust 从同一次 persistent CoreText publication 读取 retained
scene，只导出 glyph primitive 的 owned metadata：atlas page/矩形、origin、bearing、advance、bounds、
颜色以及对应 block 的 source UTF-16 range。count/fill 两阶段 ABI 在容量不足时不会写入部分数组，
并且 header、glyph 数组和 host publication 必须属于同一 Revision/frame/surface generation；Swift
不拥有 atlas 像素、layout、scene 或 GPU handle。该入口目前是 `--visual-scene-glyph-self-check` 的
诊断/opt-in 协议，仍不替换生产 TextKit source mirror，也不提交真实 Metal surface。见 ADR 0121。

在 glyph publication 稳定后，`yu_storage_session_macos_render_host_surface_submit` 增加一个更窄的
真实 surface 验证入口：Swift 在 AppKit main thread 提供临时 `NSView`，Rust 同步创建
或复用同一 view 的 `MetalSurface`、附着 `CAMetalLayer`，复用同一 persistent CoreText publication，按
`Revision gate → atlas sync → render_plan → consumer commit` 提交到真实 drawable。返回值只包含
Revision、surface generation、frame serial、upload/command/damage/atlas 计数和 submitted 标志；
view attachment、Metal device、renderer、atlas、target 和 command queue 都只由 Rust/backend 持有；
显式 detach 时先恢复 view backing layer，再释放这些资源。surface config/scale 变化会推进
generation 并触发下一帧 full clear。该入口由 `--macos-render-host-surface-self-check` 显式触发，不是
可视化演示模式，也不改变生产 TextKit source mirror。见 ADR 0122、0123。

当前 document-host 的产品窗口已经增加一个置于 source TextKit mirror 上方的
`MacosSurfaceHostView` 和 `MacosSurfaceHostCoordinator`。它们只把 AppKit attach/layout/resize、
scroll、编辑 Revision 和 close 生命周期安排成上述 surface submit；同一 geometry/Revision 不会
重复提交，离开 window 前一定 detach。surface frame 只覆盖 `NSScrollView.contentView`，不覆盖
原生 scroller；`hitTest` 返回空，因此输入、IME、选择和 VoiceOver 仍由 TextKit/source mirror
接收。透明 CAMetalLayer 在成功提交后显示 Rust glyph coverage，失败或 detach 时自动隐藏并回退
到 source mirror。空文档使用 `yu_storage_session_macos_font_metrics` 配置 CoreText viewport。
该 adapter 不让 Swift 拥有 Markdown、layout、scene 或 Metal handle。见 ADR 0124、0125。

当前 render host 已进一步固定 document-space viewport 原点：block glyph、damage 和 caret 的
`y` 坐标仍来自同一份文档坐标，而 `RenderPlan::viewport().y()` 取当前 scroll origin，native
Metal bridge 在提交边界统一减去该原点得到 surface-local 坐标。viewport 的 width/height 仍是
可见 surface 尺寸，不会误变成整篇文档高度。这个修复是完整 visual renderer 迁移的前置契约，
并不关闭 TextKit source mirror 的字形回退。见 ADR 0135。

产品窗口现在还有一个独立的 `MacosVisualDecorationView` sibling，位于 Metal surface 之上，
只绘制当前 Revision 的 visual selection/caret rectangles，并对 hit-test 返回空。它使用已经
通过 Revision + composition generation 校验的 Rust/CoreText-shaped geometry FFI；选择矩形和
caret 是 document-space owned scalar，Swift 只应用当前 scroll origin，不复制 source、selection、
IME、HeightIndex 或 Accessibility state。TextKit 在 decoration frame 有效时停止自绘
selection/caret；active composition、frame stale、surface detach 或 native submit 失败时立即清空
overlay 并恢复 TextKit 自绘。TextKit visual mirror 现在只作为上述失败/组合输入 fallback，完整
visual renderer 仍未迁移。见 ADR 0136、0137。

在该 sibling 与 persistent Metal surface 均稳定后，`DocumentTextView` 还使用一个更窄的
source-glyph gate：只有当前 Revision、composition generation、字体/宽度、scroll origin、
viewport/surface 尺寸和 backing scale 全部与最后一次成功 submit 相同，且 decoration frame
有效时，才停止 TextKit source glyph 和 insertion point 的绘制。TextKit 仍保留 string、
selection、NSTextInputClient、IME、复制粘贴和 Accessibility 所有权；编辑、active marked
text、滚动或 resize 期间先继续显示 native source mirror，直到新的 Rust publication 到达
surface。stale、detach 或 submit 失败会立即清除门控并恢复 TextKit 绘制，因此这一步是主视觉
层验证，不是完整 visual renderer 迁移。见 ADR 0139。

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
painter order，solid `FillRect` 不需要 atlas，glyph 的 atlas page 通过尺寸/bytes fingerprint
去重 `AtlasPageUpload`，stale/missing entry 会被拒绝。共享 `yu-render` 当前没有 `wgpu`/Metal
device 或窗口依赖；`RenderUploader` 只定义未来 backend 上传 alpha page 的最小边界。`yu-render`
已用 fake uploader 覆盖 `FontShaper → LayoutSnapshot → Scene → RenderPlan` 端到端 revision、
fill/glyph command order、atlas upload 去重和 command origin；实际 texture 生命周期和 command
encoding 由 macOS backend 承担，不回写 shared plan。

macOS storage FFI 的 `yu_storage_session_macos_visual_render_plan` 是这一链路的诊断 publication：
Rust 使用 CoreText-shaped layout 和 `CoreTextGlyphRasterizer` 生成临时 CPU `GlyphAtlas`，然后复用
`yu_workspace::assemble_viewport_render_frame`，把 Revision-bound fill/glyph command、page metadata
和 damage 的 owned scalars 复制给 Swift。count/fill 在完整 plan 验证后才写数组，容量不足或 stale
Revision 不会发布部分窗口；atlas 像素、CoreText object、layout/cache 和 GPU handle 不跨 FFI。
CoreText numeric face id 由进程内共享 catalog 保持稳定，因此反复建立 shaper 不需要清空 layout cache。
`--visual-render-plan-self-check` 使用 code fixture 同时验证 solid fill、shaped glyph、painter
order、atlas page fingerprints 和 stale Revision；生产 TextKit mirror 和 Metal surface 仍可回退。

`platform/macos/yu-render-macos::CoreTextViewportFrameBuilder` 是诊断 FFI 与 backend 之间的 Rust
准备层：它持有稳定 `CoreTextShaper`、CPU `GlyphAtlas`、`RenderPlanBuilder` 和
`ViewportFramePublisher`，按可见 shaped layout（活动 composition 的受影响 block span 使用未缓存
transient layout）按需 rasterize glyph，并让同一 page fingerprint 跨 Revision/generation 进入 `MetalAtlas`。
`publish_and_submit` 严格复用
`publication → revision gate → atlas sync → render_plan → consumer commit` 顺序；它不持有
`EditorDocument`、surface 或 GPU handle。ignored AppKit probe 现在使用真实 CoreText publication
验证 attachment/resize/drawable/retained target，生产窗口仍未调用该入口。

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

图片资源沿用同一条 source-backed 边界。`yu-projection::ImageSource` 只引用 image source、alt
label、inline destination 和 reference label 的 `TextRange`；Projection 映射 strictly-outside
edit 时同步这些 ranges，图片 URL 不进入 `EditorDocument` 的第二份状态。`yu-assets::ImageCache`
把 destination 作为去重 key，提供可由平台 worker 轮询的 pending 队列、RGBA8 尺寸校验和
Revision-bound `ImagePublication`；decoded bytes 可以跨 Revision 重绑定，旧 Revision 的结果在
cache boundary 被拒绝。macOS storage FFI 的 `YuStorageVisualImage` 只复制同一 Revision 的
UTF-16 ranges、kind 和 resource fingerprint，native 可以用已有 source-range API 取得 destination
后再排入 ImageIO。`yu-render-macos` 现在提供独立的 ImageIO worker，将相对路径解析到文档目录，
只把 owned RGBA8 bytes 带回 Rust；`MetalImageAtlas` 负责按 publication generation 上传
backend-owned `MTLPixelFormatRGBA8Unorm` texture。Scene/RenderPlan 通过 opaque resource
fingerprint 携带 `Image` command；Metal 侧没有 ready texture 时绘制 command 自带的 fallback
rectangle，不阻塞编辑线程。`yu-layout` 现在把每个 image 的 source/alt/visual ranges 投影为
`ImagePlacement`，`yu-workspace` 将其转换为 document-space `ImagePrimitive`，并在 glyph 之后
按 painter order 进入 Scene/RenderPlan；layout hit-test 命中图片时返回完整 source range，FFI
同时以 UTF-16 返回该 range。资源未 ready 时仍只显示 placeholder，实际产品 host 的 ImageIO
worker/`MetalImageAtlas` publication wiring 仍是后续工作。见 ADR 0141、0142、0143。
