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
                                                  yu-font
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
composition overlay 不推进 source Revision，因此不会触发 projection cache 失效。

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

`platform/macos/yu-font-macos` 是 macOS-only 的 CoreText 适配层。`CoreTextFontCatalog::system`
负责读取 CoreText 当前可见的 family 名称，`CoreTextFontResolver::resolve` 负责根据
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
