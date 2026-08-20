# ADR 0072：共享 Viewport Frame Publisher

## 状态

已接受（Phase 1 诊断）

## 背景

`ViewportFrameCache` 与 `MetalViewportHostSession` 已经能够拒绝 stale frame，但如果
AppKit host 自己调用 `assemble_viewport_render_frame`、分配 serial，再把 frame 交给
session，平台层仍然会复制一部分 editor/workspace 发布规则。这样容易出现 host-local
serial 与 source Revision 脱节，也让未来的其他平台重复实现同样的组装流程。

## 决策

- `yu-workspace::ViewportFramePublisher` 是不持有平台对象的共享发布器；它只持有最新的
  revision-bound cache、下一个 publication serial 和最近一次 publication。
- `publish` 从 `EditorDocument` 的当前 Revision 组装 scene/render plan，检查二者仍属于
  同一个当前 Revision，然后返回一个 owned `ViewportFramePublication`。
- `ViewportFramePublication` 把 `ViewportRenderFrame`、Revision 和 monotonic serial 作为
  一个不可拆散的交接结果；平台 host 不需要自己推断 frame 是否已经更新。
- macOS `MetalViewportHostSession::accept_publication` 只接受当前 Revision、frame 内部
  Revision 匹配且 serial 严格递增的 publication；验证完成后才写入 host cache。
- host 仍可使用低层 `publish_frame` 进行边界测试或过渡代码，但产品路径必须优先消费
  `ViewportFramePublication`。

## 结果

- workspace 成为 EditorDocument 到平台 host 的唯一 viewport frame 组装边界。
- publication 可以跨线程/平台 API 作为 owned 值传递，而不会携带 `EditorDocument`、source、
  layout cache、AppKit 对象或 GPU handle。
- 重复、乱序和旧 Revision publication 在进入 Metal renderer 前被拒绝；host 的当前 frame、
  serial 和 last submission 在失败时保持不变。
- publication 当前会保留一份 cache 副本和一份 owned handoff；后续如果 profiling 证明复制
  成本过高，可以在不改变 Revision/serial 协议的前提下改为引用计数或不可变 frame handle。

