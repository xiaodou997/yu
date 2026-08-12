# ADR 0074：共享 Viewport Frame Handle

## 状态

已接受（Phase 1 诊断）

## 背景

`ViewportFramePublisher` 已把 scene、render plan、Revision 和 serial 绑定成一个 owned
publication，但 publication 仍会把完整 `ViewportRenderFrame` 复制进 publisher cache；macOS
host 接收时又会复制一次。对于大 viewport 或含大量 glyph 的 frame，这会让发布路径产生与
编辑无关的深拷贝，抵消持久化文本和增量布局带来的收益。

## 决策

- `ViewportRenderFrame` 保持不可变、owned 和 Revision-bound；共享边界使用
  `Arc<ViewportRenderFrame>`，不把 `Arc` 暴露给 scene 或 editor canonical state。
- `ViewportFrameCache` 保存一个共享 frame handle，并继续提供 borrowed `get` 查询；需要跨边界
  传递的调用方使用 `current_frame_handle`。
- `ViewportFramePublication` 保存同一个共享 handle，并提供 `frame()` 的 borrowed view 与
  `frame_handle()` 的 owned clone；clone 只增加引用计数，不复制 scene/render plan。
- macOS `MetalViewportHostSession::accept_publication` 直接把 publication handle 放入 host
  cache；Revision、内部 frame Revision 和 serial 的验证顺序不变。
- handle 共享仅限不可变 render preparation；GPU texture、surface、source、layout cache 和
  `EditorDocument` 仍不进入 shared frame。

## 结果

- publisher cache、平台 publication handoff 和 macOS host cache 可以共享同一 frame allocation。
- stale、回退和 surface lifecycle 规则不变；任何 host 都可以在后台持有 publication，而新
  Revision 发布只替换 cache 的一个 `Arc` handle。
- 无窗口测试通过 `Arc::ptr_eq` 锁定 cache/publication/host 的零深拷贝 handoff。
- 未来若需要跨进程或跨 GPU queue 传递，可在不改变 Revision/serial 外层协议的情况下替换为
  更专门的 immutable frame storage。

