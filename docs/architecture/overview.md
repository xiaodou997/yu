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
yu-text ◄── yu-markdown ◄── yu-projection ◄── yu-editor ◄── yu-editor-ffi
   ▲             ▲                                 ▲              ▲
   │             │                                 │              │
yu-core      yu-inspect                      ProjectionCache   macOS/Swift shell
```

后续预计增加：

```text
yu-markdown-edit
yu-font
yu-layout
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

`yu-editor::EditorDocument` 是编辑阶段的状态边界：它同时保存 canonical `TextBuffer`、
revision-bound `EditorSelection` 和 transient composition。平台 view 可以保留 AppKit 的
渲染/输入投影，但永久命令必须回到该边界并通过 Transaction 提交。

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
它通过 `yu-markdown::parse_inline` 获取 parser-owned `InlineDocument`，不再在 projection 内
维护另一套 delimiter scanner；当前 token layer 和 pairing 仍是保守风险验证，正式 inline CST
扩展后继续复用同一个 source/visual mapping 边界。

`yu-editor::EditorDocument` 拥有 revision-bound `ProjectionCache`：同一 Revision/range 查询命中
缓存，永久 edit 会映射严格位于 changed range 外的 projection，并保守地使相交或边界 projection
失效。composition overlay 不推进 source Revision，因此不会触发 projection cache 失效。
