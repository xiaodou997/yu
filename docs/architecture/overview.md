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
```

后续预计增加：

```text
yu-markdown-edit
yu-font
yu-scene
yu-render
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
`TextSnapshot`、source range、visible/hidden runs 和双向 mapping，不生成第二份可编辑文本。
它通过 `yu-markdown::parse_inline` 获取 parser-owned `InlineDocument` 和 matched
`InlineSpan`，不再在 projection 内维护 delimiter pairing；visible run 同时携带 Plain、Emphasis、
Strong 或 Code style，供后续 layout 使用。当前 span 仍是保守的 Phase 1 语义层，不宣称完整
CommonMark inline AST。

`yu-editor::EditorDocument` 拥有 revision-bound `ProjectionCache`：同一 Revision/range 查询命中
缓存，永久 edit 会映射严格位于 changed range 外的 projection，并保守地使相交或边界 projection
失效。`block_projection(index)` 以当前 `MarkdownDocument` 的 `(range, kind)` 为 key，并在
增量 block sequence 更新后再次验证 entry；普通 block 返回 inline projection，fenced code 返回
独立的 `CodeProjection`，只隐藏 fence 行并把 body 当作字面量 code run，不会把 body 中的
Markdown delimiter 当成 emphasis。
composition overlay 不推进 source Revision，因此不会触发 projection cache 失效。

`yu-layout::LayoutSnapshot` 是 block-local、revision-bound 的纯 Rust 布局契约：它消费
`Projection` 的 visible runs，按 grapheme cluster 生成 `VisualLine`/`VisualCluster`，并提供
`LayoutCaret` 与 `LayoutHit` 的 source/visual 双向查询。当前默认只使用确定性的
`MonospaceMetrics`；真实字体 shaping 通过 `ClusterMetrics` 注入，布局层不依赖窗口或 GPU。
`EditorDocument` 现在拥有独立的 `LayoutCache`：entry 以 block 的 `(range, kind)` 和
`LayoutConfig` 为 key，同一 revision/config 查询命中同一个 cache-owned snapshot。永久 edit
会通过 layout/projection mapping 保留严格位于 changed range 外的布局，并在 block range 或
kind 改变时删除 entry。`LayoutSnapshot::height_index` 暴露 Fenwick prefix index，支持后续
viewport virtualization 的 O(log n) 高度查询与点更新；当前仍未连接窗口、GPU 或真实字体。
