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
Transaction ◄── Command ◄── Source Anchor ◄── Hit Test ◄──────────── GPU
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
yu-text
   ▲
   │
yu-markdown ◄── yu-inspect
```

后续预计增加：

```text
yu-editor
yu-markdown-edit
yu-projection
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

