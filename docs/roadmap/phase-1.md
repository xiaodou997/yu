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
- [x] source-backed identity/inline projection 与 hidden delimiter 双向 mapping
- [x] `yu-markdown` lossless inline token CST 被 `yu-projection` 消费
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

## 非目标

- 完整 CommonMark/GFM；
- 产品级窗口、菜单或设置页；
- 自研字体 shaping；
- 三个平台同时达到产品质量；
- 第三方插件 ABI。
